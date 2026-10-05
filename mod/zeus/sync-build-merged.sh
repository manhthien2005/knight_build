#!/bin/bash
# sync-build-merged.sh — Build Zeus_Knight.jar with POTATO integrated for official v4.0.3.
#
# Merged pipeline for official v4.0.3:
#   1. Copy official mod/zeus/vanilla.jar to /work/jar/vanilla.jar
#   2. Compile and run PatchZeus against v4.0.3
#   3. Compile and run PatchCanvas, writing class to BUILD/classes/com/silverknight/TemCanvas.class
#   4. Compile and run PatchLayers
#   5. Compile and run PatchGraphics
#   6. Compile POTATO.java only (no legacy bx.java)
#   7. Compile Zeus.java against patched classes + official vanilla + microemulator
#   8. Overlay all patched/new classes onto pristine v4.0.3 stage
#   9. Repack final Zeus_Knight.jar
#  10. Run comprehensive bytecode gates before promoting artifact
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
cp "$REPO_ROOT/mod/potato/tools"/*.java     "$POTATO_TOOLS/"

echo "== synced source =="
printf '   Zeus.java    %s lines  CTL_VERSION %s  K_* count %s\n' \
    "$(wc -l < "$ZEUS_SRC/Zeus.java")" \
    "$(grep -oP 'CTL_VERSION\s*=\s*\K[0-9]+' "$ZEUS_SRC/Zeus.java")" \
    "$(grep -oE 'K_[A-Z_0-9]+ *= *[0-9]+' "$ZEUS_SRC/Zeus.java" | sort -u | wc -l)"
printf '   POTATO.java  %s lines\n' \
    "$(wc -l < "$POTATO_SRC/POTATO.java")"

# -- Fresh build dir ---------------------------------------------------------
rm -rf "$BUILD"
mkdir -p "$BUILD/classes" "$BUILD/stage" "$BUILD/tools"

# -- Step 1: PatchZeus against v4.0.3 ----------------------------------------
echo "== [1/8] compile & run PatchZeus =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" \
    -d "$BUILD/tools" "$ZEUS_TOOLS/PatchZeus.java"

java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchZeus \
    "$JARS/vanilla.jar" "$BUILD/classes"

# -- Step 2: PatchCanvas (com/silverknight/TemCanvas.run rewrite) ------------
echo "== [2/8] compile & run PatchCanvas =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" \
    -d "$BUILD/tools" "$POTATO_TOOLS/PatchCanvas.java"

java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchCanvas \
    "$JARS/vanilla.jar" "$BUILD/classes/com/silverknight/TemCanvas.class"

# -- Step 3: PatchLayers (MiniMap & EffectManager gate) -----------------------
echo "== [3/8] compile & run PatchLayers =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" \
    -d "$BUILD/tools" "$POTATO_TOOLS/PatchLayers.java"

java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchLayers \
    "$JARS/vanilla.jar" "$BUILD/classes"

# -- Step 4: PatchGraphics (CLib/mGraphics primitive draw counting) ----------
echo "== [4/8] compile & run PatchGraphics =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" \
    -d "$BUILD/tools" "$POTATO_TOOLS/PatchGraphics.java"

java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchGraphics \
    "$JARS/vanilla.jar" "$BUILD/classes"

# -- Step 5: javac(POTATO.java) against patched CP ---------------------------
echo "== [5/8] compile POTATO.java (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" \
    -cp "$BUILD/classes:$CP" -d "$BUILD/classes" \
    "$POTATO_SRC/POTATO.java"

# -- Step 6: javac(Zeus.java) against patched CP + POTATO --------------------
echo "== [6/8] compile Zeus.java (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" \
    -cp "$BUILD/classes:$CP" -d "$BUILD/classes" \
    "$ZEUS_SRC/Zeus.java"

# -- Step 7: overlay onto pristine v4.0.3 stage ------------------------------
echo "== [7/8] stage vanilla v4.0.3 jar =="
cd "$BUILD/stage"
jar xf "$JARS/vanilla.jar"

echo "== [7/8] overlay patched classes =="
cd "$BUILD/classes"
find . -name '*.class' -print0 | while IFS= read -r -d '' f; do
    echo "   $f"
    cp --parents "$f" "$BUILD/stage/"
done

# -- Step 8: repack final Zeus_Knight.jar -----------------------------------
echo "== [8/8] repack → $OUT =="
cd "$BUILD/stage"
jar cfm "$JARS/$OUT" META-INF/MANIFEST.MF .
cd /
ls -la "$JARS/$OUT"

# -- Step 9: Comprehensive Bytecode Gates -----------------------------------
echo "== [9/8] executing v4.0.3 bytecode gates =="
TMP=$(mktemp -d)
cd "$TMP"
cp "$JARS/$OUT" .

# Gate 1: Extract all 11 required classes
jar xf "$OUT" \
    Zeus.class \
    Main/GameCanvas.class \
    GameScreen/GameScreen.class \
    GameScreen/SelectCharScreen.class \
    InterfaceComponents/MsgDialog.class \
    netcommand/Cmd_Message.class \
    Model/Menu2.class \
    com/silverknight/TemCanvas.class \
    Thread_More/MiniMap.class \
    Model/EffectManager.class \
    CLib/mGraphics.class \
    META-INF/MANIFEST.MF

rc=0

# Gate 2: CTL constants
keys=$(javap -p -constants Zeus.class | grep -c 'static final int K_')
ver=$(javap -p -constants Zeus.class | grep -oP 'CTL_VERSION = \K[0-9]+')
srckeys=$(grep -oE 'K_[A-Z_0-9]+ *= *[0-9]+' "$ZEUS_SRC/Zeus.java" | sort -u | wc -l)

# Gate 3: Snapshot constants from publish(long)
pub_bytecode=$(javap -p -constants -c Zeus.class | awk '
  /^[ ]*private static void publish\(long\);/ { in_pub=1; next }
  in_pub && /^[ ]*(public|protected|private|static|\})/ { exit }
  in_pub { print }
')

snapver=""
snap_key_count=0
if [ -n "$pub_bytecode" ]; then
    snapver=$(echo "$pub_bytecode" | grep -oP 'String v=\K[0-9]+' | sort -u | head -1)
    snap_key_count=$(echo "$pub_bytecode" | grep -coE 'String [a-z]+=')
fi

# Gate 4: Hook call site counts
gc_tick=$(javap -c Main/GameCanvas.class | grep -c 'Zeus.tick' || true)
gs_paint=$(javap -c GameScreen/GameScreen.class | grep -c 'Zeus.paint' || true)
cmd_sent=$(javap -c netcommand/Cmd_Message.class | grep -c 'Zeus.sent' || true)
menu_local=$(javap -c Model/Menu2.class | grep -c 'Zeus.menu' || true)
menu_server=$(javap -c Model/Menu2.class | grep -c 'Zeus.serverMenu' || true)

# Gate 5: Field visibility widening
sc_selectChar_public=$(javap -p GameScreen/SelectCharScreen.class | grep -c 'public int selectChar;' || true)
md_cmdList_public=$(javap -p InterfaceComponents/MsgDialog.class | grep -c 'public CLib.mVector cmdList;' || true)

# Gate 6: POTATO hooks
tc_dorepaint=$(javap -c com/silverknight/TemCanvas.class | grep -c 'POTATO.doRepaint' || true)
tc_parallel_calls=$(javap -c com/silverknight/TemCanvas.class | grep -Ec 'invokevirtual.*(repaint|serviceRepaints)' || true)
mm_skiplayer=$(javap -c Thread_More/MiniMap.class | grep -c 'POTATO.skipLayer' || true)
em_skiplayer=$(javap -c Model/EffectManager.class | grep -c 'POTATO.skipLayer' || true)
mg_countdraw=$(javap -c CLib/mGraphics.class | grep -c 'POTATO.countDraw' || true)

# Gate 7: POTATO presence & guard
potato_present=$(jar tf "$JARS/$OUT" | grep -c '^POTATO.class$' || true)
guard_decl_ok=$(grep -c 'static boolean guard = true' "$POTATO_SRC/POTATO.java" || true)
guard_prop_ok=$(grep -cP 'intProp\("potato\.guard",\s*1\)' "$POTATO_SRC/POTATO.java" || true)

# Gate 8: Manifest version check
manifest_v403=$(grep -c '4.0.3' META-INF/MANIFEST.MF || true)

# Gate 9: No legacy classes
legacy_bx_present=$(jar tf "$JARS/$OUT" | grep -c 'bx.class' || true)
legacy_a_present=$(jar tf "$JARS/$OUT" | grep -c 'com/silverknight/a.class' || true)

printf '   CTL_VERSION                    %s (must be 15)\n' "$ver"
printf '   CTL keys                       %s (must be 38, source: %s)\n' "$keys" "$srckeys"
printf '   Snapshot version               %s (must be 6)\n' "$snapver"
printf '   Snapshot keys                  %s (must be 49)\n' "$snap_key_count"
printf '   Main.GameCanvas.update tick    %s (must be 1)\n' "$gc_tick"
printf '   GameScreen.paint               %s (must be 1)\n' "$gs_paint"
printf '   Cmd_Message.send trace         %s (must be 1)\n' "$cmd_sent"
printf '   Menu2.startAt local menu       %s (must be 1)\n' "$menu_local"
printf '   Menu2.setinfoDynamic server    %s (must be 1)\n' "$menu_server"
printf '   SelectCharScreen.selectChar    %s (must be public: 1)\n' "$sc_selectChar_public"
printf '   MsgDialog.cmdList              %s (must be public: 1)\n' "$md_cmdList_public"
printf '   TemCanvas.run doRepaint        %s (must be 1)\n' "$tc_dorepaint"
printf '   TemCanvas.run parallel repaint %s (must be 0)\n' "$tc_parallel_calls"
printf '   MiniMap.paint skipLayer        %s (must be 1)\n' "$mm_skiplayer"
printf '   EffectManager.paintAll skip    %s (must be 1)\n' "$em_skiplayer"
printf '   CLib.mGraphics countDraw       %s (must be 7)\n' "$mg_countdraw"
printf '   POTATO.class in jar            %s (must be 1)\n' "$potato_present"
printf '   POTATO.guard enabled           %s (decl=%s, prop=%s)\n' \
    "$([ "$guard_decl_ok" = 1 ] && [ "$guard_prop_ok" = 1 ] && echo 'yes' || echo 'NO')" "$guard_decl_ok" "$guard_prop_ok"
printf '   MANIFEST.MF version 4.0.3      %s\n' "$([ "$manifest_v403" -ge 1 ] && echo 'yes' || echo 'NO')"
printf '   Legacy bx.class present        %s (must be 0)\n' "$legacy_bx_present"
printf '   Legacy com/silverknight/a      %s (must be 0)\n' "$legacy_a_present"

[ "$ver" = "15" ]                    || { echo "   !! FAIL: CTL_VERSION != 15"; rc=1; }
[ "$keys" = "38" ]                   || { echo "   !! FAIL: CTL key count != 38"; rc=1; }
[ "$keys" = "$srckeys" ]             || { echo "   !! FAIL: jar key count differs from source"; rc=1; }
[ "$snapver" = "6" ]                 || { echo "   !! FAIL: snapshot version != 6"; rc=1; }
[ "$snap_key_count" = "49" ]         || { echo "   !! FAIL: snapshot key count != 49"; rc=1; }
[ "$gc_tick" = "1" ]                 || { echo "   !! FAIL: GameCanvas tick != 1"; rc=1; }
[ "$gs_paint" = "1" ]                || { echo "   !! FAIL: GameScreen paint != 1"; rc=1; }
[ "$cmd_sent" = "1" ]                || { echo "   !! FAIL: Cmd_Message sent != 1"; rc=1; }
[ "$menu_local" = "1" ]              || { echo "   !! FAIL: Menu2 startAt != 1"; rc=1; }
[ "$menu_server" = "1" ]             || { echo "   !! FAIL: Menu2 setinfoDynamic != 1"; rc=1; }
[ "$sc_selectChar_public" = "1" ]    || { echo "   !! FAIL: SelectCharScreen.selectChar not public"; rc=1; }
[ "$md_cmdList_public" = "1" ]       || { echo "   !! FAIL: MsgDialog.cmdList not public"; rc=1; }
[ "$tc_dorepaint" = "1" ]            || { echo "   !! FAIL: TemCanvas doRepaint != 1"; rc=1; }
[ "$tc_parallel_calls" = "0" ]       || { echo "   !! FAIL: TemCanvas has parallel repaint/serviceRepaints active"; rc=1; }
[ "$mm_skiplayer" = "1" ]            || { echo "   !! FAIL: MiniMap skipLayer != 1"; rc=1; }
[ "$em_skiplayer" = "1" ]            || { echo "   !! FAIL: EffectManager skipLayer != 1"; rc=1; }
[ "$mg_countdraw" = "7" ]            || { echo "   !! FAIL: mGraphics countDraw != 7"; rc=1; }
[ "$potato_present" = "1" ]          || { echo "   !! FAIL: POTATO.class missing from jar"; rc=1; }
[ "$guard_decl_ok" = "1" ]           || { echo "   !! FAIL: POTATO.guard declaration missing"; rc=1; }
[ "$guard_prop_ok" = "1" ]           || { echo "   !! FAIL: POTATO.guard intProp missing"; rc=1; }
[ "$manifest_v403" -ge 1 ]           || { echo "   !! FAIL: MANIFEST.MF does not identify 4.0.3"; rc=1; }
[ "$legacy_bx_present" = "0" ]       || { echo "   !! FAIL: legacy bx.class present in jar"; rc=1; }
[ "$legacy_a_present" = "0" ]        || { echo "   !! FAIL: legacy com/silverknight/a.class present in jar"; rc=1; }

# -- Step 10: Promote artifact & write manifest (only if all gates pass) -----
if [ "$rc" = 0 ]; then
    JAR_PATH="$JARS/$OUT"
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
  "snapshot_key_count": $snap_key_count,
  "built_at":           "$built_at",
  "patcher_sha256":     "$patcher_sha"
}
EOF
    echo "   zeus-jar.json written"
    if [ -d "$REPO_ROOT/vendor/game" ]; then
        cp "$JAR_PATH" "$REPO_ROOT/vendor/game/$OUT"
        cp /work/jar/zeus-jar.json "$REPO_ROOT/vendor/game/zeus-jar.json"
        echo "   promoted $OUT and zeus-jar.json to $REPO_ROOT/vendor/game/"
    fi
fi

cd / && rm -rf "$TMP"
[ "$rc" = 0 ] && echo "== ALL GATES PASSED (OK) ==" || { echo "== GATES FAILED — do not deploy =="; exit 1; }
