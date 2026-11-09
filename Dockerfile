# The image holds the server binary and the tools it calls:
# git, an SSH client, the static Docker CLI, and CA certificates.
# Ansible, OpenTofu, and shell jobs run in execution environments.
# The entrypoint in the binary creates the data directory, fixes ownership of
# an existing volume when started as root, joins the socket group, drops to
# uid 10001, and replaces itself with the server.

FROM rust:1.99.1-alpine@sha256:44819b27be4c97b0e4d7203bd2e02c4820f97700211b6a37fb4c246a2de59383 AS build
# hadolint ignore=DL3018 # Alpine package names stay unpinned so security updates still apply.
RUN apk add --no-cache build-base cmake perl pkgconf
SHELL ["/bin/ash", "-o", "pipefail", "-c"]
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
ARG TARGETARCH
RUN case "$TARGETARCH" in \
        amd64) target=x86_64-unknown-linux-musl ;; \
        arm64) target=aarch64-unknown-linux-musl ;; \
        *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && cargo build --release --locked --target "$target" \
    && cp "target/$target/release/koreograf" /koreograf

FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3 AS tools
# Renovate updates DOCKER_VERSION and TINI_VERSION but not the checksums.
# Copy DOCKER_SHA256_* and TINI_SHA256_* from the release download. A stale
# checksum fails the build.
ARG DOCKER_VERSION=29.9.2
ARG DOCKER_SHA256_AMD64=9ac225999fd5e863f7694c728ef23d480616b1e5639328fa309befd4243d69eb
ARG DOCKER_SHA256_ARM64=873b3e0207857c3b2f28392fb922f3ea6efe37620c16d4d4aaf2fef409c90b73
ARG TINI_VERSION=0.19.0
ARG TINI_SHA256_AMD64=c5b0666b4cb676901f90dfcb37106783c5fe2077b04590973b885950611b30ee
ARG TINI_SHA256_ARM64=eae1d3aa50c48fb23b8cbdf4e369d0910dfc538566bfd09df89a774aa84a48b9
ARG TARGETARCH
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
# hadolint ignore=DL3008 # Bookworm package names stay unpinned so security updates still apply.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git openssh-client \
    && rm -rf /var/lib/apt/lists/* \
    && case "$TARGETARCH" in \
        amd64) arch=x86_64; sum=$DOCKER_SHA256_AMD64; tini_arch=amd64; tini_sum=$TINI_SHA256_AMD64 ;; \
        arm64) arch=aarch64; sum=$DOCKER_SHA256_ARM64; tini_arch=arm64; tini_sum=$TINI_SHA256_ARM64 ;; \
        *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && curl -fsSL -o /tmp/docker.tgz \
        "https://download.docker.com/linux/static/stable/$arch/docker-$DOCKER_VERSION.tgz" \
    && printf '%s  %s\n' "$sum" /tmp/docker.tgz | sha256sum -c - \
    && tar -xzf /tmp/docker.tgz -C /tmp \
    && install -D -m 755 /tmp/docker/docker /out/usr/local/bin/docker \
    && curl -fsSL -o /tmp/tini-static \
        "https://github.com/krallin/tini/releases/download/v$TINI_VERSION/tini-static-$tini_arch" \
    && printf '%s  %s\n' "$tini_sum" /tmp/tini-static | sha256sum -c - \
    && install -D -m 755 /tmp/tini-static /out/sbin/tini \
    && rm -rf /tmp/docker /tmp/docker.tgz /tmp/tini-static \
    && mkdir -p /out/usr/bin /out/usr/lib /out/usr/share \
    && cp -a /usr/bin/git /usr/bin/ssh /out/usr/bin/ \
    && cp -a /usr/lib/git-core /out/usr/lib/ \
    && cp -a /usr/share/git-core /out/usr/share/ \
    && { for bin in /usr/bin/git /usr/bin/ssh /usr/lib/git-core/git-remote-http; do \
            ldd "$bin"; \
         done | awk '{ for (i = 1; i <= NF; i++) if ($i ~ /^\//) print $i }' | sort -u \
         | while read -r lib; do \
             mkdir -p "/out$(dirname "$lib")"; \
             cp -L "$lib" "/out$(dirname "$lib")/"; \
         done; } \
    && for dir in /usr/lib/*/pkcs11; do \
        if [ -d "$dir" ]; then cp -a --parents "$dir" /out/; fi; \
    done \
    && for lib in /lib/*/libnss_files.so.2 /lib/*/libnss_dns.so.2; do \
        if [ -f "$lib" ]; then cp -L --parents "$lib" /out/; fi; \
    done \
    && mkdir -p /out/etc/ssh /out/etc/ssl /out/opt/koreograf \
    && { cp -a /etc/ssh/ssh_config /out/etc/ssh/ 2>/dev/null || true; } \
    && cp -a /etc/ssl/certs /out/etc/ssl/ \
    && if [ -f /etc/ssl/openssl.cnf ]; then cp -a /etc/ssl/openssl.cnf /out/etc/ssl/; fi \
    && printf '%s\n' 'passwd: files' 'group: files' 'hosts: files dns' \
        > /out/etc/nsswitch.conf \
    && printf '%s\n' \
        'root:x:0:0:root:/root:/sbin/nologin' \
        'koreograf:x:10001:10001:Koreograf:/var/lib/koreograf:/sbin/nologin' \
        > /out/etc/passwd \
    && printf '%s\n' 'root:x:0:' 'koreograf:x:10001:' > /out/etc/group \
    && mkdir -p /out/var/lib/koreograf /out/tmp /out/etc/koreograf/ca \
    && chown 10001:10001 /out/var/lib/koreograf /out/tmp /out/etc/koreograf/ca \
    && chmod 1777 /out/tmp

FROM scratch
# x-release-please-start-version
ARG VERSION=0.1.0-alpha.9
# x-release-please-end
ARG SOURCE=https://github.com/koreograf/koreograf
ARG GIT_SHA
ARG BUILT_AT
LABEL org.opencontainers.image.version="${VERSION}" \
    org.opencontainers.image.source="${SOURCE}" \
    org.opencontainers.image.revision="${GIT_SHA}" \
    org.opencontainers.image.title="Koreograf" \
    org.opencontainers.image.description="Team-scoped automation with execution environments"
COPY --from=build /koreograf /usr/local/bin/koreograf
COPY --from=tools /out/ /
COPY web /opt/koreograf/web
COPY examples /opt/koreograf/examples
ENV KOREOGRAF_BIND=0.0.0.0:8080 \
    KOREOGRAF_CONTAINER_IMAGE=1 \
    KOREOGRAF_DATA=/var/lib/koreograf \
    KOREOGRAF_WEB=/opt/koreograf/web \
    KOREOGRAF_EXAMPLES=/opt/koreograf/examples \
    KOREOGRAF_GIT_SHA=${GIT_SHA} \
    KOREOGRAF_BUILT_AT=${BUILT_AT}
WORKDIR /var/lib/koreograf
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
    CMD ["/usr/local/bin/koreograf", "healthcheck"]
USER 10001:10001
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/koreograf", "entrypoint"]
CMD ["koreograf"]
