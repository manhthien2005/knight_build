/*
 * POTATO — local-only paint control for KnightOnline_402 (J2ME MIDlet).
 *
 * Owns exactly two things:
 *   1. Paint decoupled from tick. Vanilla com.silverknight.a.run() paints on
 *      every 40 ms iteration, so the only way to cut render cost was to
 *      lengthen the period — which also slowed game logic, and logic is what
 *      faces the server. Here the 40 ms period (25 Hz tick) is untouched and
 *      only painting is skipped.
 *   2. Draw accounting, so "lighter" is a measurement off bx — the single
 *      wrapper every draw passes through — not an estimate.
 *
 * Sends no packet, reads no packet, draws no RNG, and is unreachable from any
 * network handler. doRepaint is called from the canvas loop once per tick.
 */
import javax.microedition.lcdui.Canvas;

public final class POTATO {
    /** paint 1 of every N ticks; 1 = vanilla, 0 = never paint. */
    public static int paintEvery = 1;

    /**
     * Keep painting in states whose logic lives in the draw path (see
     * mustPaint). Measurement-only escape hatch: with no login there is no game
     * scene, so the guard would force paint on every tick and hide the effect
     * of paintEvery. Never ship a tab with guard off.
     */
    public static boolean guard = true;

    /** ticks observed (one per canvas loop iteration). */
    public static long ticks = 0L;
    /** ticks that actually painted. */
    public static long paints = 0L;
    /** bx draw calls, summed across every primitive. */
    public static long draws = 0L;

    /** report interval in ms; 0 disables reporting. */
    public static long reportMs = 0L;
    private static long drawsAtReport = 0L;
    private static long ticksAtReport = 0L;
    private static long paintsAtReport = 0L;
    private static long reportAt = 0L;

    /*
     * Runtime control. The system property alone fixes the mode at launch, but
     * "Ẩn tab" is a checkbox: it has to change while the tab runs. A polled file
     * is used rather than a listening socket because a socket inside the client
     * would be an unauthenticated endpoint any local process could drive, and
     * file permissions already express "who may switch this tab".
     */
    /** control file, or null to leave the mode fixed at its launch value. */
    private static String ctlPath = null;
    /** how often the control file is consulted, in ms. */
    private static long ctlEveryMs = 1000L;
    private static long ctlPolledAt = 0L;
    private static long ctlStamp = -1L;
    /** times the control file changed the mode; surfaced in the report. */
    public static long ctlApplied = 0L;

    /*
     * Layer gating. Independent of paintEvery: paintEvery decides how often the
     * whole frame is drawn, layerMask decides what a drawn frame contains. Both
     * are needed — a visible tab still has to be watchable, so it cannot use a
     * high paintEvery, and dropping the minimap and effects is the part of the
     * frame a player does not miss.
     *
     * A set bit SKIPS that layer, so 0 is vanilla. Bits are assigned in
     * tools/PatchLayers.java, which injects the call site:
     *   1 = ey.a(bx)  minimap
     *   2 = br.a(bx)  effects
     */
    public static int layerMask = 0;
    /** layer draws skipped; surfaced in the report so gating is visible. */
    public static long layerSkips = 0L;
    private static long layerSkipsAtReport = 0L;

    private POTATO() {
    }

    /*
     * Config comes from system properties, not a menu entry, so a launcher can
     * set the mode per tab without the emulator needing focus or input.
     */
    static {
        paintEvery = intProp("potato.paintEvery", 1);
        reportMs = (long) intProp("potato.reportMs", 0);
        guard = intProp("potato.guard", 1) != 0;
        if (paintEvery < 0) {
            paintEvery = 0;
        }
        ctlPath = System.getProperty("potato.ctl");
        if (ctlPath != null && ctlPath.length() == 0) {
            ctlPath = null;
        }
        ctlEveryMs = (long) intProp("potato.ctlEveryMs", 1000);
        if (ctlEveryMs < 100L) {
            ctlEveryMs = 100L;
        }
        layerMask = intProp("potato.layerMask", 0);
        if (layerMask < 0) {
            layerMask = 0;
        }
    }

    /**
     * Whether the caller's draw layer is gated off. Called from the prologue
     * PatchLayers injects into each layer method; returning true makes that
     * method return before drawing anything.
     *
     * Only layers whose bodies were read and found free of gameplay side
     * effects get a bit — see tools/PatchLayers.java for the per-layer argument.
     */
    public static boolean skipLayer(int bit) {
        if ((layerMask & bit) == 0) {
            return false;
        }
        ++layerSkips;
        return true;
    }

    private static int intProp(String key, int fallback) {
        try {
            String v = System.getProperty(key);
            if (v != null && v.length() > 0) {
                return Integer.parseInt(v.trim());
            }
        } catch (Throwable t) {
            // absent or unparseable: keep the vanilla-equivalent default
        }
        return fallback;
    }

    /** Called by bx on every primitive that reaches Graphics. */
    public static void countDraw() {
        ++draws;
    }

    /**
     * Consults the control file when due. Content is one or two integers:
     * "<paintEvery> [layerMask]" — e.g. "10 3" to hide the tab with both layers
     * gated, "1" to restore vanilla painting and leave the mask as it is.
     *
     * Both knobs live in one file so a single write moves the tab to a complete
     * configuration; two files could be read half-applied.
     *
     * lastModified gates the read so a steady state costs one stat per second
     * and no parse. Every failure keeps the current mode: a launcher that
     * truncates the file mid-write must not be able to stall the client.
     */
    private static void pollControl(long now) {
        if (ctlPath == null || now - ctlPolledAt < ctlEveryMs) {
            return;
        }
        ctlPolledAt = now;
        try {
            java.io.File f = new java.io.File(ctlPath);
            long stamp = f.lastModified();
            if (stamp == 0L || stamp == ctlStamp) {
                return;         // absent, or unchanged since the last read
            }
            ctlStamp = stamp;
            int[] want = new int[2];
            int n = readInts(f, want);
            if (n >= 1 && want[0] != paintEvery) {
                paintEvery = want[0];
                ++ctlApplied;
            }
            if (n >= 2 && want[1] != layerMask) {
                layerMask = want[1];
                ++ctlApplied;
            }
        } catch (Throwable t) {
            // unreadable or absent: keep the mode we already have
        }
    }

    /**
     * Parses up to out.length non-negative integers from the file, in order,
     * and returns how many were found. A partially written file yields fewer
     * values, and the caller applies only those — never a garbage default.
     */
    private static int readInts(java.io.File f, int[] out) {
        java.io.InputStream in = null;
        try {
            in = new java.io.FileInputStream(f);
            byte[] buf = new byte[32];
            int n = in.read(buf);
            if (n <= 0) {
                return 0;
            }
            int found = 0;
            int i = 0;
            while (i < n && found < out.length) {
                while (i < n && (buf[i] < (byte) '0' || buf[i] > (byte) '9')) {
                    ++i;
                }
                if (i >= n) {
                    break;
                }
                int v = 0;
                int digits = 0;
                while (i < n && buf[i] >= (byte) '0' && buf[i] <= (byte) '9') {
                    v = v * 10 + (buf[i] - (byte) '0');
                    if (++digits > 6) {
                        return found;       // implausible, keep what parsed
                    }
                    ++i;
                }
                out[found++] = v;
            }
            return found;
        } catch (Throwable t) {
            return 0;
        } finally {
            if (in != null) {
                try {
                    in.close();
                } catch (Throwable t) {
                    // nothing useful to do on a failed close
                }
            }
        }
    }

    /**
     * Replaces `this.repaint(); this.serviceRepaints();` in the canvas loop.
     * Called once per tick whether or not it paints, so it is also the tick
     * counter.
     */
    public static void doRepaint(Canvas canvas) {
        if (ctlPath != null) {
            pollControl(System.currentTimeMillis());
        }
        boolean painted = shouldPaint();
        if (painted) {
            canvas.repaint();
            canvas.serviceRepaints();
        }
        ++ticks;
        if (painted) {
            ++paints;
        }
        if (reportMs > 0L) {
            report();
        }
    }

    /**
     * Whether this tick paints.
     *
     * Guard: some vanilla logic lives inside the draw path, so paint cannot be
     * dropped unconditionally — not even at paintEvery 0, which is why the
     * guard is checked before the never-paint case. cf.i(bx) advances the
     * map-19/67 cutscene and ends it through n.b().c(); eq.a(bx) advances
     * character-select animation. Both run only from a draw call, so those
     * states keep painting regardless of paintEvery. Verified in
     * mod/src_decomp/cf.java:1858 and mod/src_decomp/eq.java:158.
     */
    public static boolean shouldPaint() {
        if (paintEvery == 1) {
            return true;                    // vanilla behaviour
        }
        if (guard && mustPaint()) {
            return true;
        }
        if (paintEvery == 0) {
            return false;                   // hidden tab: paint nothing
        }
        return ticks % (long) paintEvery == 0L;
    }

    private static boolean mustPaint() {
        try {
            if (fu.a != fu.c) {
                return true;    // any screen other than the game scene
            }
            if (fu.q != null && (fu.q.d == 19 || fu.q.d == 67)) {
                return true;    // cutscene advanced from cf.i(bx)
            }
        } catch (Throwable t) {
            return true;        // never trade a stall for a saved frame
        }
        return false;
    }

    private static void report() {
        long now = System.currentTimeMillis();
        if (reportAt == 0L) {
            reportAt = now;
            return;
        }
        long dt = now - reportAt;
        if (dt < reportMs) {
            return;
        }
        System.out.println("POTATO tps=" + ((ticks - ticksAtReport) * 1000L / dt)
                + " fps=" + ((paints - paintsAtReport) * 1000L / dt)
                + " draws/s=" + ((draws - drawsAtReport) * 1000L / dt)
                + " paintEvery=" + paintEvery
                + " guard=" + (guard ? 1 : 0)
                + " ctl=" + ctlApplied
                + " mask=" + layerMask
                + " skips/s=" + ((layerSkips - layerSkipsAtReport) * 1000L / dt)
                + " screen=" + screenId());
        reportAt = now;
        ticksAtReport = ticks;
        paintsAtReport = paints;
        drawsAtReport = draws;
        layerSkipsAtReport = layerSkips;
    }

    /** Coarse screen label, so the report is readable with no display. */
    private static String screenId() {
        try {
            if (fu.a == fu.c) {
                return "game";
            }
            if (fu.a == fu.b) {
                return "login";
            }
            return "other";
        } catch (Throwable t) {
            return "?";
        }
    }
}
