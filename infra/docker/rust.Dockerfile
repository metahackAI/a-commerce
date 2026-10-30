# syntax=docker/dockerfile:1.7
#
# All Rust services from one build. The builder compiles the three binaries once (BuildKit cache
# mounts keep crates and target/ between builds); each runtime target copies one binary into a
# distroless, non-root image (uid 65532, no shell).
#
#   docker build -f infra/docker/rust.Dockerfile --target gateway  -t acom/acom-gateway:local  .
#   docker build -f infra/docker/rust.Dockerfile --target commerce -t acom/acom-commerce:local .
#   docker build -f infra/docker/rust.Dockerfile --target agent    -t acom/acom-agent:local    .
#
# Context: the repository root (see .dockerignore).

ARG RUST_VERSION=1.99

FROM rust:${RUST_VERSION}-slim-bookworm AS builder
WORKDIR /src
ENV CARGO_TERM_COLOR=never
COPY Cargo.toml Cargo.lock rust-toolchain.toml clippy.toml ./
COPY .cargo .cargo
COPY crates crates
COPY services services
COPY xtask xtask
COPY migrations migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked --bin acom-gateway --bin acom-commerce --bin acom-agent \
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
ENV ACOM_COMMERCE_ADDR=0.0.0.0:7001 ACOM_WORKER_ADDR=0.0.0.0:7003 ACOM_POLICY_DIR=/app/policies
EXPOSE 7001 7003
ENTRYPOINT ["/usr/local/bin/acom-commerce"]
CMD ["serve"]

FROM runtime AS agent
LABEL org.opencontainers.image.title="acom-agent"
COPY --from=builder /out/acom-agent /usr/local/bin/acom-agent
ENV ACOM_AGENT_ADDR=0.0.0.0:7002
EXPOSE 7002
HEALTHCHECK --interval=10s --timeout=3s --start-period=5s --retries=3 CMD ["/usr/local/bin/acom-agent", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/acom-agent"]
