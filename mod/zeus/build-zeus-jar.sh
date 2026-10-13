#!/bin/bash
# Build Zeus_Knight.jar from vanilla + Zeus source on official v4.0.3.
#
# Build pipeline:
#   - Compile PatchZeus (ASM from microemulator.jar)
#   - Run PatchZeus to patch v4.0.3 bytecode:
#       * Main/GameCanvas.update(): injects Zeus.tick()
#       * GameScreen/SelectCharScreen.selectChar: widens to public
#       * InterfaceComponents/MsgDialog.cmdList: widens to public
#       * netcommand/Cmd_Message.send(): injects Zeus.sent()
#       * Model/Menu2.startAt() & setinfoDynamic(): injects Zeus.menu() and Zeus.serverMenu()
#       * GameScreen/GameScreen.paint(): injects Zeus.paint()
#   - Compile Zeus.java against patched classes + vanilla + microemulator
#   - Overlay patched classes + Zeus.class onto vanilla.jar
#   - Repack final Zeus_Knight.jar
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

[ -f "$SRC/Zeus.java" ] || { echo "khong thay $SRC/Zeus.java — missing Zeus source"; exit 1; }
[ -f "$SRC/BossCatalog.java" ] || { echo "khong thay $SRC/BossCatalog.java — missing BossCatalog source"; exit 1; }
[ -f "$SRC/BossHuntState.java" ] || { echo "khong thay $SRC/BossHuntState.java — missing BossHuntState source"; exit 1; }
[ -f "$SRC/BossHuntConfig.java" ] || { echo "khong thay $SRC/BossHuntConfig.java — missing BossHuntConfig source"; exit 1; }
[ -f "$SRC/BossScheduleParser.java" ] || { echo "khong thay $SRC/BossScheduleParser.java — missing BossScheduleParser source"; exit 1; }
[ -f "$SRC/BossScheduleReader.java" ] || { echo "khong thay $SRC/BossScheduleReader.java — missing BossScheduleReader source"; exit 1; }
[ -f "$SRC/BossHuntScheduler.java" ] || { echo "khong thay $SRC/BossHuntScheduler.java — missing BossHuntScheduler source"; exit 1; }
[ -f "$SRC/BossHuntController.java" ] || { echo "khong thay $SRC/BossHuntController.java — missing BossHuntController source"; exit 1; }

rm -rf "$BUILD"
mkdir -p "$BUILD/classes" "$BUILD/stage" "$BUILD/tools"

echo "== compiling PatchZeus =="
javac -nowarn -encoding UTF-8 -cp "$JARS/microemulator.jar" -d "$BUILD/tools" \
    "$TOOLS"/PatchZeus.java

echo "== running PatchZeus against v4.0.3 =="
java -cp "$BUILD/tools:$JARS/microemulator.jar" PatchZeus \
    "$JARS/vanilla.jar" "$BUILD/classes"

echo "== compiling sources against patched classes (--release $RELEASE) =="
javac -nowarn -encoding UTF-8 --release "$RELEASE" \
    -cp "$BUILD/classes:$CP" -d "$BUILD/classes" \
    "$SRC/BossCatalog.java" "$SRC/BossHuntState.java" "$SRC/BossHuntConfig.java" \
    "$SRC/BossScheduleParser.java" "$SRC/BossScheduleReader.java" \
    "$SRC/BossHuntScheduler.java" "$SRC/BossHuntController.java" "$SRC/Zeus.java"

echo "== verifying generated classes =="
for cls in BossCatalog.class BossHuntState.class BossHuntConfig.class \
    BossHuntScheduler.class 'BossHuntScheduler$ScheduleEntry.class' \
    BossHuntController.class 'BossHuntController$MapResolver.class' \
    BossScheduleParser.class 'BossScheduleParser$Result.class' 'BossScheduleParser$Status.class' 'BossScheduleParser$ZoneInfo.class' \
    BossScheduleReader.class 'BossScheduleReader$Entry.class' 'BossScheduleReader$ScheduleReport.class'; do
    if [ ! -f "$BUILD/classes/$cls" ]; then
        echo "ERROR: Required class $cls was not generated in $BUILD/classes" >&2
        exit 1
    fi
done

echo "== staging vanilla jar =="
cd "$BUILD/stage"
jar xf "$JARS/vanilla.jar"

echo "== overlaying patched classes =="
cd "$BUILD/classes"
find . -name '*.class' -print0 | while IFS= read -r -d '' f; do
    echo "   $f"
    cp --parents "$f" "$BUILD/stage/"
done

echo "== repacking Zeus_Knight.jar reproducibly =="
python3 -c "
import os, zipfile
stage = '$BUILD/stage'
out_jar = '$JARS/$OUT'
fixed_time = (2026, 1, 1, 0, 0, 0)
with zipfile.ZipFile(out_jar, 'w', compression=zipfile.ZIP_DEFLATED) as zout:
    manifest = os.path.join(stage, 'META-INF/MANIFEST.MF')
    if os.path.exists(manifest):
        zinfo = zipfile.ZipInfo('META-INF/MANIFEST.MF', date_time=fixed_time)
        zinfo.compress_type = zipfile.ZIP_DEFLATED
        with open(manifest, 'rb') as f:
            zout.writestr(zinfo, f.read())
    all_files = []
    for root, dirs, files in os.walk(stage):
        for f in files:
            full = os.path.join(root, f)
            rel = os.path.relpath(full, stage).replace(os.sep, '/')
            if rel != 'META-INF/MANIFEST.MF':
                all_files.append((rel, full))
    all_files.sort()
    for rel, full in all_files:
        zinfo = zipfile.ZipInfo(rel, date_time=fixed_time)
        zinfo.compress_type = zipfile.ZIP_DEFLATED
        zinfo.external_attr = 0o644 << 16
        with open(full, 'rb') as f:
            zout.writestr(zinfo, f.read())
"
echo "== verifying packaged parser classes in $OUT =="
for cls in BossCatalog.class BossHuntState.class BossHuntConfig.class \
    BossHuntScheduler.class 'BossHuntScheduler$ScheduleEntry.class' \
    BossHuntController.class 'BossHuntController$MapResolver.class' \
    BossScheduleParser.class 'BossScheduleParser$Result.class' 'BossScheduleParser$Status.class' 'BossScheduleParser$ZoneInfo.class' \
    BossScheduleReader.class 'BossScheduleReader$Entry.class' 'BossScheduleReader$ScheduleReport.class'; do
    if ! jar tf "$JARS/$OUT" | grep -Fqx "$cls"; then
        echo "ERROR: Required class $cls missing from $OUT" >&2
        exit 1
    fi
done

cd /
ls -la "$JARS/$OUT"
echo "== done =="
