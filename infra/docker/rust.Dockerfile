# syntax=docker/dockerfile:1.7
#
# All Rust services from one build. The builder compiles the three binaries once; each runtime
# target copies one binary into a distroless, non-root image (uid 65532, no shell).
#
#   docker build -f infra/docker/rust.Dockerfile --target gateway  -t acom/acom-gateway:local  .
#   docker build -f infra/docker/rust.Dockerfile --target commerce -t acom/acom-commerce:local .
#   docker build -f infra/docker/rust.Dockerfile --target agent    -t acom/acom-agent:local    .
#
# Dependencies are compiled in their own layer (cargo-chef): the layer only changes when
# Cargo.toml / Cargo.lock do, so a code change rebuilds only our crates. The layer is plain image
# content (no cache mounts), so the registry and GitHub Actions layer caches keep it too.
#
# Context: the repository root (see .dockerignore).

# The image's toolchain is the build toolchain: rust-toolchain.toml is left out of the context
# copies, because its channel name ("1.99") differs from the image's ("1.99.1") and rustup would
# download a second 650 MB toolchain on every uncached build. Keep the minor version in step.
ARG CHEF_IMAGE=lukemathwalker/cargo-chef:0.1.78-rust-1.99-slim-bookworm

FROM ${CHEF_IMAGE} AS chef
WORKDIR /src
ENV CARGO_TERM_COLOR=never

# The dependency recipe: manifests and lockfile only, so it is stable across code changes.
FROM chef AS planner
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY .cargo .cargo
COPY crates crates
COPY services services
COPY xtask xtask
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json \
      --bin acom-gateway --bin acom-commerce --bin acom-agent
COPY Cargo.toml Cargo.lock clippy.toml ./
COPY .cargo .cargo
COPY crates crates
COPY services services
COPY xtask xtask
COPY migrations migrations
COPY docs/api/generated docs/api/generated
RUN cargo build --release --locked --bin acom-gateway --bin acom-commerce --bin acom-agent \
 && mkdir -p /out \
 && cp target/release/acom-gateway target/release/acom-commerce target/release/acom-agent /out/

FROM gcr.io/distroless/cc-debian12:nonroot AS runtime
LABEL org.opencontainers.image.source="https://github.com/metahackAI/a-commerce" \
      org.opencontainers.image.licenses="Apache-2.0"
ENV ACOM_LOG_FORMAT=json RUST_LOG=info
USER 65532:65532

FROM runtime AS gateway
LABEL org.opencontainers.image.title="acom-gateway"
COPY --from=builder /out/acom-gateway /usr/local/bin/acom-gateway
ENV ACOM_GATEWAY_ADDR=0.0.0.0:7000
EXPOSE 7000
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 CMD ["/usr/local/bin/acom-gateway", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/acom-gateway"]

FROM runtime AS commerce
LABEL org.opencontainers.image.title="acom-commerce"
COPY --from=builder /out/acom-commerce /usr/local/bin/acom-commerce
COPY policies /app/policies
COPY seed /app/seed
ENV ACOM_COMMERCE_ADDR=0.0.0.0:7001 ACOM_WORKER_ADDR=0.0.0.0:7003 ACOM_POLICY_DIR=/app/policies \
    ACOM_SEED_DIR=/app/seed/demo
EXPOSE 7001 7003
ENTRYPOINT ["/usr/local/bin/acom-commerce"]
CMD ["serve"]

FROM runtime AS agent
LABEL org.opencontainers.image.title="acom-agent"
COPY --from=builder /out/acom-agent /usr/local/bin/acom-agent
COPY agents /app/agents
ENV ACOM_AGENT_ADDR=0.0.0.0:7002 ACOM_AGENTS_DIR=/app/agents
EXPOSE 7002
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 CMD ["/usr/local/bin/acom-agent", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/acom-agent"]
