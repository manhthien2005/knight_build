#!/usr/bin/env bash
# build-agent-linux.sh — Build zeus-agent as a static Linux binary and stage it
# for the Docker build context.
#
# Usage (from repo root or from Tool/tool/scripts/):
#   bash Tool/tool/scripts/build-agent-linux.sh
#
# Prerequisites:
#   - Docker running (used to cross-compile with a Rust musl toolchain)
#   - Or: native Linux / WSL2 with rustup + x86_64-unknown-linux-musl target
#
# Output: docker-build/vendor/tools/zeus-agent (static ELF, no glibc dep)
#
# The binary is intentionally NOT checked into git (it is a binary artifact).
# Regenerate with this script before `docker build`.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
TOOL_DIR="$REPO_ROOT/Tool/tool"
OUT_DIR="$REPO_ROOT/docker-build/vendor/tools"
TARGET="x86_64-unknown-linux-musl"
BINARY="$TOOL_DIR/target/$TARGET/release/zeus-agent"

cd "$TOOL_DIR"

# Detect if we are on Linux natively or need Docker for cross-compile.
if [[ "$(uname -s)" == "Linux" ]]; then
    echo "[build-agent] Building natively on Linux..."
    # Ensure the musl target is installed.
    RUSTUP_TOOLCHAIN=stable rustup target add "$TARGET" 2>/dev/null || true
    RUSTUP_TOOLCHAIN=stable \
        cargo build --release --target "$TARGET" -p zeus-agent
else
    echo "[build-agent] Non-Linux host — building via Docker..."
    docker run --rm \
        -v "$TOOL_DIR:/build" \
        -w /build \
        rust:1-slim \
        bash -euc "
            rustup target add $TARGET
            apt-get update -qq && apt-get install -y -qq musl-tools
            RUSTUP_TOOLCHAIN=stable cargo build --release --target $TARGET -p zeus-agent
        "
fi

# Verify static linkage.
if command -v file >/dev/null 2>&1; then
    file "$BINARY" | grep -q 'statically linked' \
        || { echo "[build-agent] ERROR: binary is NOT statically linked"; exit 1; }
fi

# Copy to Docker build context.
mkdir -p "$OUT_DIR"
cp "$BINARY" "$OUT_DIR/zeus-agent"
chmod +x "$OUT_DIR/zeus-agent"

echo "[build-agent] OK: $OUT_DIR/zeus-agent"
echo "[build-agent] sha256: $(sha256sum "$OUT_DIR/zeus-agent" | awk '{print $1}')"
echo "[build-agent] size:   $(stat -c %s "$OUT_DIR/zeus-agent" 2>/dev/null || stat -f %z "$OUT_DIR/zeus-agent") bytes"
