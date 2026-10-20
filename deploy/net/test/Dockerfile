# A systemd machine with what the hub needs, for run.sh. Not an image to deploy.
ARG BASE=debian:bookworm-slim
FROM ${BASE}
ENV container=docker DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq \
 && apt-get install -y -qq systemd systemd-sysv wireguard-tools nftables iptables iproute2 iputils-ping procps python3 >/dev/null \
 && rm -rf /var/lib/apt/lists/* \
 && systemctl mask getty.target console-getty.service systemd-logind.service systemd-timesyncd.service >/dev/null 2>&1; true
STOPSIGNAL SIGRTMIN+3
CMD ["/lib/systemd/systemd"]
