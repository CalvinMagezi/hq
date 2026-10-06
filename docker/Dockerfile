# syntax=docker/dockerfile:1.7
#
# Official HQ image. Nothing is compiled here: the image is assembled from the
# signed, already verified release artifacts, which the publish workflow (and
# docker/smoke-test.sh users) place in the build context:
#
#   hq-linux-amd64.tar.gz   hq-<version>-linux-x86_64.tar.gz, renamed
#   hq-linux-arm64.tar.gz   hq-<version>-linux-aarch64.tar.gz, renamed
#   hq-web.tar.gz           hq-web-<version>.tar.gz, renamed
#   entrypoint.sh           docker/entrypoint.sh
#
#   docker build -f docker/Dockerfile --build-arg VERSION=0.9.1-main.19 \
#     --build-arg REVISION=<release git sha> --build-arg SOURCE_REVISION=<sha of docker/> <context dir>
#
# Ubuntu 22.04 matches the glibc 2.35 floor the release binaries are built for.
FROM ubuntu:22.04

ARG TARGETARCH
ARG VERSION=unknown
ARG REVISION=unknown
ARG SOURCE_REVISION=unknown
ARG CREATED=unknown

LABEL org.opencontainers.image.title="HQ" \
      org.opencontainers.image.description="Agent-HQ: a local-first AI agent hub with a web UI" \
      org.opencontainers.image.source="https://github.com/CalvinMagezi/hq" \
      org.opencontainers.image.url="https://github.com/CalvinMagezi/hq" \
      org.opencontainers.image.documentation="https://github.com/CalvinMagezi/hq/blob/main/docs/DOCKER.md" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${SOURCE_REVISION}" \
      dev.agent-hq.release-revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.base.name="docker.io/library/ubuntu:22.04"

# git and curl are used by the agent's tools, bubblewrap by the bash sandbox, and tini is PID 1
# so zombies left by the bash tool are reaped without `docker run --init`.
# hadolint ignore=DL3008
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git curl bubblewrap tini \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --user-group --no-create-home --home-dir /data/home --shell /bin/bash hq \
    && install -d -o hq -g hq -m 0750 /data

# The bind mount keeps the tarballs out of the image layers.
RUN --mount=type=bind,target=/ctx \
    set -eux; \
    tar -xzf "/ctx/hq-linux-${TARGETARCH}.tar.gz" -C /usr/local/bin hq; \
    mkdir -p /usr/local/share/hq/web/dist; \
    tar -xzf /ctx/hq-web.tar.gz -C /usr/local/share/hq/web/dist; \
    install -m 0755 /ctx/entrypoint.sh /usr/local/bin/hq-entrypoint; \
    test -f /usr/local/share/hq/web/dist/index.html; \
    hq --version

# All state lives under /data. HQ_WEB_BIND is non-loopback so the published port
# works, which makes HQ require the web token (the entrypoint creates one).
ENV HOME=/data/home \
    HQ_VAULT_PATH=/data/vault \
    HQ_CONFIG_PATH=/data/config.yaml \
    HQ_WEB_BIND=0.0.0.0 \
    HQ_WEB_STATIC_DIR=/usr/local/share/hq/web/dist \
    HQ_GOVERNANCE__BASH__SANDBOX=best_effort

USER 10001:10001
WORKDIR /data
VOLUME ["/data"]
EXPOSE 5678

# HQ_WS_PORT moves the in-container port; EXPOSE stays 5678 as documentation only.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD ["sh", "-c", "curl --fail --silent --output /dev/null \"http://127.0.0.1:${HQ_WS_PORT:-5678}/health\""]

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/hq-entrypoint"]
CMD ["hq", "start", "all"]
