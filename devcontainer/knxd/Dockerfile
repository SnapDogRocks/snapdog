FROM alpine:edge@sha256:020dfcbaaf4cc1078bf2d9c7ba31a8466e334061dcd2f248001d68f79e52c000

# Enable testing repository for knxd package
RUN echo "https://dl-cdn.alpinelinux.org/alpine/edge/testing" >> /etc/apk/repositories

# Install knxd and dependencies from edge repositories
RUN apk add --no-cache \
    knxd \
    libusb \
    libev \
    fmt \
    tini \
    net-tools \
    && rm -rf /var/cache/apk/*

# Create knxd user and group
RUN addgroup -g 1000 knxd \
    && adduser -D -u 1000 -G knxd -h /home/knxd -s /bin/sh knxd

# Create necessary directories with proper permissions
RUN mkdir -p \
    /var/run/knxd \
    /var/log/knxd \
    /home/knxd/.config \
    /home/knxd/data \
    && chown -R knxd:knxd \
        /var/run/knxd \
        /var/log/knxd \
        /home/knxd

# Copy entrypoint script
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Set default environment variables
ENV ADDRESS=1.1.128
ENV CLIENT_ADDRESS=1.1.129:8
ENV INTERFACE=dummy
ENV DEBUG_LEVEL=info
ENV FILTERS=single,pace:queue
ENV KNXD_OPTS=""

# Expose KNX ports - both TCP and UDP to work with Testcontainers
EXPOSE 3671/udp 6720/tcp

# Use tini as init system and run as knxd user
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/entrypoint.sh"]

# Health check - check if knxd is listening on port 6720
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD netstat -ln | grep -q ':6720' || exit 1

# Run as non-root user
USER knxd
WORKDIR /home/knxd
