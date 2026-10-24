FROM rust:1.99.1-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS builder
WORKDIR /src
COPY .cargo/config.toml ./.cargo/config.toml
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates
RUN cargo build --profile dist --locked --bin ownserp

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
LABEL org.opencontainers.image.title="OwnSERP" \
      org.opencontainers.image.description="OwnSERP, an MCP server for the Serper.dev search API" \
      org.opencontainers.image.source="https://github.com/devops-infinity/ownserp" \
      org.opencontainers.image.version="1.0.0" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0"
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home ownserp
COPY --from=builder /src/target/dist/ownserp /usr/local/bin/ownserp
USER ownserp
ENV OWN_SERPER_BIND=0.0.0.0:8777
EXPOSE 8777
HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD ["sh", "-c", "curl -fsS \"http://127.0.0.1:${OWN_SERPER_BIND##*:}/healthz\""]
ENTRYPOINT ["ownserp"]
CMD ["serve", "--http"]
