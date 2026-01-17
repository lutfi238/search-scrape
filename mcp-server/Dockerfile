FROM rust:latest as builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy manifests
COPY Cargo.toml ./

# Copy source code
COPY src ./src

# Build application
RUN cargo build --release

# Copy source code
COPY src ./src

# Build application
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies including curl for debugging
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create app user with home directory for fastembed model cache
RUN useradd -r -s /bin/false -m -d /home/appuser appuser

# Create cache directory with proper permissions
RUN mkdir -p /home/appuser/.cache && chown -R appuser:appuser /home/appuser

# Copy binary from builder stage
COPY --from=builder /app/target/release/mcp-server /usr/local/bin/mcp-server
COPY --from=builder /app/target/release/search-scrape-mcp /usr/local/bin/search-scrape-mcp

# Change ownership
RUN chown appuser:appuser /usr/local/bin/mcp-server /usr/local/bin/search-scrape-mcp

# Switch to app user
USER appuser

# Expose port
EXPOSE 5000

# Set environment variables
ENV RUST_LOG=info
ENV SEARXNG_URL=http://searxng:8080
# Use home directory for fastembed cache (correct env var name)
ENV FASTEMBED_CACHE_DIR=/home/appuser/.cache/fastembed
ENV HF_HOME=/home/appuser/.cache/huggingface

# Start the application
CMD ["mcp-server"]