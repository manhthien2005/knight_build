# KnightOnline_402 (J2ME MIDlet) on MicroEmulator 2.0.4, viewed over noVNC.
# Target: Railway 2 vCPU / 1 GiB, 2 emulator tabs.
#
# Stage 1 (jre): jlink runtime + CDS archive.
# Stage 2 (agent): Rust build of zeus-agent.
# Stage 3 (final): runtime image.
#
# Measured effect of jlink stage vs shipping the full eclipse-temurin JRE:
#   image 597 MB -> 533 MB, container RAM 196 MiB -> 185 MiB.
FROM eclipse-temurin:11.0.32_9-jdk-jammy AS jre
RUN jlink \
        --add-modules java.base,java.desktop,java.logging,java.management,java.naming,java.prefs,java.security.jgss,java.instrument,jdk.unsupported,jdk.attach \
        --strip-debug --no-header-files --no-man-pages --compress=2 \
        --output /jre \
 && /jre/bin/java -Xshare:dump

# ── Stage 2: Build zeus-agent (Rust) ─────────────────────────────────────────
# ABI CONTRACT: both this stage and the final runtime use ubuntu:22.04 (glibc 2.35).
# DO NOT change this to rust:1-slim — that image now ships glibc 2.39 (Debian Trixie)
# which produces a binary that cannot load in the ubuntu:22.04 runtime.
FROM ubuntu:22.04 AS agent-builder
ENV DEBIAN_FRONTEND=noninteractive

# Install complete build + verification toolchain.
# file     : ELF format verification (was missing → false "not ELF" failure)
# binutils : readelf, objdump, nm
# build-essential: gcc, g++, make, libc6-dev (replaces separate gcc libc6-dev)
# pkg-config libssl-dev perl: needed by ring/ureq crates
# curl ca-certificates: rustup installer
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        curl \
        ca-certificates \
        build-essential \
        pkg-config \
        libssl-dev \
        perl \
        file \
        binutils \
 && rm -rf /var/lib/apt/lists/*

# Install Rust stable via rustup. RUSTUP_HOME/CARGO_HOME in /usr/local so PATH
# persists correctly across all subsequent RUN layers via the ENV below.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path

WORKDIR /src
# Copy only the Rust workspace to keep layer cache tight.
COPY tool .

# ── Toolchain override ────────────────────────────────────────────────────────
# rust-toolchain.toml pins channel = "stable-x86_64-pc-windows-gnu" for the
# Windows dev host (required to avoid /usr/bin/link Git Bash collision — see
# the comment inside that file). RUSTUP_TOOLCHAIN env var outranks the file
# per rustup precedence: env > rust-toolchain.toml > default.
# The file itself documents this approach on line 9. DO NOT edit that file.
ENV RUSTUP_TOOLCHAIN=stable

# ── Pre-compile host assertion ────────────────────────────────────────────────
# Fail immediately if the active host is not Linux. This prevents wasting the
# full cargo build only to produce a PE32+ binary and a confusing error later.
RUN set -eux; \
    echo '=== Rust toolchain info ==='; \
    rustc --version; \
    rustc -vV; \
    cargo --version; \
    rustup show active-toolchain; \
    HOST="$(rustc -vV | sed -n 's/^host: //p')"; \
    echo "active host: ${HOST}"; \
    test "${HOST}" = "x86_64-unknown-linux-gnu" \
        || { echo "FATAL: rustc host is '${HOST}', expected x86_64-unknown-linux-gnu"; exit 1; }; \
    echo '=== host assertion PASSED ==='

# ── Ensure Linux target is installed ─────────────────────────────────────────
RUN rustup target add x86_64-unknown-linux-gnu

# ── Compile ───────────────────────────────────────────────────────────────────
# Explicit --target prevents any ambient config from redirecting to Windows.
# Output: /src/target/x86_64-unknown-linux-gnu/release/zeus-agent
RUN cargo build --release -p zeus-agent --target x86_64-unknown-linux-gnu

# ── Binary format verification (in builder, where file/readelf are installed) ─
# Each command is a separate test; a missing tool or wrong format causes an
# immediate, clearly-named failure — not a misleading "toolchain misconfigured".
RUN set -eux; \
    BIN="target/x86_64-unknown-linux-gnu/release/zeus-agent"; \
    echo '=== verifying binary exists and is executable ==='; \
    test -f "${BIN}"; \
    test -x "${BIN}"; \
    echo '=== file utility check ==='; \
    command -v file; \
    file "${BIN}" | tee /tmp/zeus-file.txt; \
    grep -q "ELF 64-bit" /tmp/zeus-file.txt \
        || { echo "FAIL: not ELF 64-bit — see above"; cat /tmp/zeus-file.txt; exit 1; }; \
    grep -q "x86-64" /tmp/zeus-file.txt \
        || { echo "FAIL: not x86-64 — see above"; cat /tmp/zeus-file.txt; exit 1; }; \
    echo '=== readelf check ==='; \
    command -v readelf; \
    readelf -h "${BIN}"; \
    readelf -h "${BIN}" | grep -q "Machine:.*X86-64" \
        || { echo "FAIL: readelf Machine is not X86-64"; exit 1; }; \
    echo '=== ldd check (builder glibc) ==='; \
    ldd "${BIN}"; \
    echo '=== binary format verification PASSED ==='

# ── Stage 3: Final runtime image ──────────────────────────────────────────────
FROM ubuntu:22.04

# X server + noVNC bridge + minimal WM. No desktop environment, no browser,
# no systemd/snapd, no audio stack: the game JAR only uses
# javax.microedition.{lcdui,io,rms,midlet} (verified by scanning all 187
# classes), so there is no media/audio code path to support.
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
        iproute2 \
 && (dpkg --purge --force-depends \
        libgl1-mesa-dri libllvm15 \
        python3-numpy python3-babel python-babel-localedata \
        python3-netaddr ieee-data liblapack3 libblas3 libgfortran5 libquadmath0 || true) \
 && rm -rf /var/lib/apt/lists/* /var/cache/apt/archives/*.deb \
           /usr/share/doc /usr/share/man /usr/share/locale/[a-df-z]* \
           /usr/share/novnc/*.md /usr/share/novnc/karma.conf.js /usr/share/novnc/tests \
 && [ -e /usr/share/novnc/index.html ] || ln -s vnc.html /usr/share/novnc/index.html

COPY --from=jre /jre /opt/java
ENV JAVA_HOME=/opt/java \
    PATH=/opt/java/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

COPY vendor/microemulator-2.0.4/microemulator.jar /opt/microemulator-2.0.4/microemulator.jar
COPY vendor/game/Zeus_Knight.jar /opt/knight/game/Zeus_Knight.jar
COPY vendor/game/zeus-jar.json /opt/knight/game/zeus-jar.json
COPY vendor/tools/jattach /usr/local/bin/jattach

# zeus-agent: exact path from the explicit --target build.
# NOT target/release/ — that path is ambiguous and may not exist when
# --target is used. Using the canonical target-qualified path.
COPY --from=agent-builder \
    /src/target/x86_64-unknown-linux-gnu/release/zeus-agent \
    /usr/local/bin/zeus-agent

# ── ABI gate (final runtime stage) ───────────────────────────────────────────
# Verify the binary is loadable inside THIS ubuntu:22.04 environment.
# file/readelf are NOT installed here (build tools only). ldd is available
# via libc-bin which is always present. ld-linux is in libc6.
# ZEUS_SMOKE_TEST=1 executes the binary itself: early-exit path in main(),
# prints wire constants, exits 0, no network calls.
RUN set -eux; \
    echo '=== final runtime ABI gate ==='; \
    test -x /usr/local/bin/zeus-agent; \
    echo '--- runtime glibc ---'; \
    ldd --version | head -1; \
    echo '--- ldd zeus-agent ---'; \
    ldd /usr/local/bin/zeus-agent | tee /tmp/ldd-out.txt; \
    grep -q "not found" /tmp/ldd-out.txt \
        && { echo "FATAL: unresolved shared lib(s) — see ldd output above"; exit 1; } \
        || true; \
    echo '--- ld-linux ELF verify ---'; \
    /lib64/ld-linux-x86-64.so.2 --verify /usr/local/bin/zeus-agent; \
    echo '--- zeus-agent smoke-test ---'; \
    ZEUS_SMOKE_TEST=1 /usr/local/bin/zeus-agent; \
    echo '=== ABI gate PASSED ==='


# Verify immutable artifacts (jar + jattach). zeus-agent sha is no longer pinned
# here — it changes every build. The jar sha256 is cross-checked against the
# manifest field so a jar swapped without its manifest fails the build.
RUN set -eu; \
    jar_sha="$(grep -oP '"jar_sha256":\s*"\K[0-9a-f]{64}' /opt/knight/game/zeus-jar.json)"; \
    printf '%s  %s\n' \
        dbd5f3eb8365d3e839d6a203149e0e3776fc1a0585e16ac1fc23f76c9fcae1c6 /opt/microemulator-2.0.4/microemulator.jar \
        01bbf0575badcbf5e47231c78ce6a5d848f7110650328fce7e6252b196473478 /opt/knight/game/Zeus_Knight.jar \
        a08cb795a1e8d11ea6c2dd6adf8c9edead9a7c3bbca07681dad79cc3eaec0ef4 /usr/local/bin/jattach \
        "$jar_sha" /opt/knight/game/Zeus_Knight.jar \
    | sha256sum -c - \
 && chmod +x /usr/local/bin/jattach \
 && chmod +x /usr/local/bin/zeus-agent

COPY bin/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh \
 && mkdir -p /opt/knight/logs /opt/knight/state

# Supabase URL + anon key are compile-time constants in zeus-agent binary.
# Only env vars needed at runtime:
#   ZEUS_DEVICE_NAME  (optional, default: knight-node)
#   RAILWAY_SERVICE_ID (for stable keypair across redeploys, injected by Railway)
# No email/password needed — device auth is derived from the keypair.
ENV MALLOC_ARENA_MAX=2 \
    HOME=/root \
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
# Health check: verify websockify is bound on $PORT. bash /dev/tcp is available
# without curl in the base image. start-period covers Xvnc + JVM boot time.
# Railway's health policy uses its own TCP check, but this HEALTHCHECK is used
# by `docker ps` and CI smoke tests.
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD bash -c '</dev/tcp/localhost/6080' || exit 1
# entrypoint.sh sets up X + noVNC, then exec's zeus-agent as PID 1.
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]


