/*
 * Prove the injected layer gate actually short-circuits the method body.
 *
 * Why this test exists: ey.a(bx) and br.a(bx) are only reached from cn.a(bx),
 * the in-game scene render. At the login screen there is no game scene, so a
 * benchmark reports skips/s=0 no matter whether the gate works — it is never
 * called. javap proves the prologue is present and the verifier proves it is
 * well-formed, but neither proves it returns early.
 *
 * This class sits in the default package and compiles against the jar, so it can
 * name ey, br, bx and dy directly.
 *
 * Each layer needs its own way of making "the body ran" observable:
 *
 *   ey  a bx whose Graphics field is null. The body's first act is to draw, so
 *       ungated it raises NullPointerException and gated it returns silently.
 *
 *   br  an empty effect list makes the body a no-op loop, which is why a plain
 *       `new br()` cannot distinguish the two. A dy subclass is added to the
 *       list whose a(bx) records the call, so "the body ran" becomes "the
 *       element was dispatched to" rather than "something threw".
 */
public final class GateTest {

    public static void main(String[] args) {
        int failures = 0;
        failures += checkEy();
        failures += checkBr();
        System.out.println(failures == 0 ? "GATE TEST PASS" : "GATE TEST FAIL");
        if (failures != 0) {
            System.exit(1);
        }
    }

    /** dy whose draw is observable, so br's dispatch can be detected. */
    static final class Probe extends dy {
        boolean drawn = false;

        public void a(bx bx2) {
            this.drawn = true;
        }
    }

    private static int checkEy() {
        long before = POTATO.layerSkips;

        POTATO.layerMask = 0;
        Throwable ungated = callEy();
        long afterUngated = POTATO.layerSkips;

        POTATO.layerMask = 1;
        Throwable gated = callEy();
        long afterGated = POTATO.layerSkips;

        boolean ran = ungated != null;
        boolean skipped = gated == null;
        boolean counted = afterUngated == before && afterGated == before + 1L;

        System.out.println("ey.a(bx) bit=1"
                + "  ungated=" + (ran ? "ran (" + ungated.getClass().getName() + ")" : "DID NOT RUN")
                + "  gated=" + (skipped ? "skipped" : "RAN")
                + "  counter=" + (counted ? "ok" : "WRONG"));
        return (ran && skipped && counted) ? 0 : 1;
    }

    /** Calls ey.a(bx) with a null-Graphics bx; returns what it threw, or null. */
    private static Throwable callEy() {
        try {
            new ey().a(new bx());
            return null;
        } catch (Throwable t) {
            return t;
        }
    }

    private static int checkBr() {
        long before = POTATO.layerSkips;

        POTATO.layerMask = 0;
        boolean dispatchedUngated = callBr();
        long afterUngated = POTATO.layerSkips;

        POTATO.layerMask = 2;
        boolean dispatchedGated = callBr();
        long afterGated = POTATO.layerSkips;

        boolean counted = afterUngated == before && afterGated == before + 1L;

        System.out.println("br.a(bx) bit=2"
                + "  ungated=" + (dispatchedUngated ? "dispatched" : "DID NOT DISPATCH")
                + "  gated=" + (!dispatchedGated ? "skipped" : "DISPATCHED")
                + "  counter=" + (counted ? "ok" : "WRONG"));
        return (dispatchedUngated && !dispatchedGated && counted) ? 0 : 1;
    }

    /** Runs br.a(bx) over a one-element list; true if the element was drawn. */
    private static boolean callBr() {
        try {
            br b = new br();
            Probe p = new Probe();
            b.a((Object) p);            // et.a(Object): append to the list
            b.a(new bx());              // br.a(bx): the gated draw
            return p.drawn;
        } catch (Throwable t) {
            System.out.println("   br probe threw: " + t);
            return false;
        }
    }
}
