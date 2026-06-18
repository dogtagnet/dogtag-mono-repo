# syntax=docker/dockerfile:1
# ----------------------------------------------------------------------------------------------
# groomer-api — there is NO separate groomer-api crate. The groomer business backend IS the
# `vet-api` binary, run with BUSINESS_TYPE=groomer + groomer env/port. This Dockerfile therefore
# builds the SAME vet-api binary (impl §7 / BUILD_PROMPT Phase 8). Build context = monorepo root:
#   docker build -f stacks/groomer/api/Dockerfile -t dogtag-groomer-api .
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
    cargo build --release --bin vet-api && \
    cp /build/target/release/vet-api /usr/local/bin/vet-api

# ---- runtime ---------------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/* && \
    useradd --system --uid 10001 --create-home dogtag
COPY --from=builder /usr/local/bin/vet-api /usr/local/bin/vet-api
RUN mkdir -p /data && chown dogtag:dogtag /data
USER dogtag
ENV RUST_LOG=info
# The groomer backend runs the vet-api binary; BUSINESS_TYPE=groomer is set via compose env.
ENV BUSINESS_TYPE=groomer
# Internal listen port (vet-api binds 0.0.0.0:41874; compose maps host 43618 -> 41874).
EXPOSE 41874
ENTRYPOINT ["/usr/local/bin/vet-api"]
