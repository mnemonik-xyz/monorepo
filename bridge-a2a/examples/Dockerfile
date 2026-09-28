# bridge-a2a Dockerfile
#
# Build: docker build -t bridge-a2a -f bridge-a2a/examples/Dockerfile .
# (run from the workspace root so the full Cargo workspace is in context)

FROM rust:1.80-slim AS builder

WORKDIR /build

# Install SQLite build deps (required for rusqlite bundled feature).
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first for layer caching.
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml core/Cargo.toml
COPY mcp/Cargo.toml mcp/Cargo.toml
COPY mnemonic-a2a/Cargo.toml mnemonic-a2a/Cargo.toml
COPY bridge-a2a/Cargo.toml bridge-a2a/Cargo.toml

# Stub sources so `cargo fetch` succeeds.
RUN mkdir -p core/src mcp/src mnemonic-a2a/src bridge-a2a/src && \
    echo "fn main() {}" > core/src/lib.rs && \
    echo "fn main() {}" > mcp/src/lib.rs && \
    echo "fn main() {}" > mnemonic-a2a/src/lib.rs && \
    echo "fn main() {}" > bridge-a2a/src/lib.rs && \
    echo "fn main() {}" > bridge-a2a/src/main.rs

RUN cargo fetch

# Copy real sources and build the release binary.
COPY . .
RUN cargo build --release -p bridge-a2a

# ── runtime image ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/bridge-a2a /usr/local/bin/bridge-a2a

# Ephemeral keypair (swap with a volume-mounted file for persistence).
ENV BRIDGE_LISTEN=0.0.0.0:8080
ENV BRIDGE_UPSTREAM=http://upstream-a2a:9000
ENV BRIDGE_DB_PATH=/data/bridge-a2a.db
ENV BRIDGE_FAILURE_MODE=attest-best-effort

RUN mkdir -p /data

VOLUME ["/data"]
EXPOSE 8080

ENTRYPOINT ["bridge-a2a"]
