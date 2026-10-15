# Image for the issue #532 "100K live sessions" compose stack.
#
# It deliberately contains **no source and no binaries**: the repository is
# mounted at /src (read-only) by docker-compose.yml and the build writes to a
# named volume (CARGO_TARGET_DIR=/build). That way
#   * the build context stays tiny (no `target/`, no `.git/` — a repo-root
#     context would ship ~100 GB to the daemon),
#   * there is no glibc/musl assumption about the host (the binary is built
#     against this image's libc and runs in this image),
#   * and rebuilding after a code change needs no image rebuild at all.
#
# `rust:bookworm` tracks current stable, which is well above the workspace-wide
# MSRV (1.95.0 in Cargo.toml).
FROM rust:bookworm

# Where the build output goes (a named volume in compose, so repeated runs reuse
# it) and where the crate registry cache lives.
ENV CARGO_TARGET_DIR=/build \
    CARGO_HOME=/cargo \
    CARGO_TERM_COLOR=never

WORKDIR /src

# `--locked` is the build command's business (see docker-compose.yml); nothing
# else is needed here: rmqttd and the harness have no runtime dependencies
# beyond libc.
CMD ["cargo", "--version"]
