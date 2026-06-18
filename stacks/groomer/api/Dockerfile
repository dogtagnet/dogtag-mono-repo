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
# The groomer backend IS the vet-api binary. FEATURES defaults to `mongo` (persistent prod store);
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
RUN mkdir -p /data && chown dogtag:dogtag /data
USER dogtag
ENV RUST_LOG=info
# The groomer backend runs the vet-api binary; BUSINESS_TYPE=groomer is set via compose env.
ENV BUSINESS_TYPE=groomer
# Internal listen port. Compose runs the groomer with PORT=43618 (binds 0.0.0.0:43618,
# maps 43618 -> 43618); the vet-api default is 41874 when PORT is unset.
EXPOSE 43618
ENTRYPOINT ["/usr/local/bin/vet-api"]
