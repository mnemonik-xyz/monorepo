# syntax=docker/dockerfile:1.7
#
# Production image for `mnemonic-mcp` (HTTP transport, fastembed/local-embed).
#
# Build context is the REPO ROOT (needs core/, mcp/, docs/):
#   docker build -f mcp/Dockerfile -t ghcr.io/mnemonik-xyz/mnemonic-mcp:latest .
#
# Design notes:
#   * Builder installs g++ — fastembed's ONNX Runtime (`ort`) needs libstdc++.
#     The old rust:1-slim image lacked it, which is why prod fell back to a
#     native build. This fixes that so the container is fully self-sufficient.
#   * docs/ is COPY'd into the image so the RAG seed (mcp/src/seed.rs walks
#     docs/**/*.md) works with NO repo checkout on the host.
#   * Runtime is non-root, on debian-slim, with the ONNX shared-lib deps
#     (libstdc++6, libgomp1) and curl for the healthcheck.
#   * Mutable state (SQLite, rag_chunks artifact, fastembed model cache) lives
#     under /data — mount a volume there so it survives container replacement.

# ---- Builder ---------------------------------------------------------------
FROM rust:1-bookworm AS builder

# g++ + libssl-dev + pkg-config: ONNX Runtime (libstdc++) and TLS.
RUN apt-get update && apt-get install -y --no-install-recommends \
        g++ pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy the workspace manifests + members. Cargo.lock pins the build (--locked).
COPY Cargo.toml Cargo.lock* ./
COPY core/ core/
COPY mcp/ mcp/

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked -p mnemonic-mcp --features local-embed \
    && cp /build/target/release/mnemonic-mcp /usr/local/bin/mnemonic-mcp

# ---- Runtime ---------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libstdc++6 libgomp1 curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /app --shell /usr/sbin/nologin mnemonic

WORKDIR /app

COPY --from=builder /usr/local/bin/mnemonic-mcp /usr/local/bin/mnemonic-mcp
# docs/ powers the RAG protocol-knowledge seed; find_docs_root() resolves "docs"
# against the process CWD (/app).
COPY docs/ /app/docs/

# Persisted, mutable state directory (volume mount point). Owned by the runtime
# user so the SQLite DB, rag_chunks artifact, and model cache are writable.
RUN mkdir -p /data/rag_chunks /data/model-cache /keypair \
    && chown -R mnemonic:mnemonic /data /keypair /app

ENV MCP_TRANSPORT=http \
    MCP_HTTP_HOST=0.0.0.0 \
    MCP_HTTP_PORT=3000 \
    STORAGE_MODE=local \
    EMBED_PROVIDER=fastembed \
    DATABASE_PATH=/data/attestations.db \
    RAG_CHUNK_DIR=/data/rag_chunks \
    FASTEMBED_CACHE_DIR=/data/model-cache \
    MNEMONIC_KEYPAIR_PATH=/keypair/id.json \
    RUST_LOG=info

USER mnemonic
EXPOSE 3000

# The MCP /health endpoint returns {"status":"ok",...}. --start-period covers
# first-boot fastembed model download + RAG seed on a fresh volume.
HEALTHCHECK --interval=30s --timeout=5s --start-period=180s --retries=5 \
    CMD curl -fsS http://localhost:3000/health || exit 1

ENTRYPOINT ["mnemonic-mcp"]
CMD ["--transport", "http", "--port", "3000"]
