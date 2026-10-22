# The public forwarder (docs/EDGE.md). Build from the repository root:
#   docker build -f deploy/edge/Dockerfile -t hq-edge .
# Run it with host networking so port 443 and the iroh UDP port are the machine's own:
#   docker run -d --name hq-edge --network host --restart unless-stopped \
#     -v hq-edge-data:/data -v "$PWD/config.yaml:/data/.hq/config.yaml:ro" hq-edge
FROM rust:1.98.1-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release -p hq-cli --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /data --shell /usr/sbin/nologin hq-edge

COPY --from=builder /app/target/release/hq /usr/local/bin/hq

# The edge key, the device list and the relay's certificate cache are kept under /data/.hq/edge.
ENV HOME=/data
ENV HQ_CONFIG_PATH=/data/.hq/config.yaml
USER hq-edge
WORKDIR /data
VOLUME /data

EXPOSE 443/tcp
CMD ["hq", "edge", "run"]
