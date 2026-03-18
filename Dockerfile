# --- Runtime Stage only for Kaniko ---
FROM alpine:3.21

# Install runtime dependencies
RUN apk add --no-cache \
    chromium \
    harfbuzz \
    ca-certificates \
    && rm -rf /var/cache/apk/*

# Non-root user
RUN adduser -u 1000 -D -s /bin/sh rebarr

WORKDIR /app

# Copy pre-built binary from build argument
ARG BINARY_PATH
COPY ${BINARY_PATH} ./rebarr

# Copy Rocket config (and other runtime files)
COPY Rocket.toml ./Rocket.toml

# Data dir
RUN mkdir -p /data && chown rebarr:rebarr /data

USER rebarr

# Environment
ENV DATABASE_URL=sqlite:/data/rebarr.db
ENV REBARR_PROVIDERS_DIR=/data/providers
ENV RUST_LOG=info
ENV ROCKET_ADDRESS=0.0.0.0
ENV ROCKET_PORT=8000

VOLUME ["/data"]
EXPOSE 8000

CMD ["./rebarr"]