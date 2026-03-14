# --- Build Stage ---
FROM rust:1.88.0-alpine AS builder

# Install dependencies
RUN apk add --no-cache musl-dev pkgconfig openssl-dev curl

# Set Rust OpenSSL to use vendored static libs
ENV OPENSSL_STATIC=1
ENV OPENSSL_DIR=/usr/local/ssl

WORKDIR /app

# This makes docker cache dependencies.... somehow
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main(){}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy source
COPY src ./src
COPY migrations ./migrations

# Compile rebarr
RUN cargo build --release --bin rebarr

# --- Runtime Stage ---
FROM alpine:3.21

RUN apk add --no-cache \
    chromium \
    harfbuzz \
    ca-certificates \
    && rm -rf /var/cache/apk/*

# Non-root user
RUN adduser -u 1000 -D -s /bin/sh rebarr

WORKDIR /app

COPY --from=builder /app/target/release/rebarr ./rebarr
COPY Rocket.toml ./Rocket.toml

RUN mkdir -p /data && chown rebarr:rebarr /data

USER rebarr

ENV DATABASE_URL=sqlite:/data/rebarr.db
ENV REBARR_PROVIDERS_DIR=/data/providers
ENV RUST_LOG=info
ENV ROCKET_ADDRESS=0.0.0.0
ENV ROCKET_PORT=8000

VOLUME ["/data"]
EXPOSE 8000

CMD ["./rebarr"]
