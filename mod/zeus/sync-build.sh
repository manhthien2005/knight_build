#!/bin/bash
# Sync the host source into the container, then build Zeus_Knight.jar.
#
# Exists because /work is container-internal, not a bind mount: only /host is.
# Building straight from /work/zeus/src silently uses whatever was copied there
# last — which is how a jar at CTL_VERSION 13 with 29 keys got deployed against
# a source at CTL_VERSION 13 with 35 keys. docs/19 and docs/21 both warn about
# that trap; this script removes it instead of warning about it.
#
#   docker exec knight-potato bash /host/docker-build/mod/zeus/sync-build.sh [out.jar]
#
# $1 = output jar name (default Zeus_Knight.jar), written to /work/jar/
set -euo pipefail

OUT="${1:-Zeus_Knight.jar}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SRC="$REPO_ROOT/mod/zeus"
DST=/work/zeus

[ -d "$SRC" ] || { echo "khong thay $SRC — repository missing mod/zeus"; exit 1; }

mkdir -p "$DST/src" "$DST/tools"
if [ -f "$SRC/vanilla.jar" ]; then
    cp "$SRC/vanilla.jar" /work/jar/vanilla.jar
fi
if [ -f "$REPO_ROOT/vendor/microemulator-2.0.4/microemulator.jar" ]; then
    cp "$REPO_ROOT/vendor/microemulator-2.0.4/microemulator.jar" /work/jar/microemulator.jar
fi
cp "$SRC/src/Zeus.java"        "$DST/src/Zeus.java"
cp "$SRC/tools"/*.java         "$DST/tools/"
cp "$SRC/build-zeus-jar.sh"    "$DST/build-zeus-jar.sh"
chmod +x "$DST/build-zeus-jar.sh"

echo "== source da sync =="
printf '   Zeus.java   %s dong  md5 %s\n' \
    "$(wc -l < "$DST/src/Zeus.java")" \
    "$(md5sum "$DST/src/Zeus.java" | cut -c1-12)"
printf '   K_* hang    %s\n' \
    "$(grep -oE 'K_[A-Z_0-9]+ *= *[0-9]+' "$DST/src/Zeus.java" | sort -u | wc -l)"
printf '   CTL_VERSION %s\n' \
    "$(grep -oP 'CTL_VERSION *= *\K[0-9]+' "$DST/src/Zeus.java")"
printf '   PatchZeus   %s hook tai RETURN\n' \
    "$(grep -c 'opcode == Opcodes.RETURN' "$DST/tools/PatchZeus.java")"

bash "$DST/build-zeus-jar.sh" "$OUT"

# Gate: the four properties that tell a good build from the one that shipped broken.
echo "== nghiem thu =="
TMP=$(mktemp -d)
cd "$TMP"
cp "/work/jar/$OUT" . && jar xf "$OUT" Zeus.class cn.class

keys=$(javap -p -constants Zeus.class | grep -c 'static final int K_')
ver=$(javap -p -constants Zeus.class | grep -oP 'CTL_VERSION = \K[0-9]+')
paint=$(javap -c cn.class | grep -c 'Zeus.paint')
snapv=$(javap -p -constants -c Zeus.class | grep -oE 'String v=[0-9]' | sort -u | tail -1)
snapk=$(javap -p -constants -c Zeus.class | grep -coE 'String [a-z]+=')

srckeys=$(grep -oE 'K_[A-Z_0-9]+ *= *[0-9]+' "$DST/src/Zeus.java" | sort -u | wc -l)

printf '   K_* trong jar      %s  (source: %s)\n' "$keys" "$srckeys"
printf '   CTL_VERSION        %s\n' "$ver"
printf '   Zeus.paint sites   %s  (phai la 2; 1 = patcher cu bi loi)\n' "$paint"
printf '   snapshot           %s, %s khoa\n' "$snapv" "$snapk"

rc=0
[ "$keys" = "$srckeys" ] || { echo "   !! so khoa jar khac source"; rc=1; }
[ "$paint" = "2" ]       || { echo "   !! Zeus.paint khong phai 2 call site"; rc=1; }

# Build manifest (WIRE-CONTRACT.md §2.4). The agent reads this at boot to report
# devices.jar_ctl_version / jar_snapshot_version and must NOT parse bytecode, so the four
# numbers measured above are written down beside the jar and travel with it everywhere the
# jar goes. Emitted only when the gate passed: a jar we refuse to deploy must not get a
# manifest that looks deployable — fail-closed, like zeus-control.txt itself.
if [ "$rc" = 0 ]; then
    JAR_PATH="/work/jar/$OUT"
    # snapv is the whole grep match "String v=6"; strip up to and including the last
    # "v=" so the manifest carries the bare integer the JSON schema expects.
    snapver="${snapv##*v=}"
    jar_sha=$(sha256sum "$JAR_PATH" | cut -d' ' -f1)
    jar_sz=$(stat -c %s "$JAR_PATH")
    patcher_sha=$(sha256sum "$DST/tools/PatchZeus.java" | cut -d' ' -f1)
    built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > /work/jar/zeus-jar.json <<EOF
{
  "jar_sha256":         "$jar_sha",
  "jar_size":           $jar_sz,
  "ctl_version":        $ver,
  "snapshot_version":   $snapver,
  "ctl_key_count":      $keys,
  "snapshot_key_count": $snapk,
  "built_at":           "$built_at",
  "patcher_sha256":     "$patcher_sha"
}
EOF
    printf '   zeus-jar.json    /work/jar/zeus-jar.json (ctl v%s/%s khoa, snapshot v%s/%s khoa)\n' \
        "$ver" "$keys" "$snapver" "$snapk"
    if [ -d "$REPO_ROOT/vendor/game" ]; then
        cp "$JAR_PATH" "$REPO_ROOT/vendor/game/$OUT"
        cp /work/jar/zeus-jar.json "$REPO_ROOT/vendor/game/zeus-jar.json"
    fi
fi

cd / && rm -rf "$TMP"
[ "$rc" = 0 ] && echo "   OK" || echo "   THAT BAI — dung deploy jar nay"
exit "$rc"
