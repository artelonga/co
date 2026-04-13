# Stage 1: Build
FROM rust:1.88-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/co

# --- Dependency cache layer ---
# Copy only manifests + create dummy sources so cargo resolves & builds deps.
# This layer is cached as long as Cargo.toml/Cargo.lock don't change.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml core/Cargo.toml
COPY core/proto/ core/proto/
COPY core/build.rs core/build.rs
COPY co-cli/Cargo.toml co-cli/Cargo.toml
COPY co-web/Cargo.toml co-web/Cargo.toml
COPY co/Cargo.toml co/Cargo.toml
COPY game-core/Cargo.toml game-core/Cargo.toml

# Dummy lib.rs for each crate so cargo can resolve the workspace
RUN mkdir -p core/src co-cli/src co-web/src co/src game-core/src && \
    echo "pub fn _dummy() {}" > core/src/lib.rs && \
    echo "fn main() {}" > co-cli/src/main.rs && \
    echo "pub fn _dummy() {}" > co-web/src/lib.rs && \
    echo "pub fn _dummy() {}" > co/src/lib.rs && \
    echo "pub fn _dummy() {}" > game-core/src/lib.rs

# Build deps only (cached unless Cargo.toml/lock change)
RUN cargo build --release -p co-web 2>/dev/null || true

# --- Source build layer ---
# Copy real source (invalidates only this layer on code changes)
COPY core/ core/
COPY co-cli/ co-cli/
COPY co-web/ co-web/
COPY co/ co/
COPY game-core/ game-core/

# Remove dummy crate artifacts so cargo rebuilds from real source
RUN rm -rf target/release/.fingerprint/co-* target/release/.fingerprint/co_web-* \
           target/release/.fingerprint/game-core-* target/release/.fingerprint/game_core-* \
           target/release/deps/libco-* target/release/deps/libco_web-* \
           target/release/deps/libgame_core-* target/release/deps/co_web-* \
           target/release/co-web 2>/dev/null; \
    cargo build --release -p co-web

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /usr/src/co/target/release/co-web /app/co-web

# Content repos are cloned at runtime (CO-50), NOT bundled in image.
# The server clones from GitHub on first boot into /data/repos/.

VOLUME /data

ENV CO_WEB_DATA=/data
ENV CO_WEB_PORT=3000

RUN mkdir -p /data && groupadd -r co && useradd -r -g co -d /data co && chown co:co /data
USER co

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3000/api/health || exit 1

CMD ["/app/co-web"]
