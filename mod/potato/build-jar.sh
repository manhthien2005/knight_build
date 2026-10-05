#!/bin/bash
# Build a patched jar from vanilla + POTATO using the container's JDK.
#
# Official v4.0.3 architecture:
#   - source: POTATO.java (framerate limiter and layer gating controller).
#   - bytecode:
#       * com/silverknight/TemCanvas.run(): PatchCanvas rewrites repaint() / serviceRepaints()
#         to route through POTATO.doRepaint(Canvas).
#       * Thread_More/MiniMap.paint() & Model/EffectManager.paintAll(): PatchLayers injects
#         POTATO.skipLayer gates directly at method entry.
#       * CLib/mGraphics: PatchGraphics injects POTATO.countDraw() into all 7 primitive
#         drawing methods touching javax.microedition.lcdui.Graphics.
#
# $1 = output jar name (default potato.jar)
set -euo pipefail

OUT="${1:-potato.jar}"
JARS=/work/jar
SRC=/work/src
TOOLS=/work/tools
BUILD=/work/build
CP="$JARS/vanilla.jar:$JARS/microemulator.jar"
RELEASE=6

rm -rf "$BUILD"
mkdir -p "$BUILD/classes" "$BUILD/stage" "$BUILD/tools"

echo "== compiling POTATO.java (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" -cp "$CP" -d "$BUILD/classes" \
    "$SRC"/POTATO.java

echo "== compiling patch tools =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" -d "$BUILD/tools" \
    "$TOOLS"/PatchCanvas.java "$TOOLS"/PatchLayers.java "$TOOLS"/PatchGraphics.java

echo "== rewriting com/silverknight/TemCanvas.run() =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchCanvas \
    "$JARS/vanilla.jar" "$BUILD/classes/com/silverknight/TemCanvas.class"

echo "== gating draw layers (MiniMap, EffectManager) =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchLayers \
    "$JARS/vanilla.jar" "$BUILD/classes"

echo "== patching graphics draw counter (CLib/mGraphics) =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchGraphics \
    "$JARS/vanilla.jar" "$BUILD/classes"

echo "== staging vanilla jar =="
cd "$BUILD/stage"
jar xf "$JARS/vanilla.jar"

echo "== overlaying patched classes =="
cd "$BUILD/classes"
find . -name '*.class' -print0 | while IFS= read -r -d '' f; do
    echo "   $f"
    cp --parents "$f" "$BUILD/stage/"
done

echo "== repacking =="
cd "$BUILD/stage"
jar cfm "$JARS/$OUT" META-INF/MANIFEST.MF .
cd /
ls -la "$JARS/$OUT"
