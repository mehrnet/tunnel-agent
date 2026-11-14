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

WORKDIR /src
# Keep these legacy workspaces outside the tunnel-agent repository root. Cargo
# 1.79 otherwise discovers the agent's Edition 2024 manifest while walking
# parents and fails before it can read either transport manifest.
COPY vendor/rathole /src/rathole
COPY vendor/trusttunnel /src/trusttunnel

RUN OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --manifest-path /src/rathole/Cargo.toml \
    && OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --manifest-path /src/trusttunnel/Cargo.toml --bin trusttunnel_endpoint \
    && strip --strip-unneeded \
      /src/rathole/target/release/rathole \
      /src/trusttunnel/target/release/trusttunnel_endpoint

FROM scratch AS export
COPY --from=build /src/rathole/target/release/rathole /rathole
COPY --from=build /src/trusttunnel/target/release/trusttunnel_endpoint /trusttunnel_endpoint
