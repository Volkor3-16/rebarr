# --- Builder Stage ---
FROM rust:1-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY providers ./providers
COPY web ./web
COPY Rocket.toml ./Rocket.toml

RUN cargo build --release --bin rebarr

# --- Runtime Stage ---
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      chromium \
      xvfb \
      x11vnc \
      novnc \
      websockify \
      fluxbox \
      nginx \
      supervisor \
      wget \
      ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd -u 1000 -m -s /bin/sh rebarr

WORKDIR /app

# Copy binary built in the Debian builder image (glibc-compatible at runtime)
COPY --from=builder /build/target/release/rebarr ./rebarr

# Copy web assets
COPY web/ ./web/

# Copy default providers (seeded to /data on first run by entrypoint)
COPY providers/ ./providers/

# Copy Rocket config (and other runtime files)
COPY Rocket.toml ./Rocket.toml

# Entrypoint script
COPY docker-entrypoint.sh ./docker-entrypoint.sh
RUN chmod +x ./docker-entrypoint.sh

# Desktop/runtime config
COPY docker/supervisord.conf /etc/supervisord.conf
COPY docker/nginx.conf /etc/nginx/nginx.conf
COPY docker/healthcheck.sh /usr/local/bin/healthcheck.sh
RUN chmod +x /usr/local/bin/healthcheck.sh

# Data dir
RUN mkdir -p /data && chown -R rebarr:rebarr /data

USER rebarr

# Environment
ENV DATABASE_URL=sqlite:/data/rebarr.db
ENV REBARR_PROVIDERS_DIR=/data/providers
ENV RUST_LOG=info
ENV ROCKET_ADDRESS=127.0.0.1
ENV ROCKET_PORT=18000
ENV DISPLAY=:99
ENV CHROME_HEADLESS=false

VOLUME ["/data"]
EXPOSE 8000
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 CMD /usr/local/bin/healthcheck.sh

ENTRYPOINT ["./docker-entrypoint.sh"]
