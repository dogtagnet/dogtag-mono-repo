# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# admin-api — multi-stage Rust build (impl §7). Build context MUST be the monorepo root (the crate
# path-depends on crates/dogtag-standard-rs + the root Cargo.lock). Build with:
#   docker build -f stacks/admin/api/Dockerfile -t dogtag-admin-api .
# ----------------------------------------------------------------------------------------------

# ---- builder ---------------------------------------------------------------------------------
FROM rust:1.83-bookworm AS builder
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY stacks/vet/api ./stacks/vet/api
COPY stacks/admin/api ./stacks/admin/api

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --bin admin-api && \
    cp /build/target/release/admin-api /usr/local/bin/admin-api

# ---- runtime ---------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --create-home dogtag
COPY --from=builder /usr/local/bin/admin-api /usr/local/bin/admin-api
RUN mkdir -p /data && chown dogtag:dogtag /data
USER dogtag
ENV RUST_LOG=info
# Internal listen port (the binary binds 0.0.0.0:39742; compose maps the host port to this).
EXPOSE 39742
ENTRYPOINT ["/usr/local/bin/admin-api"]
