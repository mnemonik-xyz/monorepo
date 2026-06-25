#!/usr/bin/env bash
# Pre-populate the fastembed (all-MiniLM-L6-v2) model into an hf-hub cache so the
# server can start offline. Needed where huggingface.co LFS (cdn-lfs) is blocked;
# the hf-mirror.com endpoint serves the weights.
#
# Usage: ./fetch-fastembed-model.sh [CACHE_DIR]   (default: $HOME/.fe)
set -euo pipefail
CACHE="${1:-$HOME/.fe}"
ENDPOINT="${HF_ENDPOINT:-https://hf-mirror.com}"
REPO="Qdrant/all-MiniLM-L6-v2-onnx"
COMMIT="0000000000000000000000000000000000000000" # placeholder; fine for HF_HUB_OFFLINE
SNAP="$CACHE/models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/$COMMIT"
REFS="$CACHE/models--Qdrant--all-MiniLM-L6-v2-onnx/refs"
mkdir -p "$SNAP" "$REFS"
printf '%s' "$COMMIT" > "$REFS/main"
for f in tokenizer.json config.json special_tokens_map.json tokenizer_config.json model.onnx; do
  echo "fetching $f ..."
  curl -fsSL -o "$SNAP/$f" "$ENDPOINT/$REPO/resolve/main/$f"
done
echo "model cached at: $CACHE"
echo "run server with: FASTEMBED_CACHE_DIR=$CACHE HF_HUB_OFFLINE=1 ..."
