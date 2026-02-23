# 1. Build Stage
FROM rust:latest as builder

WORKDIR /usr/src/app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy the app crate (it's excluded from the workspace, so we build it directly)
COPY app/ .

# Build the API
RUN cargo build --release

# 2. Runtime Stage
FROM debian:bookworm-slim

WORKDIR /app

# Install runtime dependencies (OpenSSL is critical for HTTPs/Postgres)
RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /usr/src/app/target/release/wager-api /usr/local/bin/wager-api

# Set default env vars
ENV PORT=3000
ENV RUST_LOG=info

# Expose port
EXPOSE 3000


# Run the binary
CMD ["wager-api"]
