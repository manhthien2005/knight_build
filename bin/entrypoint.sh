#!/bin/bash
# PID 1 for the container: X server -> WM -> noVNC bridge -> N emulator tabs,
# then supervise the tabs until the container is stopped.
set -uo pipefail

DISPLAY_NUM="${DISPLAY_NUM:-1}"
export DISPLAY=":${DISPLAY_NUM}.0"
LOGS=/opt/knight/logs
ROOT=/opt/knight/accounts
PORT="${PORT:-6080}"
VNC_PORT=$((5900 + DISPLAY_NUM))
MAX_LOG_BYTES="${MAX_LOG_BYTES:-5242880}"
# Trim cadence: full GC every TRIM_INTERVAL seconds, but only on tabs whose RSS
# already exceeds TRIM_RSS_KB. 0 disables trimming entirely.
TRIM_INTERVAL="${TRIM_INTERVAL:-900}"
TRIM_RSS_KB="${TRIM_RSS_KB:-180000}"

log() { printf '[boot] %s\n' "$*"; }

mkdir -p "$LOGS"

# --- X server ------------------------------------------------------------
# Bound to localhost: the only reachable entry point is websockify on $PORT.
# 16-bit depth halves framebuffer traffic; the MIDlet renders indexed sprites.
Xvnc ":${DISPLAY_NUM}" \
    -geometry "${VNC_GEOMETRY:-800x600}" \
    -depth "${VNC_DEPTH:-16}" \
    -SecurityTypes None \
    -localhost \
    -AlwaysShared \
    -desktop knight \
    >"$LOGS/xvnc.log" 2>&1 &
XVNC_PID=$!

for _ in $(seq 50); do
    xdotool getdisplaygeometry >/dev/null 2>&1 && break
    sleep 0.2
done
xdotool getdisplaygeometry >/dev/null 2>&1 || { log "Xvnc failed"; cat "$LOGS/xvnc.log"; exit 1; }
log "Xvnc up on :${DISPLAY_NUM} (${VNC_GEOMETRY:-800x600}x${VNC_DEPTH:-16})"

# --- window manager ------------------------------------------------------
# openbox only: gives focus handling and movable/decorated frames so both
# emulator tabs can be seen and clicked. No panel, no compositor, no desktop.
openbox >"$LOGS/openbox.log" 2>&1 &
OPENBOX_PID=$!

# --- noVNC bridge --------------------------------------------------------
# No TLS here on purpose: Railway terminates HTTPS at its edge, so the
# browser still gets wss:// while this stays a plain local hop.
websockify --web=/usr/share/novnc/ "0.0.0.0:${PORT}" "localhost:${VNC_PORT}" \
    >"$LOGS/websockify.log" 2>&1 &
WS_PID=$!
log "noVNC on :${PORT} -> localhost:${VNC_PORT}"

# --- emulator tabs -------------------------------------------------------
read -r -a ACCS <<<"${ACCOUNTS:-acc1 acc2}"
declare -A PIDS=()

start_acc() {
    local acc="$1"
    mkdir -p "$ROOT/$acc/home" "$ROOT/$acc/tmp"
    java \
        -Duser.home="$ROOT/$acc/home" \
        -Djava.io.tmpdir="$ROOT/$acc/tmp" \
        -Xms8m \
        -Xmx"${HEAP_MAX:-320m}" \
        -Xss512k \
        -XX:+UseSerialGC \
        -XX:ReservedCodeCacheSize=32m \
        -XX:MaxMetaspaceSize=96m \
        -XX:-UsePerfData \
        -XX:MinHeapFreeRatio="${MIN_HEAP_FREE:-10}" \
        -XX:MaxHeapFreeRatio="${MAX_HEAP_FREE:-25}" \
        -jar /opt/microemulator-2.0.4/microemulator.jar \
        --id "$acc" \
        --rms file \
        --resizableDevice "${DEVICE_WIDTH:-360}" "${DEVICE_HEIGHT:-480}" \
        /opt/knight/game/KnightOnline_402.jar \
        >>"$LOGS/$acc.log" 2>&1 &
    PIDS["$acc"]=$!
    log "$acc pid ${PIDS[$acc]}"
}

# Tile tabs left-to-right as their windows appear, so a fresh container is
# usable over noVNC without dragging windows apart by hand.
place_new_window() {
    local before="$1" col="$2" wid
    for _ in $(seq 100); do
        wid="$(comm -13 <(printf '%s\n' "$before") \
                        <(xdotool search --name MicroEmulator 2>/dev/null | sort) | head -n1)"
        [ -n "$wid" ] && break
        sleep 0.3
    done
    [ -n "$wid" ] || return 0
    xdotool windowmove "$wid" $((col * (DEVICE_WIDTH + 16))) 0 2>/dev/null
}

col=0
for acc in "${ACCS[@]}"; do
    before="$(xdotool search --name MicroEmulator 2>/dev/null | sort)"
    start_acc "$acc"
    place_new_window "$before" "$col"
    col=$((col + 1))
done

# --- shutdown / supervision ---------------------------------------------
shutdown() {
    trap - TERM INT
    log "stopping"
    for acc in "${ACCS[@]}"; do kill "${PIDS[$acc]}" 2>/dev/null; done
    kill "$WS_PID" "$OPENBOX_PID" "$XVNC_PID" 2>/dev/null
    wait
    exit 0
}
trap shutdown TERM INT

# Periodic RAM trim. A SerialGC full GC combined with
# MinHeapFreeRatio/MaxHeapFreeRatio makes the JVM *uncommit* the pages it no
# longer needs, so RSS actually drops instead of only heap "used" dropping.
# Measured on this image: 254 MB RSS -> 194 MB after the garbage was collected.
# Only trimmed above TRIM_RSS_KB so a quiet tab is never paused for nothing.
trim_ram() {
    local acc pid rss
    for acc in "${ACCS[@]}"; do
        pid="${PIDS[$acc]}"
        kill -0 "$pid" 2>/dev/null || continue
        rss="$(awk '/VmRSS/{print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)"
        [ "${rss:-0}" -gt "$TRIM_RSS_KB" ] || continue
        jattach "$pid" jcmd GC.run >/dev/null 2>&1
        sleep 1
        log "trim $acc ${rss}kB -> $(awk '/VmRSS/{print $2}' "/proc/$pid/status" 2>/dev/null)kB"
    done
}

ticks=0
while :; do
    sleep 10
    kill -0 "$XVNC_PID" 2>/dev/null || { log "Xvnc died"; exit 1; }
    kill -0 "$WS_PID" 2>/dev/null || { log "websockify died"; exit 1; }
    for acc in "${ACCS[@]}"; do
        if ! kill -0 "${PIDS[$acc]}" 2>/dev/null; then
            log "$acc exited, restarting"
            before="$(xdotool search --name MicroEmulator 2>/dev/null | sort)"
            start_acc "$acc"
            place_new_window "$before" 0
        fi
        # Unbounded logs would fill the Railway disk on a multi-day session.
        if [ "$(stat -c %s "$LOGS/$acc.log" 2>/dev/null || echo 0)" -gt "$MAX_LOG_BYTES" ]; then
            : >"$LOGS/$acc.log"
        fi
    done
    ticks=$((ticks + 10))
    if [ "$TRIM_INTERVAL" -gt 0 ] && [ "$ticks" -ge "$TRIM_INTERVAL" ]; then
        ticks=0
        trim_ram
    fi
done
