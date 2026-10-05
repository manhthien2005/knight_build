#!/bin/bash
# Sync host source into container, then build Zeus_Knight.jar on official v4.0.3.
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

echo "== nghiem thu bytecode gates v4.0.3 =="
TMP=$(mktemp -d)
cd "$TMP"
cp "/work/jar/$OUT" .
jar xf "$OUT" \
    Zeus.class \
    Main/GameCanvas.class \
    GameScreen/GameScreen.class \
    GameScreen/SelectCharScreen.class \
    InterfaceComponents/MsgDialog.class \
    netcommand/Cmd_Message.class \
    Model/Menu2.class \
    GameScreen/LogoScreen.class \
    GameScreen/LoginScreen.class \
    META-INF/MANIFEST.MF

keys=$(javap -p -constants Zeus.class | grep -c 'static final int K_')
ver=$(javap -p -constants Zeus.class | grep -oP 'CTL_VERSION = \K[0-9]+')
tick=$(javap -c Main/GameCanvas.class | grep -c 'invokestatic.*Zeus\.tick:()V')
paint=$(javap -c GameScreen/GameScreen.class | grep -c 'invokestatic.*Zeus\.paint:(LCLib/mGraphics;)V')
sent=$(javap -c netcommand/Cmd_Message.class | grep -c 'invokestatic.*Zeus\.sent:(Lnet/Message;)V')
menu=$(javap -c Model/Menu2.class | grep -c 'invokestatic.*Zeus\.menu:(LCLib/mVector;Ljava/lang/String;)Z')
smenu=$(javap -c Model/Menu2.class | grep -c 'invokestatic.*Zeus\.serverMenu:(LCLib/mVector;IILjava/lang/String;)Z')
sc_pub=$(javap -p GameScreen/SelectCharScreen.class | grep -c 'public int selectChar;')
md_pub=$(javap -p InterfaceComponents/MsgDialog.class | grep -c 'public CLib.mVector cmdList;')
mkdir -p vanilla_tmp
(cd vanilla_tmp && jar xf /work/jar/vanilla.jar GameScreen/LogoScreen.class)
vanilla_logo_sha=$(sha256sum vanilla_tmp/GameScreen/LogoScreen.class | cut -d' ' -f1)
final_logo_sha=$(sha256sum GameScreen/LogoScreen.class | cut -d' ' -f1)
logo_pristine=0
[ "$final_logo_sha" = "$vanilla_logo_sha" ] && logo_pristine=1
rm -rf vanilla_tmp
gc_connect_guard=$(javap -c Main/GameCanvas.class | grep -c 'invokestatic.*Zeus\.serverTargetSafe:()Z')
ls_login_guard=$(javap -c -p GameScreen/LoginScreen.class | grep -c 'invokestatic.*Zeus\.serverTargetSafe:()Z')
midlet_ver=$(grep -oP 'MIDlet-Version: *\K[0-9.]+' META-INF/MANIFEST.MF)
potato_class=$(jar tf "$OUT" | grep -c 'POTATO\.class' || true)
bx_class=$(jar tf "$OUT" | grep -c '^bx\.class$' || true)

potato_calls=$(jar tf "$OUT" | grep '\.class$' | while read -r c; do javap -c "${c%.class}" 2>/dev/null; done | grep -cE 'POTATO\.(doRepaint|skipLayer|countDraw)' || true)

pub_bytecode=$(javap -p -constants -c Zeus.class | awk '
  /^[ ]*private static void publish\(long\);/ { in_pub=1; next }
  in_pub && /^[ ]*(public|protected|private|static|\})/ { exit }
  in_pub { print }
')

snapver=""
snapk=0
if [ -n "$pub_bytecode" ]; then
    snapver=$(echo "$pub_bytecode" | grep -oP 'String v=\K[0-9]+' | sort -u | head -1)
    snapk=$(echo "$pub_bytecode" | grep -coE 'String [a-z]+=')
fi

srckeys=$(grep -oE 'K_[A-Z_0-9]+ *= *[0-9]+' "$DST/src/Zeus.java" | sort -u | wc -l)

printf '   K_* trong jar          %s (source: %s)\n' "$keys" "$srckeys"
printf '   CTL_VERSION            %s\n' "$ver"
printf '   snapshot               v%s, %s khoa\n' "$snapver" "$snapk"
printf '   Zeus.tick              %s call sites (phai la 1)\n' "$tick"
printf '   Zeus.paint             %s call sites (phai la 1)\n' "$paint"
printf '   Zeus.sent              %s call sites (phai la 1)\n' "$sent"
printf '   Zeus.menu              %s call sites (phai la 1)\n' "$menu"
printf '   Zeus.serverMenu        %s call sites (phai la 1)\n' "$smenu"
printf '   SelectChar.selectChar  %s public (phai la 1)\n' "$sc_pub"
printf '   MsgDialog.cmdList      %s public (phai la 1)\n' "$md_pub"
printf '   LogoScreen pristine    %s (phai la 1)\n' "$logo_pristine"
printf '   GameCanvas.connect     %s guard (phai la 1)\n' "$gc_connect_guard"
printf '   LoginScreen.login      %s guard (phai la 1)\n' "$ls_login_guard"
printf '   MIDlet-Version         %s (phai la 4.0.3)\n' "$midlet_ver"
printf '   POTATO.class           %s (phai la 0)\n' "$potato_class"
printf '   bx.class               %s (phai la 0)\n' "$bx_class"
printf '   POTATO calls           %s (phai la 0)\n' "$potato_calls"

rc=0
[ "$keys" = "$srckeys" ]   || { echo "   !! so khoa jar khac source"; rc=1; }
[ "$ver" = "15" ]          || { echo "   !! CTL_VERSION khong phai 15"; rc=1; }
[ "$keys" = "38" ]         || { echo "   !! CTL key count khong phai 38"; rc=1; }
[ "$snapver" = "6" ]       || { echo "   !! snapshot version khong phai 6"; rc=1; }
[ "$snapk" = "49" ]        || { echo "   !! snapshot key count khong phai 49"; rc=1; }
[ "$tick" = "1" ]          || { echo "   !! Zeus.tick khong phai 1 call site"; rc=1; }
[ "$paint" = "1" ]         || { echo "   !! Zeus.paint khong phai 1 call site"; rc=1; }
[ "$sent" = "1" ]          || { echo "   !! Zeus.sent khong phai 1 call site"; rc=1; }
[ "$menu" = "1" ]          || { echo "   !! Zeus.menu khong phai 1 call site"; rc=1; }
[ "$smenu" = "1" ]         || { echo "   !! Zeus.serverMenu khong phai 1 call site"; rc=1; }
[ "$sc_pub" = "1" ]        || { echo "   !! SelectCharScreen.selectChar khong phai public"; rc=1; }
[ "$md_pub" = "1" ]        || { echo "   !! MsgDialog.cmdList khong phai public"; rc=1; }
[ "$logo_pristine" = "1" ]    || { echo "   !! LogoScreen.class khong trung vanilla"; rc=1; }
[ "$gc_connect_guard" = "1" ] || { echo "   !! GameCanvas.connect serverTargetSafe guard khong phai 1 call site"; rc=1; }
[ "$ls_login_guard" = "1" ]   || { echo "   !! LoginScreen.login serverTargetSafe guard khong phai 1 call site"; rc=1; }
[ "$midlet_ver" = "4.0.3" ]|| { echo "   !! MIDlet-Version khong phai 4.0.3"; rc=1; }
[ "$potato_class" = "0" ]  || { echo "   !! POTATO.class co mat trong jar"; rc=1; }
[ "$bx_class" = "0" ]      || { echo "   !! legacy bx.class co mat trong jar"; rc=1; }
[ "$potato_calls" = "0" ]  || { echo "   !! POTATO calls con ton tai trong jar"; rc=1; }

if [ "$rc" = 0 ]; then
    echo "== running ServerFailClosedTest =="
    javac -encoding UTF-8 -cp "/work/jar/$OUT:/work/jar/microemulator.jar" -d "$TMP" \
        "$DST/tools/ServerFailClosedTest.java"
    java -Djava.awt.headless=true -cp "$TMP:/work/jar/$OUT:/work/jar/microemulator.jar" \
        ServerFailClosedTest

    JAR_PATH="/work/jar/$OUT"
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
