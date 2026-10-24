FROM rust:1.99.1-bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates
RUN cargo build --release --locked --bin ownserp

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
RUN useradd --system --uid 10001 --create-home ownserp
COPY --from=builder /src/target/release/ownserp /usr/local/bin/ownserp
USER ownserp
ENV OWN_SERPER_BIND=0.0.0.0:8777
EXPOSE 8777
ENTRYPOINT ["ownserp"]
CMD ["serve", "--http"]
