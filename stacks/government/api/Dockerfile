# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# government-api — multi-stage Rust build. The government credential authority is a NET-NEW,
# separately-deployable role stack (its own crate `government-api`, own docker-compose, own Mongo,
# own on-chain wiring) — unlike groomer, it does NOT reuse the vet-api binary. Build context MUST be
# the monorepo root (the crate path-depends on crates/dogtag-standard-rs + the root Cargo.lock):
#   docker build -f stacks/government/api/Dockerfile -t dogtag-government-api .
# ----------------------------------------------------------------------------------------------

# ---- builder ---------------------------------------------------------------------------------
FROM rust:1.83-bookworm AS builder
WORKDIR /build

# Copy the whole workspace (paths matter: the bin crate path-depends on ../../../crates/*).
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY stacks/vet/api ./stacks/vet/api
COPY stacks/admin/api ./stacks/admin/api
COPY stacks/government/api ./stacks/government/api

# Build only the government-api binary in release. FEATURES defaults to `mongo` (persistent prod
# store); override to empty (`--build-arg FEATURES=`) for an ephemeral MemStore dev image.
ARG FEATURES=mongo
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --features "${FEATURES}" --bin government-api && \
    cp /build/target/release/government-api /usr/local/bin/government-api

# ---- runtime ---------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --create-home dogtag
COPY --from=builder /usr/local/bin/government-api /usr/local/bin/government-api
USER dogtag
ENV RUST_LOG=info
# Internal listen port (the binary binds 0.0.0.0:44832; compose maps the host port to this).
EXPOSE 44832
ENTRYPOINT ["/usr/local/bin/government-api"]
