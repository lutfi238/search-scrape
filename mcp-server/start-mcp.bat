@echo off
docker run --rm -i ^
  --network search-scrape_default ^
  --env-file "%~dp0mcp.env" ^
  -v "%~dp0model-cache:/home/appuser/.cache/fastembed" ^
  search-scrape-mcp:latest ^
  search-scrape-mcp
