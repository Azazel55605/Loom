# syntax=docker/dockerfile:1

# Build context is the REPO ROOT (see docker-compose.local.yml): the crate is
# part of a Cargo workspace and cannot be built without the workspace manifest
# and its sibling crates.
#
# Dependency compilation is cached with cargo-chef. Cargo has no equivalent of
# `npm install` that stops after fetching dependencies, so the usual Docker
# trick of "copy manifests, install, then copy source" does not work: `cargo
# build` needs the source tree present, which would put the sources in the same
# layer as the dependency build and invalidate it on every edit. cargo-chef
# works around this by distilling the workspace manifests into a recipe.json,
# then compiling only the dependencies from that recipe — so the expensive layer
# is keyed on dependency changes, not source changes.

FROM rust:1-slim AS chef

# --locked: build cargo-chef against its own committed lockfile, so this layer
# is reproducible rather than drifting with its transitive dependencies.
RUN cargo install cargo-chef --locked
WORKDIR /build

# Planner stage: reads the workspace and emits a dependency recipe. This runs on
# every build, but it is fast and its output changes only when dependencies do.
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

# Only recipe.json is copied here, NOT the source. This layer therefore survives
# any source edit and is rebuilt only when the recipe changes.
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Now the source, and a build that reuses the compiled dependencies above.
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
