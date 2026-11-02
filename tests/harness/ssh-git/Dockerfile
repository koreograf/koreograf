# Throwaway SSH and git target for the image smoke.
FROM debian:bookworm-slim@sha256:99c0b25c46d0214d5a2091092e7908907dcaecd6a962cde0ac700acee222b6f3

# hadolint ignore=DL3008 # Bookworm package names stay unpinned so security updates still apply.
RUN apt-get update \
    && apt-get install -y --no-install-recommends git openssh-server \
    && rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /usr/local/bin/ssh-git-entrypoint
RUN chmod 755 /usr/local/bin/ssh-git-entrypoint

EXPOSE 22
ENTRYPOINT ["/usr/local/bin/ssh-git-entrypoint"]
