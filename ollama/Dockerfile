FROM ollama/ollama:latest AS model-puller

# Pre-pull the model at build time (Decision 11).
# Start the server in the background, pull the model, then stop.
RUN ollama serve & \
    SERVER_PID=$! && \
    sleep 5 && \
    ollama pull qwen2.5:3b && \
    kill $SERVER_PID && \
    wait $SERVER_PID 2>/dev/null || true

FROM ollama/ollama:latest

# Copy pre-pulled model data from builder stage
COPY --from=model-puller /root/.ollama /root/.ollama

# Install curl for health checks
RUN apt-get update && apt-get install -y --no-install-recommends curl && \
    rm -rf /var/lib/apt/lists/*

EXPOSE 11434

# Entrypoint script: start ollama, issue warm-up inference, then keep running
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
