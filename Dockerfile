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
# CRITICAL: builder must use the SAME glibc as the runtime (ubuntu:22.04 = 2.35).
# rust:1-slim is currently based on Debian Trixie which ships glibc 2.39.
# A binary compiled against 2.39 cannot run on 2.35 → GLIBC_2.39 not found crash.
#
# Solution: build inside ubuntu:22.04 itself, install Rust toolchain via rustup.
# This guarantees the compiled binary uses glibc 2.35 and loads cleanly in Stage 3.
FROM ubuntu:22.04 AS agent-builder
ENV DEBIAN_FRONTEND=noninteractive
# Build toolchain: curl (rustup), ca-certificates, gcc (cc linker), pkg-config,
# libssl-dev (needed by some ureq TLS feature even when using rustls), perl (ring crate).
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        curl ca-certificates gcc libc6-dev pkg-config libssl-dev perl make \
 && rm -rf /var/lib/apt/lists/*

# Install Rust stable via rustup (non-interactive, no PATH modification needed in RUN).
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path

WORKDIR /src
# Only copy Rust workspace — not the whole repo — to keep layer caches tight.
COPY tool .

# CRITICAL: rust-toolchain.toml in ./tool pins channel = "stable-x86_64-pc-windows-gnu"
# for the Windows dev host (avoids /usr/bin/link coreutils collision under Git Bash).
# RUSTUP_TOOLCHAIN env var outranks rust-toolchain.toml per rustup precedence rules.
# (Documented in rust-toolchain.toml comment: "the Linux agent build must override it
#  rather than edit this file — RUSTUP_TOOLCHAIN=stable cargo build …")
ENV RUSTUP_TOOLCHAIN=stable

# Verify: host must be x86_64-unknown-linux-gnu, NOT x86_64-pc-windows-gnu.
# If this RUN step shows a Windows host, the build is wrong and must be fixed before
# the cargo build step wastes time producing a PE32+ binary.
RUN echo '=== toolchain verification ===' && \
    rustc --version && \
    rustc -vV && \
    cargo --version && \
    rustup show active-toolchain && \
    echo '=== toolchain OK ==='

RUN cargo build --release -p zeus-agent

# Regression gate: confirm the output is an ELF Linux binary, NOT a Windows PE32+.
# If this fails, the toolchain override above is not working correctly.
RUN echo '=== binary format check ===' && \
    file target/release/zeus-agent && \
    file target/release/zeus-agent | grep -q "ELF 64-bit" || \
        { echo "FATAL: zeus-agent is not an ELF binary — toolchain misconfigured"; exit 1; } && \
    echo '=== binary format OK (ELF 64-bit) ==='
# Output: /src/target/release/zeus-agent

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

# zeus-agent: built in stage 2, not pre-compiled vendor binary.
COPY --from=agent-builder /src/target/release/zeus-agent /usr/local/bin/zeus-agent

# ── ABI / glibc smoke gate ────────────────────────────────────────────────────
# Verify the compiled binary can actually be loaded inside THIS runtime image.
# This catches glibc version mismatches and Windows PE32+ binaries at build time.
#
# 1. ldd --version   : print runtime glibc (must be 2.35 for ubuntu:22.04)
# 2. ldd zeus-agent  : list shared lib deps; any "not found" = build failure
# 3. ld-linux --verify: ELF interpreter directly verifies the binary is loadable
# 4. ZEUS_SMOKE_TEST=1: actually execute the binary (early-exit, no network calls)
RUN set -eu; \
    echo '=== runtime glibc version ===' && \
    ldd --version | head -1 && \
    echo '=== ldd zeus-agent ===' && \
    ldd /usr/local/bin/zeus-agent && \
    echo '=== ld-linux verify ===' && \
    /lib64/ld-linux-x86-64.so.2 --verify /usr/local/bin/zeus-agent && \
    echo '=== zeus-agent binary smoke-test ===' && \
    ZEUS_SMOKE_TEST=1 /usr/local/bin/zeus-agent && \
    echo '=== ABI smoke gate PASSED ==='

# Verify immutable artifacts (jar + jattach). zeus-agent sha is no longer pinned
# here — it changes every build. The jar sha256 is cross-checked against the
# manifest field so a jar swapped without its manifest fails the build.
RUN set -eu; \
    jar_sha="$(grep -oP '"jar_sha256":\s*"\K[0-9a-f]{64}' /opt/knight/game/zeus-jar.json)"; \
    printf '%s  %s\n' \
        dbd5f3eb8365d3e839d6a203149e0e3776fc1a0585e16ac1fc23f76c9fcae1c6 /opt/microemulator-2.0.4/microemulator.jar \
        47d822f0a4d30f0d05d34aee0957c194c09b7e3a81c4be343693a12121a0b29a /opt/knight/game/Zeus_Knight.jar \
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


