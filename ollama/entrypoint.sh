#!/bin/sh
set -e

# Start Ollama server in the background
ollama serve &
OLLAMA_PID=$!

# Wait for Ollama to become ready
echo "Waiting for Ollama to start..."
for i in $(seq 1 30); do
    if curl -sf http://localhost:11434/api/tags >/dev/null 2>&1; then
        echo "Ollama is ready."
        break
    fi
    if [ "$i" -eq 30 ]; then
        echo "ERROR: Ollama failed to start within 30 seconds."
        exit 1
    fi
    sleep 1
done

# Warm-up inference request (Decision 11)
echo "Running warm-up inference..."
curl -sf http://localhost:11434/api/generate \
    -d '{"model":"qwen2.5:3b","prompt":"hello","stream":false,"options":{"num_predict":1}}' \
    >/dev/null 2>&1 || echo "WARN: Warm-up request failed (non-fatal)."
echo "Warm-up complete."

# Keep the Ollama server running in the foreground
wait $OLLAMA_PID
