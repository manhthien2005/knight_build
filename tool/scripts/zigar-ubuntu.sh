#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd -- "$script_dir/.." && pwd)"
zig="$project_root/.devtools/linux/zig-0.16.0/zig"

exec "$zig" ar "$@"
