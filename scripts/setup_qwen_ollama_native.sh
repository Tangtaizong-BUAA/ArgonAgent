#!/usr/bin/env bash
set -euo pipefail

# Link the local Ollama Qwen3.6 nvfp4 model into ResearchCode's native Qwen
# path without weakening the canonical Qwen3.6-27B runtime gate.

SOURCE_MODEL="${QWEN_OLLAMA_SOURCE_MODEL:-qwen3.6:27b-coding-nvfp4}"
CANONICAL_MODEL="${QWEN_OLLAMA_CANONICAL_MODEL:-Qwen/Qwen3.6-27B}"
BASE_URL="${QWEN_BASE_URL:-http://127.0.0.1:11434/v1/chat/completions}"
API_KEY="${QWEN_API_KEY:-local-qwen-ollama}"

if ! command -v ollama >/dev/null 2>&1; then
  echo "ollama command not found" >&2
  exit 2
fi

if ! ollama list | awk 'NR > 1 {print $1}' | grep -Fxq "${SOURCE_MODEL}"; then
  echo "Ollama source model not found: ${SOURCE_MODEL}" >&2
  echo "Available models:" >&2
  ollama list >&2
  exit 2
fi

if ! ollama list | awk 'NR > 1 {print $1}' | grep -Fxq "${CANONICAL_MODEL}:latest"; then
  ollama cp "${SOURCE_MODEL}" "${CANONICAL_MODEL}"
fi

cat <<EOF
export QWEN_BASE_URL="${BASE_URL}"
export QWEN_API_KEY="${API_KEY}"
export RESEARCHCODE_ENABLE_LIVE_PROVIDER="1"
export RESEARCHCODE_ALLOW_NETWORK="1"
export RESEARCHCODE_NATIVE_MODEL="qwen"
EOF
