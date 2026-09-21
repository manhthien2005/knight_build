#!/bin/bash
# sync-build-merged.sh — Build Zeus_Knight.jar with POTATO integrated.
#
# This is the merged pipeline for Task 16 (A3.x). It combines:
#   - zeus/sync-build.sh   (syncs Zeus.java + PatchZeus from host; gate)
#   - potato/build-jar.sh  (PatchCanvas + PatchLayers + POTATO.java/bx.java)
#
# Build order matters — see RUNTIME-SPEC §4.4 A3.2:
#
#   vanilla → PatchZeus → PatchCanvas → PatchLayers
#     → javac(bx, POTATO) against patched CP   ← POTATO bx sees patched classpath
#     → javac(Zeus.java) against patched CP + POTATO classes
#     → overlay all patched classes → repack
#
# Zeus.java MUST be compiled AFTER bx of POTATO is in the classpath, so it sees
# the draw-counter version that will actually run.
#
# Usage (from host):
#   MSYS_NO_PATHCONV=1 docker exec knight-potato bash /host/docker-build/mod/zeus/sync-build-merged.sh
#
# $1 = output jar name (default Zeus_Knight.jar), written to /work/jar/
set -euo pipefail

OUT="${1:-Zeus_Knight.jar}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# -- Paths -------------------------------------------------------------------
JARS=/work/jar
BUILD=/work/merged/build
ZEUS_SRC=/work/merged/zeus/src
ZEUS_TOOLS=/work/merged/zeus/tools
POTATO_SRC=/work/merged/potato/src
POTATO_TOOLS=/work/merged/potato/tools
CP="$JARS/vanilla.jar:$JARS/microemulator.jar"
RELEASE=6

# -- Sync from repository-local source ---------------------------------------
[ -d "$REPO_ROOT/mod/zeus" ]   || { echo "no $REPO_ROOT/mod/zeus — repository missing mod/zeus"; exit 1; }
[ -d "$REPO_ROOT/mod/potato" ] || { echo "no $REPO_ROOT/mod/potato — repository missing mod/potato"; exit 1; }

mkdir -p "$ZEUS_SRC" "$ZEUS_TOOLS" "$POTATO_SRC" "$POTATO_TOOLS" "$BUILD" "$JARS"

if [ -f "$REPO_ROOT/mod/zeus/vanilla.jar" ]; then
    cp "$REPO_ROOT/mod/zeus/vanilla.jar" "$JARS/vanilla.jar"
fi
if [ -f "$REPO_ROOT/vendor/microemulator-2.0.4/microemulator.jar" ]; then
    cp "$REPO_ROOT/vendor/microemulator-2.0.4/microemulator.jar" "$JARS/microemulator.jar"
fi

cp "$REPO_ROOT/mod/zeus/src/Zeus.java"      "$ZEUS_SRC/Zeus.java"
cp "$REPO_ROOT/mod/zeus/tools"/*.java       "$ZEUS_TOOLS/"
cp "$REPO_ROOT/mod/potato/src/POTATO.java"  "$POTATO_SRC/POTATO.java"
cp "$REPO_ROOT/mod/potato/src/bx.java"      "$POTATO_SRC/bx.java"
cp "$REPO_ROOT/mod/potato/tools"/*.java     "$POTATO_TOOLS/"

echo "== synced =="
printf '   Zeus.java  %s lines  CTL_VERSION %s  K_* count %s\n' \
    "$(wc -l < "$ZEUS_SRC/Zeus.java")" \
    "$(grep -oP 'CTL_VERSION\s*=\s*\K[0-9]+' "$ZEUS_SRC/Zeus.java")" \
    "$(grep -oE 'K_[A-Z_0-9]+ *= *[0-9]+' "$ZEUS_SRC/Zeus.java" | sort -u | wc -l)"

# -- Fresh build dir ---------------------------------------------------------
rm -rf "$BUILD"
mkdir -p "$BUILD/classes" "$BUILD/stage" "$BUILD/tools"

# -- Step 1: PatchZeus (fu.class tick hook + x.class k-public) ---------------
echo "== [1/6] compile PatchZeus =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" \
    -d "$BUILD/tools" "$ZEUS_TOOLS/PatchZeus.java"

echo "== [1/6] run PatchZeus =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchZeus \
    "$JARS/vanilla.jar" "$BUILD/classes"

# -- Step 2: PatchCanvas (com/silverknight/a.run rewrite) --------------------
echo "== [2/6] compile PatchCanvas =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" \
    -d "$BUILD/tools" "$POTATO_TOOLS/PatchCanvas.java"

echo "== [2/6] run PatchCanvas =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchCanvas \
    "$JARS/vanilla.jar" "$BUILD/classes/com/silverknight/a.class"

# -- Step 3: PatchLayers (ey.a(bx) + br.a(bx) gate) -------------------------
echo "== [3/6] compile PatchLayers =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" \
    -d "$BUILD/tools" "$POTATO_TOOLS/PatchLayers.java"

echo "== [3/6] run PatchLayers =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchLayers \
    "$JARS/vanilla.jar" "$BUILD/classes"

# -- Step 4: javac(bx, POTATO) against patched CP ----------------------------
# A3.2: bx must be compiled AFTER PatchZeus has run, so the bx.java POTATO
# provides sees the same class context that will run. Both bx and POTATO are
# compiled together here so POTATO.java can reference bx fields/methods.
echo "== [4/6] compile bx.java + POTATO.java (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" \
    -cp "$BUILD/classes:$CP" -d "$BUILD/classes" \
    "$POTATO_SRC/bx.java" "$POTATO_SRC/POTATO.java"

# -- Step 5: javac(Zeus.java) after POTATO bx is in classpath ----------------
# A3.2: Zeus.java calls bx.a()/bx.b() — must see the POTATO-patched bx.
echo "== [5/6] compile Zeus.java (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" \
    -cp "$BUILD/classes:$CP" -d "$BUILD/classes" \
    "$ZEUS_SRC/Zeus.java"

# -- Step 6: overlay + repack -------------------------------------------------
echo "== [6/6] stage vanilla jar =="
cd "$BUILD/stage"
jar xf "$JARS/vanilla.jar"

echo "== [6/6] overlay patched classes =="
cd "$BUILD/classes"
find . -name '*.class' -print0 | while IFS= read -r -d '' f; do
    echo "   $f"
    cp --parents "$f" "$BUILD/stage/"
done

echo "== [6/6] repack → $OUT =="
cd "$BUILD/stage"
jar cfm "$JARS/$OUT" META-INF/MANIFEST.MF .
cd /
ls -la "$JARS/$OUT"

# -- Gate checks -------------------------------------------------------------
echo "== gate checks =="
TMP=$(mktemp -d)
cd "$TMP"
cp "$JARS/$OUT" . && jar xf "$OUT" Zeus.class cn.class

keys=$(javap -p -constants Zeus.class | grep -c 'static final int K_')
ver=$(javap -p -constants Zeus.class | grep -oP 'CTL_VERSION = \K[0-9]+')
paint=$(javap -c cn.class | grep -c 'Zeus.paint')
snapv=$(javap -p -constants -c Zeus.class | grep -oE 'String v=[0-9]' | sort -u | tail -1)

srckeys=$(grep -oE 'K_[A-Z_0-9]+ *= *[0-9]+' "$ZEUS_SRC/Zeus.java" | sort -u | wc -l)

printf '   K_* in jar         %s  (source: %s)\n' "$keys" "$srckeys"
printf '   CTL_VERSION        %s\n' "$ver"
printf '   Zeus.paint sites   %s  (must be 2; 1 = broken patcher)\n' "$paint"
printf '   snapshot header    %s\n' "$snapv"

# Check POTATO.class present
potato_ok=0
jar tf "$JARS/$OUT" | grep -q 'POTATO.class' && potato_ok=1

printf '   POTATO.class       %s\n' "$([ $potato_ok = 1 ] && echo 'present' || echo 'MISSING')"

rc=0
[ "$keys" = "$srckeys" ] || { echo "   !! jar key count differs from source"; rc=1; }
[ "$paint" = "2" ]       || { echo "   !! Zeus.paint != 2 call sites"; rc=1; }
[ "$potato_ok" = "1" ]   || { echo "   !! POTATO.class not in jar"; rc=1; }

# Verify POTATO.guard default is true in SOURCE (not bytecode — it's a runtime field,
# not a compile-time constant, so javap -p -constants won't show 'boolean guard = true').
# Two conditions from RUNTIME-SPEC §4.5:
#   1. Field declaration: public static boolean guard = true;
#   2. intProp("potato.guard", 1) — default=1 means guard stays true unless explicitly disabled
guard_decl_ok=0
guard_prop_ok=0
grep -q 'static boolean guard = true' "$POTATO_SRC/POTATO.java" && guard_decl_ok=1
grep -qP 'intProp\("potato\.guard",\s*1\)' "$POTATO_SRC/POTATO.java" && guard_prop_ok=1

if [ "$guard_decl_ok" = "1" ] && [ "$guard_prop_ok" = "1" ]; then
    printf '   POTATO.guard       true (default=true + intProp default=1)\n'
else
    echo "   !! POTATO.guard default is NOT true in source — never ship this"
    [ "$guard_decl_ok" != "1" ] && echo "     missing: static boolean guard = true"
    [ "$guard_prop_ok" != "1" ] && echo "     missing: intProp(\"potato.guard\", 1)"
    rc=1
fi

# -- Build manifest (only on gate pass) --------------------------------------
if [ "$rc" = 0 ]; then
    JAR_PATH="$JARS/$OUT"
    snapver="${snapv##*v=}"
    jar_sha=$(sha256sum "$JAR_PATH" | cut -d' ' -f1)
    jar_sz=$(stat -c %s "$JAR_PATH")
    patcher_sha=$(sha256sum "$ZEUS_TOOLS/PatchZeus.java" | cut -d' ' -f1)
    built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
    cat > /work/jar/zeus-jar.json <<EOF
{
  "jar_sha256":         "$jar_sha",
  "jar_size":           $jar_sz,
  "ctl_version":        $ver,
  "snapshot_version":   $snapver,
  "ctl_key_count":      $keys,
  "snapshot_key_count": $(javap -p -constants -c Zeus.class | grep -coE 'String [a-z]+='),
  "built_at":           "$built_at",
  "patcher_sha256":     "$patcher_sha"
}
EOF
    echo "   zeus-jar.json written"
    if [ -d "$REPO_ROOT/vendor/game" ]; then
        cp "$JAR_PATH" "$REPO_ROOT/vendor/game/$OUT"
        cp /work/jar/zeus-jar.json "$REPO_ROOT/vendor/game/zeus-jar.json"
        echo "   copied $OUT and zeus-jar.json to $REPO_ROOT/vendor/game/"
    fi
fi

cd / && rm -rf "$TMP"
[ "$rc" = 0 ] && echo "== OK ==" || { echo "== FAILED — do not deploy =="; exit 1; }
