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

FROM rust:1.100-alpine AS trusttunnel

# Build TrustTunnel natively for each release architecture.  Its upstream
# release pipeline uses a real musl GCC toolchain; doing the same avoids the
# duplicate startup objects that result when Rust's musl CRT is linked through
# Zig's driver.  The GitHub matrix runs each image natively, not by emulation.
ARG TARGETARCH
ARG RUST_TARGET

RUN apk add --no-cache \
    bash \
    binutils \
    build-base \
    ca-certificates \
    clang-dev \
    cmake \
    git \
    linux-headers \
    nasm \
    perl \
    pkgconf

RUN case "$TARGETARCH:$RUST_TARGET" in \
        amd64:x86_64-unknown-linux-musl|arm64:aarch64-unknown-linux-musl) ;; \
        *) echo "native image and Rust target disagree: $TARGETARCH / $RUST_TARGET" >&2; exit 1 ;; \
    esac \
    && rustup toolchain install 1.95 --profile minimal \
    && rustup target add --toolchain 1.95 "$RUST_TARGET"

ENV CC=gcc
ENV CXX=g++
ENV CMAKE_C_COMPILER=gcc
ENV CMAKE_CXX_COMPILER=g++
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=gcc
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=gcc
ENV LIBCLANG_PATH=/usr/lib
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
