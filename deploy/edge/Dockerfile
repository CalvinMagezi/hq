# The public forwarder (docs/EDGE.md). Build from the repository root:
#   docker build -f deploy/edge/Dockerfile -t hq-edge .
# Run it with host networking so port 443 and the iroh UDP port are the machine's own. Drop every
# capability but the one that binds 443 (do not add --security-opt no-new-privileges: it disables
# the file capability the image relies on to bind 443 as a non-root user):
#   docker run -d --name hq-edge --network host --restart unless-stopped \
#     --cap-drop ALL --cap-add NET_BIND_SERVICE \
#     -v hq-edge-data:/data -v "$PWD/config.yaml:/etc/hq-edge/config.yaml:ro" hq-edge
# Mount the config outside /data: a file mounted inside the volume makes Docker create its parent
# directory as root, and the edge (a non-root user) could then not create its own directory.
# Manage devices from the host with: docker exec hq-edge hq edge allow <device id>
FROM rust:1.98.1-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release -p hq-cli --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libcap2-bin libsqlite3-0 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /data --shell /usr/sbin/nologin hq-edge

COPY --from=builder /app/target/release/hq /usr/local/bin/hq
# A non-root user cannot bind a port under 1024 under host networking without this.
RUN setcap cap_net_bind_service=+ep /usr/local/bin/hq

# The edge key, the device list and the relay's certificate cache are kept under /data/.hq/edge.
ENV HOME=/data
ENV HQ_CONFIG_PATH=/etc/hq-edge/config.yaml
USER hq-edge
WORKDIR /data
VOLUME /data

EXPOSE 443/tcp
CMD ["hq", "edge", "run"]
