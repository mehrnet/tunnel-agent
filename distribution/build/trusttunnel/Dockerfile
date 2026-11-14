# Rathole v0.5.0 pins time 0.3.29, which predates the type-inference changes
# in newer Rust. TrustTunnel at the selected revision instead resolves modern
# Edition 2024 dependencies. Build each with its compatible static toolchain.

FROM rust:1.79-alpine3.20 AS rathole

RUN apk add --no-cache \
    binutils \
    build-base \
    linux-headers \
    openssl-dev \
    openssl-libs-static \
    pkgconf \
    zlib-static

WORKDIR /src/rathole
COPY vendor/rathole /src/rathole

RUN OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release \
    && strip --strip-unneeded target/release/rathole

FROM rust:1.100-alpine3.21 AS trusttunnel

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

WORKDIR /src/trusttunnel
COPY vendor/trusttunnel /src/trusttunnel

RUN OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --bin trusttunnel_endpoint \
    && strip --strip-unneeded target/release/trusttunnel_endpoint

FROM scratch AS export
COPY --from=rathole /src/rathole/target/release/rathole /rathole
COPY --from=trusttunnel /src/trusttunnel/target/release/trusttunnel_endpoint /trusttunnel_endpoint
