#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd -- "$script_dir/.." && pwd)"

export RUSTUP_HOME="$project_root/.devtools/linux/rustup"
export CARGO_HOME="$project_root/.devtools/linux/cargo"
export ZIG_GLOBAL_CACHE_DIR="$project_root/.devtools/linux/zig-global-cache"
export ZIG_LOCAL_CACHE_DIR="${ZEUS_HSO_UBUNTU_ZIG_CACHE_DIR:-/tmp/zeus-hso-zig-cache}"
export CARGO_TARGET_DIR="${ZEUS_HSO_UBUNTU_TARGET_DIR:-/tmp/zeus-hso-cargo-target}"
export CC_x86_64_unknown_linux_gnu="$script_dir/zigcc-ubuntu.sh"
export AR_x86_64_unknown_linux_gnu="$script_dir/zigar-ubuntu.sh"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$script_dir/zigcc-ubuntu.sh"
export CRATE_CC_NO_DEFAULTS=1
export PATH="$CARGO_HOME/bin:$PATH"

if [[ ! -x "$CARGO_HOME/bin/cargo" ]]; then
    echo "Local Ubuntu Cargo toolchain is missing. See README.md for bootstrap details." >&2
    exit 1
fi

exec "$CARGO_HOME/bin/cargo" "$@"
