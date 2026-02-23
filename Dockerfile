# 1. Build Stage
FROM rust:1.85-slim-bookworm as builder

WORKDIR /usr/src/kombat-backend

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy the entire workspace
# We copy manifests first to cache dependencies (optimization), but for simplicity in a workspace, copying all is often easier/safer to ensure path dependencies work.
COPY . .

# Build the API specifically
RUN cargo build --release -p wager-api

# 2. Runtime Stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies (OpenSSL is critical for HTTPs/Postgres)
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /usr/src/kombat-backend/target/release/wager-api /usr/local/bin/wager-api

# Set default env vars
ENV PORT=3000
ENV RUST_LOG=info

# Expose port
EXPOSE 3000


# Run the binary
CMD ["wager-api"]
