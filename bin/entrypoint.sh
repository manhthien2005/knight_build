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
set -uo pipefail

DISPLAY_NUM="${DISPLAY_NUM:-1}"
export DISPLAY=":${DISPLAY_NUM}.0"
LOGS=/opt/knight/logs
PORT="${PORT:-6080}"
VNC_PORT=$((5900 + DISPLAY_NUM))

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
xdotool getdisplaygeometry >/dev/null 2>&1 || {
    log "Xvnc failed to start"
    cat "$LOGS/xvnc.log"
    exit 1
}
log "Xvnc up on :${DISPLAY_NUM} (${VNC_GEOMETRY:-800x600}x${VNC_DEPTH:-16})"

# --- window manager ------------------------------------------------------
# openbox only: gives focus handling and movable/decorated frames so both
# emulator tabs can be seen and clicked. No panel, no compositor, no desktop.
openbox >"$LOGS/openbox.log" 2>&1 &
log "openbox started"

# --- noVNC bridge --------------------------------------------------------
# No TLS here on purpose: Railway terminates HTTPS at its edge, so the
# browser still gets wss:// while this stays a plain local hop.
websockify --web=/usr/share/novnc/ "0.0.0.0:${PORT}" "localhost:${VNC_PORT}" \
    >"$LOGS/websockify.log" 2>&1 &
log "noVNC on :${PORT} -> localhost:${VNC_PORT}"

# Brief settle: give websockify a moment to bind the port before the agent
# tries to read it. 1 s is conservative — websockify typically binds in <100 ms.
sleep 1

# --- hand off to zeus-agent as PID 1 ------------------------------------
# exec replaces this shell, so zeus-agent inherits PID 1.
# Environment variables set above (DISPLAY, PORT, VNC_PORT, etc.) are visible
# to the agent via std::env::var.
log "exec zeus-agent (PID 1)"
exec zeus-agent
