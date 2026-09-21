#!/bin/bash
# Build a patched jar from vanilla + POTATO using the container's JDK.
#
# Two mechanisms, because ProGuard's output constrains what can be recompiled:
#   - source: POTATO (new) and bx (draw counter). bx lives in the default
#     package and only needs default-package neighbours, so it recompiles.
#   - bytecode: com/silverknight/a, ey and br. `a` is in a named package but
#     calls fu, bl, bx, du, dx in the default package, which Java source cannot
#     reference at all, so PatchCanvas rewrites its run() with ASM. ey and br do
#     recompile, but their call sites all live in cn.a(bx) and cn does not
#     survive CFR, so PatchLayers injects the gate into them directly.
#
# Never recompiled: fu, eh, do — they reference the class literally named `do`,
# a Java keyword, so their decompiled source is not valid Java. They stay as
# vanilla bytecode, which is also why no hook is placed inside them.
#
# $1 = output jar name (default potato.jar)
set -euo pipefail

OUT="${1:-potato.jar}"
JARS=/work/jar
SRC=/work/src
TOOLS=/work/tools
BUILD=/work/build
CP="$JARS/vanilla.jar:$JARS/microemulator.jar"

# MIDlet classes are compiled --release 6 (major 50). MicroEmulator's
# MIDletClassLoader pushes every class it loads through the ASM bundled in
# microemulator.jar (2008 build, ASM 3.x). probe-asm.sh shows its ClassReader
# does not reject the major version, but ASM 3 cannot parse invokedynamic, and
# javac 9+ compiles string concatenation to invokedynamic. Targeting 6 keeps
# concat on StringBuilder and the constant pool free of tags ASM 3 never saw.
# The patch tool itself is only run by the container JDK, so it stays default.
RELEASE=6

rm -rf "$BUILD"
mkdir -p "$BUILD/classes" "$BUILD/stage" "$BUILD/tools"

echo "== compiling patched sources (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" -cp "$CP" -d "$BUILD/classes" \
    "$SRC"/POTATO.java "$SRC"/bx.java

echo "== compiling patch tools =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" -d "$BUILD/tools" \
    "$TOOLS"/PatchCanvas.java "$TOOLS"/PatchLayers.java

echo "== rewriting com/silverknight/a.run() =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchCanvas \
    "$JARS/vanilla.jar" "$BUILD/classes/com/silverknight/a.class"

# Layer gates go in as a two-instruction prologue on ey.a(bx) and br.a(bx)
# rather than a source rebuild: their call sites are all in cn.a(bx), and cn
# does not survive CFR (probe-layers.sh). Writing into the same classes dir as
# everything else, so the overlay step below picks them up.
echo "== gating draw layers (ey, br) =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchLayers \
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
