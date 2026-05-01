# Stage 1: Build
# Note: protobuf-compiler is required because co-agent depends on the `co`
# core library which uses prost-build to generate protobuf types at compile time.
FROM rust:1.88-slim AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/co

# ── Dependency cache layer ────────────────────────────────────────────────────
# Copy only manifests so cargo can resolve and pre-build all dependencies.
# This layer is cached as long as Cargo.toml / Cargo.lock don't change.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml       core/Cargo.toml
COPY core/proto/           core/proto/
COPY core/build.rs         core/build.rs
COPY co-cli/Cargo.toml     co-cli/Cargo.toml
COPY co-web/Cargo.toml     co-web/Cargo.toml
COPY co/Cargo.toml         co/Cargo.toml
COPY game-core/Cargo.toml  game-core/Cargo.toml
COPY co-agent/Cargo.toml   co-agent/Cargo.toml
COPY dev/co-auto/Cargo.toml    dev/co-auto/Cargo.toml
COPY dev/co-pwhash/Cargo.toml  dev/co-pwhash/Cargo.toml
COPY dev/co-token/Cargo.toml   dev/co-token/Cargo.toml

RUN mkdir -p core/src co-cli/src co-web/src co/src game-core/src \
             co-agent/src dev/co-auto/src dev/co-pwhash/src dev/co-token/src && \
    echo "pub fn _dummy() {}" > core/src/lib.rs && \
    echo "fn main() {}"       > co-cli/src/main.rs && \
    echo "pub fn _dummy() {}" > co-web/src/lib.rs && \
    echo "pub fn _dummy() {}" > co/src/lib.rs && \
    echo "pub fn _dummy() {}" > game-core/src/lib.rs && \
    echo "pub fn _dummy() {}" > co-agent/src/lib.rs && \
    echo "fn main() {}"       > co-agent/src/main.rs && \
    echo "pub fn _dummy() {}" > dev/co-auto/src/lib.rs && \
    echo "fn main() {}"       > dev/co-auto/src/main.rs && \
    echo "fn main() {}"       > dev/co-pwhash/src/main.rs && \
    echo "fn main() {}"       > dev/co-token/src/main.rs

RUN cargo build --release -p co-agent 2>/dev/null || true

# ── Source build layer ────────────────────────────────────────────────────────
COPY core/      core/
COPY co-agent/  co-agent/
COPY co/        co/
COPY dev/       dev/

RUN rm -rf \
    target/release/.fingerprint/co-* \
    target/release/.fingerprint/co_agent-* \
    target/release/deps/libco-* \
    target/release/deps/co_agent-* \
    target/release/co-agent 2>/dev/null; \
    cargo build --release -p co-agent

# Stage 2: Runtime (minimal image)
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/src/co/target/release/co-agent /usr/local/bin/co-agent

RUN groupadd -r co && useradd -r -g co co
USER co

LABEL org.opencontainers.image.title="co-agent" \
      org.opencontainers.image.description="CO telemetry sidecar" \
      org.opencontainers.image.version="0.1.0" \
      org.opencontainers.image.source="https://github.com/artelonga/co"

ENTRYPOINT ["/usr/local/bin/co-agent"]
