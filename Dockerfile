# The whole harness: both binaries, a base image, and whatever you mount.
#
# Build:  docker build -t clank .
# Use:    git log -1 | docker run --rm -i --network=host \
#           -e CLANK_BASE_URL=... -e CLANK_MODEL=... clank -m "what changed?"
#
# `-i` is what makes the pipe reach it. `--network=host` is what makes a model on
# localhost reachable — and it is also the limit of the sandbox: the container
# bounds what the model can *write*, not what it can *reach* (PROTOCOL.md).
#
# Not alpine. clank is glibc-dynamic; musl has no loader for it.

FROM rust:1-slim AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM ubuntu:24.04
COPY --from=build /src/target/release/clank /usr/local/bin/clank
COPY --from=build /src/target/release/clank-jev /usr/local/bin/clank-jev
ENTRYPOINT ["/usr/local/bin/clank"]
# `clank-jev` is in the image too: docker run --rm -i clank-jev --provider kev …
