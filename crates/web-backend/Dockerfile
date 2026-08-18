# syntax=docker/dockerfile:1

# Build context is the REPO ROOT (see docker-compose.local.yml): the crate is
# part of a Cargo workspace and cannot be built without the workspace manifest
# and its sibling crates.
#
# NOTE: this copies the full repo into the build context in one layer and is not
# layer-cache-optimized — any source change re-downloads and recompiles all
# dependencies. Acceptable at this stage; worth revisiting for build speed
# (cargo-chef, or a separate manifest-only dependency layer) once the workspace
# grows.

FROM rust:1-slim AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# The package and binary are named `loom-web-backend`, not `web-backend`.
RUN cargo build --release --package loom-web-backend

FROM debian:bookworm-slim AS runtime

# The binary makes outbound HTTPS calls to connectors; without CA certificates
# every TLS handshake in the runtime image fails.
RUN apt-get update \
    && apt-get install --no-install-recommends -y ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Don't run the server as root.
RUN useradd --system --create-home --uid 10001 loom
USER loom

COPY --from=builder /build/target/release/loom-web-backend /usr/local/bin/loom-web-backend

ENV LOOM_BIND_ADDR=0.0.0.0:8080 \
    LOOM_LOG_LEVEL=info

EXPOSE 8080

ENTRYPOINT ["loom-web-backend"]
