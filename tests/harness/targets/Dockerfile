# SSH target for the lab. BASE is a Debian, Ubuntu, Rocky, or Alma image.
ARG BASE=debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
FROM ${BASE}

COPY install.sh entrypoint.sh /tmp/harness/
RUN chmod 755 /tmp/harness/install.sh /tmp/harness/entrypoint.sh \
    && /tmp/harness/install.sh \
    && mv /tmp/harness/entrypoint.sh /usr/local/bin/harness-sshd \
    && rm -rf /tmp/harness

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/harness-sshd"]
