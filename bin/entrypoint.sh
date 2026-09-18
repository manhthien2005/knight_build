#!/bin/bash
# PID 1 bootstrap for the container (RUNTIME-SPEC §6, A5).
#
# Responsibility split after Task 15:
#   entrypoint.sh  — infrastructure only: X server, WM, noVNC bridge
#   zeus-agent     — everything else: account lifecycle, supervision, Supabase,
#                    SIGTERM handling, zombie reaping, RAM trim via jattach
#
# The final `exec zeus-agent` replaces this shell process, making zeus-agent
# the true PID 1. From that point on:
#   - SIGTERM from `docker stop` lands directly on zeus-agent
#   - zeus-agent must call waitpid(-1, WNOHANG) to reap JVM zombies
#   - All process management (start, stop, restart, trim) belongs to the agent
#
# Infrastructure processes (Xvnc, openbox, websockify) are started in the
# background. If they die before exec, we fail fast. After exec, zeus-agent
# is expected to monitor them and exit (triggering Railway's restart policy)
# if a critical one dies.
#
# ── Restart-storm protection ─────────────────────────────────────────────────
#
# When zeus-agent exits (e.g. pairing fails), Railway restarts the container.
# If it does NOT kill or restart the X infrastructure, entrypoint.sh runs again
# inside the SAME container PID namespace and finds:
#   - Xvnc already running (same display number) → "Server is already active for display 1"
#   - /tmp/.X1-lock from the old Xvnc
#   - openbox/websockify still running
#
# This script detects and terminates those stale processes BEFORE starting new ones.
# It NEVER blindly deletes /tmp/.X*-lock while the old Xvnc is still alive.
set -euo pipefail

DISPLAY_NUM="${DISPLAY_NUM:-1}"
export DISPLAY=":${DISPLAY_NUM}.0"
LOGS=/opt/knight/logs
PORT="${PORT:-6080}"
VNC_PORT=$((5900 + DISPLAY_NUM))
LOCK_FILE="/tmp/.X${DISPLAY_NUM}-lock"
X11_SOCK="/tmp/.X11-unix/X${DISPLAY_NUM}"

log() { printf '[boot] %s\n' "$*"; }
log_err() { printf '[boot][ERROR] %s\n' "$*" >&2; }

mkdir -p "$LOGS"

# ─── Step 1: terminate stale X infrastructure ────────────────────────────────
#
# Strategy: find any existing Xvnc/openbox/websockify that were started in a
# previous invocation of this script. Kill them gracefully, then forcefully,
# then wait for them to exit before touching lock files.

cleanup_stale_processes() {
    local display_num="$1"
    local lock="$2"
    local sock="$3"

    # Find old Xvnc for this display.
    # Xvnc is started with ":N" as its first argument; pgrep -f finds that.
    local old_xvnc_pid
    old_xvnc_pid="$(pgrep -f "Xvnc :${display_num}" 2>/dev/null || true)"

    if [ -n "$old_xvnc_pid" ]; then
        log "Found stale Xvnc (pid=${old_xvnc_pid}), terminating..."
        kill -TERM "$old_xvnc_pid" 2>/dev/null || true
        # Wait up to 5 s for graceful exit.
        local i
        for i in $(seq 25); do
            kill -0 "$old_xvnc_pid" 2>/dev/null || break
            sleep 0.2
        done
        # Still alive? Force kill.
        if kill -0 "$old_xvnc_pid" 2>/dev/null; then
            log "Xvnc did not exit gracefully, sending SIGKILL..."
            kill -KILL "$old_xvnc_pid" 2>/dev/null || true
            sleep 0.5
        fi
        log "Stale Xvnc terminated"
    fi

    # Now that the old Xvnc is confirmed dead, remove stale lock/socket.
    # ONLY do this after confirming the process is gone.
    if [ -f "$lock" ]; then
        # Double-check: read the PID from the lock file and verify it is NOT running.
        local lock_pid
        lock_pid="$(cat "$lock" 2>/dev/null | tr -d '[:space:]' || echo '')"
        if [ -n "$lock_pid" ] && kill -0 "$lock_pid" 2>/dev/null; then
            log_err "Lock file $lock still held by live pid=$lock_pid — NOT removing"
        else
            log "Removing stale lock $lock (held by dead pid=${lock_pid})"
            rm -f "$lock"
        fi
    fi
    if [ -S "$sock" ] || [ -e "$sock" ]; then
        log "Removing stale socket $sock"
        rm -f "$sock"
    fi

    # Kill stale openbox and websockify (these don't have display-specific names,
    # so kill all of them; they will be restarted below).
    for proc in openbox websockify; do
        local old_pid
        old_pid="$(pgrep -x "$proc" 2>/dev/null || true)"
        if [ -n "$old_pid" ]; then
            log "Terminating stale $proc (pid=${old_pid})..."
            kill -TERM "$old_pid" 2>/dev/null || true
            sleep 0.5
            kill -KILL "$old_pid" 2>/dev/null || true
        fi
    done
}

cleanup_stale_processes "$DISPLAY_NUM" "$LOCK_FILE" "$X11_SOCK"

# ─── Step 2: start exactly one Xvnc ─────────────────────────────────────────
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
log "Xvnc started (pid=${XVNC_PID})"

# Wait for Xvnc to bind its socket (up to 10 s).
for _ in $(seq 50); do
    xdotool getdisplaygeometry >/dev/null 2>&1 && break
    sleep 0.2
done
xdotool getdisplaygeometry >/dev/null 2>&1 || {
    log_err "Xvnc failed to start — xdotool check failed"
    cat "$LOGS/xvnc.log"
    exit 1
}

# Verify the X11 socket exists.
[ -S "$X11_SOCK" ] || {
    log_err "X11 socket $X11_SOCK does not exist after Xvnc started"
    exit 1
}

# Verify Xvnc is still alive (may have died after socket appeared).
kill -0 "$XVNC_PID" 2>/dev/null || {
    log_err "Xvnc (pid=${XVNC_PID}) exited immediately"
    cat "$LOGS/xvnc.log"
    exit 1
}
log "Xvnc up on :${DISPLAY_NUM} (${VNC_GEOMETRY:-800x600}x${VNC_DEPTH:-16}, pid=${XVNC_PID})"

# Sanity check: there must be exactly ONE Xvnc for this display after startup.
# Two Xvnc processes for the same display = we failed to clean up the stale one.
XVNC_COUNT="$(pgrep -c -f "Xvnc :${DISPLAY_NUM}" 2>/dev/null || echo 0)"
if [ "$XVNC_COUNT" -gt 1 ]; then
    log_err "BUG: ${XVNC_COUNT} Xvnc processes found for display :${DISPLAY_NUM} — stale cleanup failed"
    exit 1
fi

# ─── Step 3: window manager ──────────────────────────────────────────────────
# openbox only: gives focus handling and movable/decorated frames so both
# emulator tabs can be seen and clicked. No panel, no compositor, no desktop.
openbox >"$LOGS/openbox.log" 2>&1 &
OPENBOX_PID=$!
log "openbox started (pid=${OPENBOX_PID})"

# ─── Step 4: noVNC bridge ────────────────────────────────────────────────────
# No TLS here on purpose: Railway terminates HTTPS at its edge, so the
# browser still gets wss:// while this stays a plain local hop.
websockify --web=/usr/share/novnc/ "0.0.0.0:${PORT}" "localhost:${VNC_PORT}" \
    >"$LOGS/websockify.log" 2>&1 &
WEBSOCKIFY_PID=$!
log "noVNC on :${PORT} -> localhost:${VNC_PORT} (pid=${WEBSOCKIFY_PID})"

# Brief settle: give websockify a moment to bind the port before the agent
# tries to read it. 1 s is conservative — websockify typically binds in <100 ms.
sleep 1

# ─── Step 5: verify all infrastructure is alive ──────────────────────────────
kill -0 "${XVNC_PID}" 2>/dev/null || {
    log_err "Xvnc (pid=${XVNC_PID}) died before handoff"
    cat "$LOGS/xvnc.log"
    exit 1
}
kill -0 "${OPENBOX_PID}" 2>/dev/null || {
    log_err "openbox died at startup"
    cat "$LOGS/openbox.log"
    exit 1
}
kill -0 "${WEBSOCKIFY_PID}" 2>/dev/null || {
    log_err "websockify died at startup"
    cat "$LOGS/websockify.log"
    exit 1
}
log "infrastructure healthy (xvnc=${XVNC_PID} openbox=${OPENBOX_PID} websockify=${WEBSOCKIFY_PID})"

# ─── Step 6: hand off to zeus-agent as PID 1 ─────────────────────────────────
# exec replaces this shell, so zeus-agent inherits PID 1.
# Environment variables set above (DISPLAY, PORT, VNC_PORT, etc.) are visible
# to the agent via std::env::var.
log "exec zeus-agent (PID 1)"
exec zeus-agent
