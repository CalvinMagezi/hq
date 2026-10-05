# Stage 1: Build
FROM rust:1.98.1-bookworm AS builder

WORKDIR /app
COPY . .

# Build the CLI binary
RUN cargo build --release -p hq-cli

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/hq /usr/local/bin/hq

# Vault lives on a mounted volume at /data/.vault
ENV HQ_VAULT_PATH=/data/.vault

EXPOSE 5678

CMD ["hq", "start", "all"]
