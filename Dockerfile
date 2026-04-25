FROM rust:1-slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock* ./
COPY core/ core/
COPY mcp/ mcp/

RUN cargo build --release -p mnemonic-mcp

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/mnemonic-mcp /usr/local/bin/mnemonic-mcp

ENV MCP_TRANSPORT=http
ENV MCP_HTTP_PORT=3000
ENV STORAGE_MODE=local
ENV RUST_LOG=info

EXPOSE 3000

ENTRYPOINT ["mnemonic-mcp"]
CMD ["--transport", "http", "--port", "3000"]
