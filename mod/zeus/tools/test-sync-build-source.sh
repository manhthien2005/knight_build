#!/usr/bin/env bash
# Focused source-sync regression only. Never invokes build-zeus-jar.sh.
set -euo pipefail

SCRIPT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/sync-build.sh}"
[ -f "$SCRIPT" ] || { echo "Missing sync-build.sh: $SCRIPT" >&2; exit 1; }

SOURCES=(
    BossCatalog.java BossHuntState.java BossHuntConfig.java
    BossScheduleParser.java BossScheduleReader.java BossHuntScheduler.java
    BossHuntController.java Zeus.java
)

SANDBOX=$(mktemp -d)
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/repo/mod/zeus/src" "$SANDBOX/repo/mod/zeus/tools" "$SANDBOX/dst/src"
SRC="$SANDBOX/repo/mod/zeus"
export TEST_DST="$SANDBOX/dst"

# Extract the actual sync path and replace only the absolute destination.
# Stop before build, bytecode probes or vendor writes can execute.
awk '
    /^DST=\/work\/zeus$/ { print "DST=\"$TEST_DST\""; next }
    /^echo "== source da sync =="$/ { print "echo SYNC_BOUNDARY_REACHED"; found=1; exit }
    { print }
    END { if (!found) exit 2 }
' "$SCRIPT" > "$SRC/sync-build.sh"
chmod +x "$SRC/sync-build.sh"

for file in "${SOURCES[@]}"; do
    printf 'fresh current source for %s\n' "$file" > "$SRC/src/$file"
    printf 'STALE older source for %s\n' "$file" > "$TEST_DST/src/$file"
done
printf 'class PatchZeus {}\n' > "$SRC/tools/PatchZeus.java"
printf '#!/usr/bin/env bash\nexit 99\n' > "$SRC/build-zeus-jar.sh"

# SYNC-01: every formerly stale destination must be replaced by fresh source.
OUTPUT=$(bash "$SRC/sync-build.sh" 2>&1) || {
    printf 'SYNC-01 FAIL: sync returned nonzero: %s\n' "$OUTPUT" >&2; exit 1;
}
grep -q '^SYNC_BOUNDARY_REACHED$' <<< "$OUTPUT" || {
    echo 'SYNC-01 FAIL: boundary not reached' >&2; exit 1;
}
for file in "${SOURCES[@]}"; do
    cmp -s "$SRC/src/$file" "$TEST_DST/src/$file" || {
        echo "SYNC-01 FAIL: absent/stale $file" >&2; exit 1;
    }
done
echo 'SYNC-01 PASS: eight sources synchronized with exact content'

# SYNC-02: missing source must be rejected even if destination is populated.
rm "$SRC/src/BossHuntController.java"
printf 'STALE fallback\n' > "$TEST_DST/src/BossHuntController.java"
if OUTPUT=$(bash "$SRC/sync-build.sh" 2>&1); then
    echo 'SYNC-02 FAIL: missing source accepted' >&2; exit 1
fi
if grep -q '^SYNC_BOUNDARY_REACHED$' <<< "$OUTPUT"; then
    echo 'SYNC-02 FAIL: synchronization proceeded to build boundary' >&2; exit 1
fi
grep -q 'BossHuntController.java' <<< "$OUTPUT" || {
    echo 'SYNC-02 FAIL: missing source not identified' >&2; exit 1;
}
[ "$(cat "$TEST_DST/src/BossHuntController.java")" = 'STALE fallback' ] || {
    echo 'SYNC-02 FAIL: destination changed during missing-source preflight' >&2; exit 1;
}
echo 'SYNC-02 PASS: missing source rejected before build, stale destination not reused'
