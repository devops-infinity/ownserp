# syntax=docker/dockerfile:1

FROM rust:1.99.1-trixie@sha256:931c2c0531a8b143e044f0f3914dfe802f15bb3d1955f0b657e9359327ac2ffc AS builder
WORKDIR /src
COPY .cargo/config.toml ./.cargo/config.toml
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    cargo build --profile dist --locked --bin ownserp \
    && cp target/dist/ownserp /tmp/ownserp

FROM debian:trixie-slim@sha256:62bd423aeee912639ca8ee681c2b762cb3e25137e0a92b20be455898a9f034ad
LABEL org.opencontainers.image.title="OwnSERP" \
      org.opencontainers.image.description="OwnSERP, an MCP server for the Serper.dev search API" \
      org.opencontainers.image.source="https://github.com/devops-infinity/ownserp" \
      org.opencontainers.image.version="2.3.0" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home ownserp
COPY --from=builder /tmp/ownserp /usr/local/bin/ownserp
USER ownserp
ENV OWN_SERPER_BIND=0.0.0.0:8777
EXPOSE 8777
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD host="${OWN_SERPER_BIND%:*}"; case "$host" in 0.0.0.0|::) host=127.0.0.1 ;; esac; curl -fsS "http://$host:${OWN_SERPER_BIND##*:}/healthz"
ENTRYPOINT ["ownserp"]
CMD ["serve", "--http"]
