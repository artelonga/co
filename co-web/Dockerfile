# Stage 1: Build
# Trixie (Debian 13) carries glibc 2.40 — needed because ort-sys (via fastembed
# / CO-164) compiles ONNX parser C++ from source and the cached object files
# can reference __isoc23_strtoull / __isoc23_strtol (glibc 2.38+ symbols).
# Bookworm's glibc 2.36 doesn't define these, breaking the link step. The
# previous CXXFLAGS=-std=c++14 workaround is unreliable because ort-sys's
# build.rs can override CXX flags. Easier to ship on a base image that has
# the symbols natively.
FROM rust:1.94-slim-trixie AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler pkg-config libssl-dev g++ \
    && rm -rf /var/lib/apt/lists/*

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
# CO-84: dev/ crates are workspace members but NOT in default-members. Cargo
# still validates they exist, so include the manifest (and a stub source) in
# the build context. The actual dev binary is never compiled in the prod image.
COPY dev/co-auto/Cargo.toml dev/co-auto/Cargo.toml
COPY dev/co-pwhash/Cargo.toml dev/co-pwhash/Cargo.toml
COPY dev/co-token/Cargo.toml dev/co-token/Cargo.toml
# CO-120: co-agent is a workspace member; include manifest + stub so cargo resolves it.
COPY co-agent/Cargo.toml co-agent/Cargo.toml

# Dummy lib.rs for each crate so cargo can resolve the workspace
RUN mkdir -p core/src co-cli/src co-web/src co/src game-core/src dev/co-auto/src dev/co-pwhash/src dev/co-token/src co-agent/src && \
    echo "pub fn _dummy() {}" > core/src/lib.rs && \
    echo "fn main() {}" > co-cli/src/main.rs && \
    echo "pub fn _dummy() {}" > co-web/src/lib.rs && \
    echo "pub fn _dummy() {}" > co/src/lib.rs && \
    echo "pub fn _dummy() {}" > game-core/src/lib.rs && \
    echo "pub fn _dummy() {}" > dev/co-auto/src/lib.rs && \
    echo "fn main() {}" > dev/co-auto/src/main.rs && \
    echo "fn main() {}" > dev/co-pwhash/src/main.rs && \
    echo "fn main() {}" > dev/co-token/src/main.rs && \
    echo "fn main() {}" > co-agent/src/main.rs

# Build deps only (cached unless Cargo.toml/lock change)
RUN cargo build --release -p co-web 2>/dev/null || true

# --- Source build layer ---
# Copy real source (invalidates only this layer on code changes)
COPY core/ core/
COPY co-cli/ co-cli/
COPY co-web/ co-web/
COPY co/ co/
COPY game-core/ game-core/
COPY dev/ dev/
COPY co-agent/ co-agent/
# CO-260: changelog_routes.rs include_str!("../../CHANGELOG.md") needs the file
# at workspace root inside the build context.
# CO-269: README.md and LICENSE are also needed so reseed_co_root_files can
# seed them into the co universe at runtime (/app/ is the root_dir in Docker).
COPY CHANGELOG.md README.md LICENSE ./
# CO-211: openapi_routes.rs uses include_str!("../../../docs/api/openapi.yaml")
# which resolves relative to the workspace root at compile time.
COPY docs/api/ docs/api/
# CO-372: calendar.rs uses include_str! to embed the sprint index at compile time.
COPY docs/scrum/sprints/ docs/scrum/sprints/

# Remove dummy crate artifacts so cargo rebuilds from real source
RUN rm -rf target/release/.fingerprint/co-* target/release/.fingerprint/co_web-* \
           target/release/.fingerprint/game-core-* target/release/.fingerprint/game_core-* \
           target/release/deps/libco-* target/release/deps/libco_web-* \
           target/release/deps/libgame_core-* target/release/deps/co_web-* \
           target/release/co-web 2>/dev/null; \
    cargo build --release -p co-web

# Stage 2: Runtime
# Trixie matches the builder's glibc 2.40 — bookworm's older glibc 2.36 would
# fail at runtime if any object compiled in the builder references trixie-only
# symbols.
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /usr/src/co/target/release/co-web /app/co-web

# CO-142 Phase E: bundle work/co/ so dev board always reflects repo task state.
# On every boot the server copies /app/seed-co/ → /data/co/ (copy_dir_all).
COPY work/co/ /app/seed-co/

# CO-271: seed well-known root files into the runtime image so
# reseed_co_root_files (called on every boot) can find them at /app/.
# NOTE: these were previously only COPY'd in the builder stage (for
# include_str! compilation), but the runtime stage never received them —
# causing /co/license to return 404 in prod despite CO-269 being deployed.
COPY CHANGELOG.md README.md LICENSE /app/

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
