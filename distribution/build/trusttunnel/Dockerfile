# TrustTunnel v1.0.41 pins time 0.3.29, which predates the type-inference
# changes in newer Rust. Keep its static compiler toolchain explicit and
# isolated from WSTunnel's current Rust requirement.
FROM rust:1.79-alpine3.20 AS build

RUN apk add --no-cache \
    binutils \
    build-base \
    linux-headers \
    openssl-dev \
    openssl-libs-static \
    pkgconf \
    zlib-static

WORKDIR /src/tunnel-agent
COPY . .

RUN OPENSSL_STATIC=1 PKG_CONFIG_ALL_STATIC=1 \
      cargo build --locked --release --manifest-path vendor/trusttunnel/Cargo.toml --bin trusttunnel_endpoint \
    && strip --strip-unneeded vendor/trusttunnel/target/release/trusttunnel_endpoint

FROM scratch AS export
COPY --from=build /src/tunnel-agent/vendor/trusttunnel/target/release/trusttunnel_endpoint /trusttunnel_endpoint
