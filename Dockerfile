# KnightOnline_402 (J2ME MIDlet) on MicroEmulator 2.0.4, viewed over noVNC.
# Target: Railway 2 vCPU / 1 GiB, 2 emulator tabs.
#
# Stage 1 builds a jlink runtime with only the modules the emulator actually
# needs, then dumps a CDS archive. Java version is pinned to the runtime already
# smoke-verified on Windows (Temurin 11.0.32+9), so JVM behaviour matches.
#
# Measured effect of this stage vs shipping the full eclipse-temurin JRE:
#   image 597 MB -> 533 MB, and container RAM with 2 tabs 196 MiB -> 185 MiB,
# because the 10 MB CDS archive is mmap'd read-only and shared by both JVMs
# instead of each one filling its own metaspace.
FROM eclipse-temurin:11.0.32_9-jdk-jammy AS jre
RUN jlink \
        --add-modules java.base,java.desktop,java.logging,java.management,java.naming,java.prefs,java.security.jgss,java.instrument,jdk.unsupported,jdk.attach \
        --strip-debug --no-header-files --no-man-pages --compress=2 \
        --output /jre \
 && /jre/bin/java -Xshare:dump

FROM ubuntu:22.04

# X server + noVNC bridge + minimal WM. No desktop environment, no browser,
# no systemd/snapd, no audio stack: the game JAR only uses
# javax.microedition.{lcdui,io,rms,midlet} (verified by scanning all 187
# classes), so there is no media/audio code path to support.
#
# The dpkg --purge line drops packages apt pulled in but nothing here executes:
# libgl1-mesa-dri + libllvm15 (~146 MB) are only a *Recommends* of tigervnc for
# GLX acceleration, which a software framebuffer never touches; the numpy/babel/
# lapack chain came in via websockify; perl modules via a tigervnc helper script.
# Verified after purging that Xvnc, openbox, noVNC and the emulator all still run.
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        tigervnc-standalone-server \
        novnc \
        websockify \
        openbox \
        xdotool \
        fonts-dejavu-core \
        fontconfig \
        libfreetype6 \
        libxext6 libxi6 libxrender1 libxtst6 \
 && dpkg --purge --force-depends \
        libgl1-mesa-dri libllvm15 \
        python3-numpy python3-babel python-babel-localedata \
        python3-netaddr ieee-data liblapack3 libblas3 libgfortran5 libquadmath0 \
 && rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*.deb \
           /usr/share/doc /usr/share/man /usr/share/locale/[a-df-z]* \
           /usr/share/novnc/*.md /usr/share/novnc/karma.conf.js /usr/share/novnc/tests \
 && [ -e /usr/share/novnc/index.html ] || ln -s vnc.html /usr/share/novnc/index.html

COPY --from=jre /jre /opt/java
ENV JAVA_HOME=/opt/java \
    PATH=/opt/java/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

COPY vendor/microemulator-2.0.4/microemulator.jar /opt/microemulator-2.0.4/microemulator.jar
COPY vendor/game/Zeus_Knight.jar /opt/knight/game/Zeus_Knight.jar
# Build manifest (docs/full_spec/tool/WIRE-CONTRACT.md §2.4). The agent reads this at boot to
# report devices.jar_ctl_version / jar_snapshot_version instead of parsing bytecode, so it must
# ship beside the jar it describes. Not sha-pinned below: it is derived data whose own
# jar_sha256 field names the jar, and a malformed copy fails closed when the agent parses it.
COPY vendor/game/zeus-jar.json /opt/knight/game/zeus-jar.json
# jattach: 63 KB static binary. A jlink runtime has no jcmd, and this is the
# only way to force a full GC from outside the JVM so the RAM trim can uncommit.
COPY vendor/tools/jattach /usr/local/bin/jattach

# Fail the build on a corrupted/substituted artifact instead of failing at runtime.
# The fourth line is the same check run through the manifest: zeus-jar.json declares the
# sha256 of the jar it describes, so re-pinning it here means a jar swapped without its
# manifest (or a manifest copied from a different build) fails the build instead of
# silently feeding the agent a wrong jar_ctl_version. grep returning nothing makes the
# line malformed, so a corrupt manifest fails too.
RUN set -eu; \
    jar_sha="$(grep -oP '"jar_sha256":\s*"\K[0-9a-f]{64}' /opt/knight/game/zeus-jar.json)"; \
    printf '%s  %s\n' \
        dbd5f3eb8365d3e839d6a203149e0e3776fc1a0585e16ac1fc23f76c9fcae1c6 /opt/microemulator-2.0.4/microemulator.jar \
        03ac4fa97bee0388edbb05287a91bc2bd1b5a8149d958ce64bbc2014f2b471ee /opt/knight/game/Zeus_Knight.jar \
        a08cb795a1e8d11ea6c2dd6adf8c9edead9a7c3bbca07681dad79cc3eaec0ef4 /usr/local/bin/jattach \
        "$jar_sha" /opt/knight/game/Zeus_Knight.jar \
    | sha256sum -c - \
 && chmod +x /usr/local/bin/jattach

COPY bin/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh && mkdir -p /opt/knight/logs

# 320 MiB heaps: 2x320 worst case still leaves ~250 MiB for X/websockify/OS on
# 1 GiB, and MaxHeapFreeRatio=25 means a tab only holds that much while it is
# genuinely busy. Capping glibc arenas keeps native RSS from drifting upward.
ENV MALLOC_ARENA_MAX=2 \
    HOME=/root \
    ACCOUNTS="acc1 acc2" \
    HEAP_MAX=320m \
    MIN_HEAP_FREE=10 \
    MAX_HEAP_FREE=25 \
    TRIM_INTERVAL=900 \
    TRIM_RSS_KB=180000 \
    VNC_GEOMETRY=800x600 \
    VNC_DEPTH=16 \
    DEVICE_WIDTH=360 \
    DEVICE_HEIGHT=480 \
    PORT=6080

EXPOSE 6080
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
