# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# indexer-api — multi-stage Rust build. The oversight indexer is a NET-NEW, separately-deployable
# service (its own crate `indexer-api`, own docker-compose, own Mongo). It scans the ROAX contract
# event logs into a non-PII index and serves the scope-enforced query API (unscoped government
# oversight + scoped vet/groomer views). Build context MUST be the monorepo root (the crate
# path-depends on crates/dogtag-standard-rs + the root Cargo.lock):
#   docker build -f stacks/indexer/api/Dockerfile -t dogtag-indexer-api .
# ----------------------------------------------------------------------------------------------

# ---- builder ---------------------------------------------------------------------------------
FROM rust:1.83-bookworm AS builder
WORKDIR /build

# Copy the whole workspace (all members must be present to load the virtual workspace).
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY stacks/vet/api ./stacks/vet/api
COPY stacks/admin/api ./stacks/admin/api
COPY stacks/government/api ./stacks/government/api
COPY stacks/indexer/api ./stacks/indexer/api

# Build only the indexer-api binary in release. FEATURES defaults to `mongo` (persistent prod
# store); override to empty (`--build-arg FEATURES=`) for an ephemeral MemStore dev image.
ARG FEATURES=mongo
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --features "${FEATURES}" --bin indexer-api && \
    cp /build/target/release/indexer-api /usr/local/bin/indexer-api

# ---- runtime ---------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --create-home dogtag
COPY --from=builder /usr/local/bin/indexer-api /usr/local/bin/indexer-api
USER dogtag
ENV RUST_LOG=info
# Internal listen port (the binary binds 0.0.0.0:46001; compose maps the host port to this).
EXPOSE 46001
ENTRYPOINT ["/usr/local/bin/indexer-api"]
