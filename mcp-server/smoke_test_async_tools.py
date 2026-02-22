#!/usr/bin/env python3
import argparse
import json
import sys
import time
import urllib.error
import urllib.request


def post_json(url: str, payload: dict) -> dict:
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=data,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            body = resp.read().decode("utf-8")
            return json.loads(body)
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {e.code}: {body}") from e
    except urllib.error.URLError as e:
        raise RuntimeError(f"Cannot reach MCP server: {e}") from e


def mcp_call(base_url: str, tool_name: str, arguments: dict) -> dict:
    raw = post_json(
        f"{base_url.rstrip('/')}/mcp/call",
        {"name": tool_name, "arguments": arguments},
    )

    if raw.get("is_error"):
        raise RuntimeError(f"Tool {tool_name} returned is_error=true: {json.dumps(raw)}")

    content = raw.get("content") or []
    if not content:
        raise RuntimeError(f"Tool {tool_name} returned empty content: {json.dumps(raw)}")

    text = content[0].get("text")
    if text is None:
        raise RuntimeError(f"Tool {tool_name} missing content[0].text: {json.dumps(raw)}")

    try:
        return json.loads(text)
    except json.JSONDecodeError as e:
        raise RuntimeError(f"Tool {tool_name} returned non-JSON text: {text}") from e


def poll_until_terminal(base_url: str, tool_name: str, job_id: str, poll_interval: float, max_polls: int):
    for i in range(1, max_polls + 1):
        status = mcp_call(base_url, tool_name, {"job_id": job_id})
        state = (status.get("status") or "").lower()
        print(f"  poll {i:02d} -> {state}")

        if state in {"completed", "failed", "expired"}:
            return status

        time.sleep(poll_interval)

    raise RuntimeError(f"Timeout waiting for {tool_name} job {job_id}")


def main():
    parser = argparse.ArgumentParser(description="Smoke test for async MCP tools")
    parser.add_argument("--base-url", default="http://localhost:5000", help="MCP HTTP server base URL")
    parser.add_argument("--poll-interval", type=float, default=2.0, help="Polling interval in seconds")
    parser.add_argument("--max-polls", type=int, default=40, help="Max number of polls per async job")
    args = parser.parse_args()

    base_url = args.base_url.rstrip("/")

    print("[1/4] scrape_batch_async")
    batch_start = mcp_call(
        base_url,
        "scrape_batch_async",
        {
            "urls": ["https://example.com", "https://example.org"],
            "max_concurrent": 2,
            "max_chars": 3000,
        },
    )
    batch_job_id = batch_start.get("job_id")
    if not batch_job_id:
        raise RuntimeError(f"scrape_batch_async did not return job_id: {batch_start}")
    print(f"  job_id: {batch_job_id}")

    print("[2/4] check_batch_status (poll)")
    batch_terminal = poll_until_terminal(
        base_url, "check_batch_status", batch_job_id, args.poll_interval, args.max_polls
    )

    if (batch_terminal.get("status") or "").lower() != "completed":
        raise RuntimeError(f"Batch job not completed successfully: {batch_terminal}")

    batch_final = mcp_call(
        base_url,
        "check_batch_status",
        {"job_id": batch_job_id, "include_results": True},
    )
    results = batch_final.get("results") or []
    print(
        "  completed:"
        f" urls_total={batch_final.get('urls_total')}"
        f", urls_completed={batch_final.get('urls_completed')}"
        f", urls_failed={batch_final.get('urls_failed')}"
        f", results={len(results)}"
    )

    print("[3/4] deep_research_async")
    agent_start = mcp_call(
        base_url,
        "deep_research_async",
        {
            "query": "rust async programming patterns",
            "max_search_results": 3,
            "crawl_depth": 1,
        },
    )
    agent_job_id = agent_start.get("job_id")
    if not agent_job_id:
        raise RuntimeError(f"deep_research_async did not return job_id: {agent_start}")
    print(f"  job_id: {agent_job_id}")

    print("[4/4] check_agent_status (poll)")
    agent_terminal = poll_until_terminal(
        base_url, "check_agent_status", agent_job_id, args.poll_interval, args.max_polls
    )

    if (agent_terminal.get("status") or "").lower() != "completed":
        raise RuntimeError(f"Agent job not completed successfully: {agent_terminal}")

    agent_final = mcp_call(
        base_url,
        "check_agent_status",
        {"job_id": agent_job_id, "include_results": True},
    )

    report = agent_final.get("final_report")
    if not isinstance(report, dict):
        raise RuntimeError(f"check_agent_status missing final_report: {agent_final}")

    stats = report.get("statistics") or {}
    print(
        "  completed:"
        f" sources_processed={agent_final.get('sources_processed')}"
        f", total_sources={agent_final.get('total_sources')}"
        f", pages_scraped={stats.get('pages_scraped')}"
        f", unique_domains={stats.get('unique_domains')}"
    )

    print("\nSmoke test passed for new async tools:")
    print("  - scrape_batch_async")
    print("  - check_batch_status")
    print("  - deep_research_async")
    print("  - check_agent_status")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        print(f"Smoke test failed: {e}")
        sys.exit(1)
