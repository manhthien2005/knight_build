#!/bin/bash
# Build Zeus_Knight.jar from vanilla + Zeus source.
#
# Reuses the potato-build container infrastructure:
#   - Compile PatchZeus (ASM from microemulator.jar)
#   - Run PatchZeus FIRST to produce fu.class (tick hook) + x.class (k public)
#   - Compile Zeus.java against the PATCHED classes, so javac sees x.k as public
#   - Overlay patched classes + Zeus.class onto vanilla.jar
#
# $1 = output jar name (default Zeus_Knight.jar)
set -euo pipefail

OUT="${1:-Zeus_Knight.jar}"
JARS=/work/jar
SRC=/work/zeus/src
TOOLS=/work/zeus/tools
BUILD=/work/zeus/build
CP="$JARS/vanilla.jar:$JARS/microemulator.jar"
RELEASE=6

rm -rf "$BUILD"
mkdir -p "$BUILD/classes" "$BUILD/stage" "$BUILD/tools"

echo "== compiling PatchZeus =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" -d "$BUILD/tools" \
    "$TOOLS"/PatchZeus.java

echo "== patching fu.class (tick hook) + x.class (k public) =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchZeus \
    "$JARS/vanilla.jar" "$BUILD/classes"

echo "== compiling Zeus.java against the patched classes (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" \
    -cp "$BUILD/classes:$CP" -d "$BUILD/classes" \
    "$SRC"/Zeus.java

echo "== staging vanilla jar =="
cd "$BUILD/stage"
jar xf "$JARS/vanilla.jar"

echo "== overlaying patched classes =="
cd "$BUILD/classes"
find . -name '*.class' -print0 | while IFS= read -r -d '' f; do
    echo "   $f"
    cp --parents "$f" "$BUILD/stage/"
done

echo "== repacking Zeus_Knight.jar =="
cd "$BUILD/stage"
jar cfm "$JARS/$OUT" META-INF/MANIFEST.MF .
cd /
ls -la "$JARS/$OUT"
echo "== done =="