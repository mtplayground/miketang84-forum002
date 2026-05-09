#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAILWIND_VERSION="${TAILWIND_VERSION:-v3.4.13}"
CACHE_DIR="$ROOT_DIR/target/tailwind"
BINARY_PATH="$CACHE_DIR/tailwindcss"
INPUT_CSS="$ROOT_DIR/assets/css/app.css"
OUTPUT_CSS="$ROOT_DIR/static/css/app.css"
CONFIG_PATH="$ROOT_DIR/tailwind.config.js"

mkdir -p "$CACHE_DIR" "$(dirname "$OUTPUT_CSS")"

if [[ ! -x "$BINARY_PATH" ]]; then
  case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
      ASSET_NAME="tailwindcss-linux-x64"
      ;;
    Linux-aarch64 | Linux-arm64)
      ASSET_NAME="tailwindcss-linux-arm64"
      ;;
    Darwin-x86_64)
      ASSET_NAME="tailwindcss-macos-x64"
      ;;
    Darwin-arm64)
      ASSET_NAME="tailwindcss-macos-arm64"
      ;;
    *)
      echo "Unsupported platform for Tailwind standalone CLI: $(uname -s)-$(uname -m)" >&2
      exit 1
      ;;
  esac

  curl -fsSL \
    "https://github.com/tailwindlabs/tailwindcss/releases/download/${TAILWIND_VERSION}/${ASSET_NAME}" \
    -o "$BINARY_PATH"
  chmod +x "$BINARY_PATH"
fi

"$BINARY_PATH" \
  --config "$CONFIG_PATH" \
  --input "$INPUT_CSS" \
  --output "$OUTPUT_CSS" \
  --minify
