# Multi-stage image for the Koreograf prototype.
# The process stays root: a mounted docker.sock is root-equivalent on the host,
# and the Debian docker client needs to open that socket.

FROM rust:1.96.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
# Renovate updates TOFU_VERSION from opentofu/opentofu releases.
ARG TOFU_VERSION=1.12.1
# hadolint ignore=DL3008 # Bookworm package names stay unpinned so security updates still apply.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ansible \
        ca-certificates \
        docker.io \
        git \
        openssh-client \
        python3 \
        unzip \
    && rm -rf /var/lib/apt/lists/*
ADD https://github.com/opentofu/opentofu/releases/download/v${TOFU_VERSION}/tofu_${TOFU_VERSION}_linux_amd64.zip /tmp/tofu.zip
RUN unzip -o /tmp/tofu.zip -d /usr/local/bin tofu \
    && chmod 755 /usr/local/bin/tofu \
    && rm /tmp/tofu.zip
COPY --from=build /src/target/release/koreograf /usr/local/bin/koreograf
COPY web /opt/koreograf/web
COPY examples /opt/koreograf/examples
ENV KOREOGRAF_BIND=0.0.0.0:8080 \
    KOREOGRAF_DATA=/var/lib/koreograf \
    KOREOGRAF_WEB=/opt/koreograf/web \
    KOREOGRAF_EXAMPLES=/opt/koreograf/examples
WORKDIR /var/lib/koreograf
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
    CMD ["python3", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/healthz')"]
# hadolint ignore=DL3002 # A mounted docker.sock is root-equivalent, so the process stays root.
USER root
CMD ["koreograf"]
