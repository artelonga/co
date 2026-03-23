# Stage 1: Build
FROM rust:1.85-slim AS builder

WORKDIR /usr/src/co

# Copy workspace manifests
COPY Cargo.toml Cargo.lock ./

# Copy crates
COPY core/ core/
COPY co-cli/ co-cli/
COPY co-web/ co-web/
COPY co/ co/

# Build only co-web in release mode
RUN cargo build --release -p co-web

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy binary (static assets are embedded via rust-embed)
COPY --from=builder /usr/src/co/target/release/co-web /app/co-web

# SQLite database volume
VOLUME /data

ENV CO_WEB_DATA=/data
ENV CO_WEB_PORT=3000

RUN groupadd -r co && useradd -r -g co -d /data co && chown co:co /data
USER co

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/api/health || exit 1

CMD ["/app/co-web"]
