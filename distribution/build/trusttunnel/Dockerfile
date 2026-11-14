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

ARG TARGETARCH
FROM ghcr.io/rust-cross/rust-musl-cross:${TARGETARCH}-musl AS trusttunnel

# Build TrustTunnel with a real musl-cross-make GCC toolchain.  Unlike Zig,
# this toolchain uses the same startup objects as Rust.  Unlike Alpine, the
# glibc base can dlopen libclang while TrustTunnel's bindgen step runs.
ARG TARGETARCH
ARG RUST_TARGET

RUN apt-get update \
    && apt-get install --yes --no-install-recommends \
    binutils \
    ca-certificates \
    nasm \
    perl \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

RUN case "$TARGETARCH:$RUST_TARGET" in \
        amd64:x86_64-unknown-linux-musl|arm64:aarch64-unknown-linux-musl) ;; \
        *) echo "native image and Rust target disagree: $TARGETARCH / $RUST_TARGET" >&2; exit 1 ;; \
    esac \
    && rustup toolchain install 1.95 --profile minimal \
    && rustup target add --toolchain 1.95 "$RUST_TARGET"

ENV CMAKE_C_COMPILER=${TARGET_CC}
ENV CMAKE_CXX_COMPILER=${TARGET_CXX}
ENV CC=${TARGET_CC}
ENV CXX=${TARGET_CXX}
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=${TARGET_CC}
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=${TARGET_CC}
# TrustTunnel pins 1.95 in rust-toolchain.toml. Set it explicitly so the
# installed musl target is used instead of the base image's toolchain.
ENV RUSTUP_TOOLCHAIN=1.95

WORKDIR /src/trusttunnel
COPY vendor/trusttunnel /src/trusttunnel

RUN cargo build --locked --release --target "$RUST_TARGET" --bin trusttunnel_endpoint \
    && strip --strip-unneeded "target/${RUST_TARGET}/release/trusttunnel_endpoint"

FROM scratch AS export
ARG RUST_TARGET
COPY --from=rathole /src/rathole/target/release/rathole /rathole
COPY --from=trusttunnel /src/trusttunnel/target/${RUST_TARGET}/release/trusttunnel_endpoint /trusttunnel_endpoint
