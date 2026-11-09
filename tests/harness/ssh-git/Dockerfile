# Throwaway SSH and git target for the image smoke.
FROM debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc

# hadolint ignore=DL3008 # Bookworm package names stay unpinned so security updates still apply.
RUN apt-get update \
    && apt-get install -y --no-install-recommends git openssh-server \
    && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/ssh-git-entrypoint
RUN chmod 755 /usr/local/bin/ssh-git-entrypoint

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/ssh-git-entrypoint"]
