FROM alpine:3.21@sha256:bc21487101ae47172509130ee1092cc273a831968d64b617776d1f69c81e42ab AS builder

ARG SNAPCAST_VERSION=0.32.3

RUN apk add --no-cache build-base cmake git boost-dev alsa-lib-dev flac-dev openssl-dev

WORKDIR /build
RUN git clone --depth 1 --branch v${SNAPCAST_VERSION} https://github.com/badaix/snapcast.git && \
    cmake -B build -S snapcast \
        -DCMAKE_BUILD_TYPE=Release \
        -DBUILD_WITH_FLAC=ON \
        -DBUILD_WITH_VORBIS=OFF \
        -DBUILD_WITH_OPUS=OFF \
        -DBUILD_WITH_AVAHI=OFF \
        -DBUILD_WITH_EXPAT=OFF \
        -DBUILD_TESTS=OFF \
        -DBUILD_CLIENT=OFF && \
    cmake --build build -j$(nproc) && \
    cmake --install build

FROM alpine:3.21@sha256:bc21487101ae47172509130ee1092cc273a831968d64b617776d1f69c81e42ab
RUN apk add --no-cache dumb-init alsa-lib flac openssl libstdc++
COPY --from=builder /usr/local/bin/snapserver /usr/local/bin/

RUN adduser -D snapcast && mkdir -p /snapsinks && chown snapcast:snapcast /snapsinks

# Default config for standalone dev usage (snapdog generates its own when managed)
RUN printf '[http]\nenabled = true\nport = 1780\n\n[server]\nport = 1704\n\n[stream]\nsource = tcp://0.0.0.0:4953?name=Zone1&sampleformat=48000:16:2&mode=server\nsource = tcp://0.0.0.0:4954?name=Zone2&sampleformat=48000:16:2&mode=server\n' > /etc/snapserver.conf

USER snapcast
EXPOSE 1704 1705 1780
ENTRYPOINT ["dumb-init", "snapserver"]
