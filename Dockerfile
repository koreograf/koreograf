# Multi-stage image for the Koreograf prototype.
# tini is PID 1. It reaps orphaned job processes and forwards stop signals.
# The entrypoint starts as root so it can hand the data directory to uid 10001,
# then drops privileges. A mounted docker.sock stays root: that socket is
# root-equivalent on the host, and the Debian docker client needs to open it.

FROM rust:1.96.0-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
# Renovate updates TOFU_VERSION only. Copy TOFU_SHA256_AMD64 and
# TOFU_SHA256_ARM64 from that release's SHA256SUMS. A stale checksum fails the build.
ARG TOFU_VERSION=1.13.2
ARG TOFU_SHA256_AMD64=723bebb0df8411daff1e5fc3862e28ff5581fe5634087f4c29a9aa41d78208b5
ARG TOFU_SHA256_ARM64=6e815c58a09ee957de606ff48cefad082942e70afc43b737ff975654c54ec453
ARG TARGETARCH
# hadolint ignore=DL3008 # Bookworm package names stay unpinned so security updates still apply.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ansible \
        ca-certificates \
        docker.io \
        git \
        openssh-client \
        python3 \
        tini \
        unzip \
    && python3 -c 'import hashlib, os, urllib.request; version=os.environ["TOFU_VERSION"]; arch=os.environ["TARGETARCH"]; sums={"amd64": os.environ["TOFU_SHA256_AMD64"], "arm64": os.environ["TOFU_SHA256_ARM64"]}; expected=sums.get(arch); \
url=f"https://github.com/opentofu/opentofu/releases/download/v{version}/tofu_{version}_linux_{arch}.zip"; \
assert expected, f"unsupported architecture: {arch}"; \
urllib.request.urlretrieve(url, "/tmp/tofu.zip"); \
digest=hashlib.sha256(open("/tmp/tofu.zip","rb").read()).hexdigest(); \
assert digest==expected, f"checksum mismatch for {arch}: {digest}"' \
    && unzip -o /tmp/tofu.zip -d /usr/local/bin tofu \
    && chmod 755 /usr/local/bin/tofu \
    && rm /tmp/tofu.zip \
    && printf 'koreograf:x:10001:\n' >> /etc/group \
    && printf 'koreograf:x:10001:10001::/var/lib/koreograf:/bin/false\n' >> /etc/passwd \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/koreograf /usr/local/bin/koreograf
COPY web /opt/koreograf/web
COPY examples /opt/koreograf/examples
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
# x-release-please-start-version
ARG VERSION=0.1.0-alpha.6
# x-release-please-end
ARG SOURCE=https://github.com/koreograf/koreograf
LABEL org.opencontainers.image.version="${VERSION}" \
    org.opencontainers.image.source="${SOURCE}"
RUN chmod 755 /usr/local/bin/entrypoint.sh \
    && mkdir -p /var/lib/koreograf \
    && chown 10001:10001 /var/lib/koreograf
ENV KOREOGRAF_BIND=0.0.0.0:8080 \
    KOREOGRAF_DATA=/var/lib/koreograf \
    KOREOGRAF_WEB=/opt/koreograf/web \
    KOREOGRAF_EXAMPLES=/opt/koreograf/examples
WORKDIR /var/lib/koreograf
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
    CMD ["python3", "-c", "import urllib.request; urllib.request.urlopen('http://127.0.0.1:8080/healthz')"]
# hadolint ignore=DL3002 # The entrypoint drops to uid 10001 unless docker.sock is mounted.
USER 0
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
CMD ["koreograf"]
