# Rathole v0.5.0 and TrustTunnel v1.0.41 pin time 0.3.29, which predates the
# type-inference changes in newer Rust. Keep this compatibility toolchain
# explicit and isolated from WSTunnel's current Rust requirement.
FROM rust:1.79-alpine3.20 AS build

RUN apk add --no-cache \
    binutils \
    build-base \
    cmake \
    linux-headers \
    nasm \
    openssl-dev \
    openssl-libs-static \
    perl \
    pkgconf \
    zlib-static

WORKDIR /src/tunnel-agent
COPY . .

RUN OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --manifest-path vendor/rathole/Cargo.toml \
    && OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --manifest-path vendor/trusttunnel/Cargo.toml --bin trusttunnel_endpoint \
    && strip --strip-unneeded \
      vendor/rathole/target/release/rathole \
      vendor/trusttunnel/target/release/trusttunnel_endpoint

FROM scratch AS export
COPY --from=build /src/tunnel-agent/vendor/rathole/target/release/rathole /rathole
COPY --from=build /src/tunnel-agent/vendor/trusttunnel/target/release/trusttunnel_endpoint /trusttunnel_endpoint
