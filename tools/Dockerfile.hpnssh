FROM alpine:3.21 AS build

ARG HPN_COMMIT

RUN apk add --no-cache \
    autoconf \
    automake \
    build-base \
    git \
    libtool \
    linux-headers \
    openssl-dev \
    openssl-libs-static \
    perl \
    zlib-dev \
    zlib-static

WORKDIR /src
RUN git clone https://github.com/rapier1/hpn-ssh.git hpn-ssh \
    && git -C hpn-ssh checkout "$HPN_COMMIT"

WORKDIR /src/hpn-ssh
RUN autoreconf -f -i \
    && CFLAGS="-Os -fno-plt" LDFLAGS="-static" ./configure \
      --prefix=/out/hpnssh \
      --sysconfdir=/out/hpnssh/etc \
      --without-pam \
      --without-kerberos5 \
      --without-libedit \
      --without-security-key-builtin \
    && make -j"$(getconf _NPROCESSORS_ONLN)" \
    && make install-nokeys

FROM scratch AS export
COPY --from=build /out/hpnssh /hpnssh
