# SSH target for the lab. BASE is a Debian, Ubuntu, Rocky, or Alma image.
ARG BASE=debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3
FROM ${BASE}

COPY install.sh entrypoint.sh /tmp/harness/
RUN chmod 755 /tmp/harness/install.sh /tmp/harness/entrypoint.sh \
    && /tmp/harness/install.sh \
    && mv /tmp/harness/entrypoint.sh /usr/local/bin/harness-sshd \
    && rm -rf /tmp/harness

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/harness-sshd"]
