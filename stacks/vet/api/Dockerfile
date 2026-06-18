# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# vet-api — multi-stage Rust build (impl §7). Build context MUST be the monorepo root (the crate
# depends on the workspace crates/dogtag-standard-rs + crates/dogtag-prover-rs and the root
# Cargo.lock). Build with:  docker build -f stacks/vet/api/Dockerfile -t dogtag-vet-api .
#
# NOTE: the SAME image/binary is reused for the groomer stack — run it with BUSINESS_TYPE=groomer
# and groomer env (there is no separate groomer-api crate; the groomer business backend is vet-api).
# ----------------------------------------------------------------------------------------------

# ---- builder ---------------------------------------------------------------------------------
FROM rust:1.83-bookworm AS builder
WORKDIR /build

# Copy the whole workspace (paths matter: the bin crate path-depends on ../../../crates/*).
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY stacks/vet/api ./stacks/vet/api
COPY stacks/admin/api ./stacks/admin/api

# Build only the vet-api binary in release. FEATURES defaults to `mongo` (persistent prod store);
# override to empty (`--build-arg FEATURES=`) for an ephemeral MemStore dev image.
ARG FEATURES=mongo
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --features "${FEATURES}" --bin vet-api && \
    cp /build/target/release/vet-api /usr/local/bin/vet-api

# ---- runtime ---------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --create-home dogtag
COPY --from=builder /usr/local/bin/vet-api /usr/local/bin/vet-api

# Persisted state (age-encrypted seed) lives under /data (mounted volume).
RUN mkdir -p /data && chown dogtag:dogtag /data
USER dogtag
ENV RUST_LOG=info
# Internal listen port (the binary binds 0.0.0.0:41874; compose maps the host port to this).
EXPOSE 41874
ENTRYPOINT ["/usr/local/bin/vet-api"]
