# WSTunnel v10.5.5 uses cfg_select!, which needs Rust 1.100. The agent itself
# retains its lower Cargo MSRV; this is the reproducible bundle-builder version.
FROM rust:1.100-alpine3.21 AS build

ARG TUNNEL_AGENT_VERSION

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

# Building inside the target Alpine platform gives every transport the native
# Keep the current-generation transports together. TrustTunnel has an older,
# compiler-constrained dependency graph and builds in its own Dockerfile.
RUN OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 TUNNEL_AGENT_VERSION="$TUNNEL_AGENT_VERSION" \
      cargo build --locked --release \
    && OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --manifest-path vendor/wstunnel/Cargo.toml \
    && OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --manifest-path vendor/rathole/Cargo.toml \
    && strip --strip-unneeded \
      target/release/tunnel-agent \
      vendor/wstunnel/target/release/wstunnel \
      vendor/rathole/target/release/rathole

FROM scratch AS export
COPY --from=build /src/tunnel-agent/target/release/tunnel-agent /tunnel-agent
COPY --from=build /src/tunnel-agent/vendor/wstunnel/target/release/wstunnel /wstunnel
COPY --from=build /src/tunnel-agent/vendor/rathole/target/release/rathole /rathole
