import java.lang.reflect.Field;
import java.lang.reflect.Method;

public class ReconnectTravelTest {

    static Field f(Class<?> owner, String name) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    static void set(String name, Object value) throws Exception {
        f(Class.forName("Zeus"), name).set(null, value);
    }

    static Object get(String name) throws Exception {
        return f(Class.forName("Zeus"), name).get(null);
    }

    static void call(String name) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod(name);
        m.setAccessible(true);
        m.invoke(null);
    }

    static boolean boolCall(String name) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod(name);
        m.setAccessible(true);
        return ((Boolean) m.invoke(null)).booleanValue();
    }

    static int intCall(String name) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod(name);
        m.setAccessible(true);
        return ((Integer) m.invoke(null)).intValue();
    }

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static void setupWorldState(int mapId) {
        if (fu.c == null) {
            fu.c = new cn();
        }
        fu.a = fu.c;
        eh.h = true;
        if (fu.q == null) {
            try {
                Field uf = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
                uf.setAccessible(true);
                sun.misc.Unsafe unsafe = (sun.misc.Unsafe) uf.get(null);
                fu.q = (cs) unsafe.allocateInstance(cs.class);
            } catch (Throwable t) {
            }
        }
        if (fu.q != null) {
            fu.q.d = mapId;
        }
        cs.i = 10;
        cs.j = 20;
        if (cn.g == null) {
            cn.g = new bq(100, (byte) 0, "hero", 0, 0);
        }
        cn.g.cG = (byte) 0; // alive
        cn.g.cx = 100;
        cn.g.cy = 100;
        cn.g.aZ = 100;
        cn.g.ba = 100;
        cn.i = null;
        fu.s = null;
        fu.t = null;
        if (fu.p != null) {
            fu.p.a = false;
        }
    }

    public static void main(String[] args) throws Exception {
        setupWorldState(1);

        // =====================================================================
        // Test 1: same_map_reconnect_reset
        // =====================================================================
        System.out.println("--- Test 1: same_map_reconnect_reset ---");
        setupWorldState(1);
        bq.m = true;
        cn.g.cO = new short[] { 10, 20 };
        set("travelMapSeen", Integer.valueOf(1));
        set("travelHops", Integer.valueOf(5));

        // Transition to char-select screen
        if (fu.i == null) {
            fu.i = new x();
        }
        fu.a = fu.i;
        call("sessionTick");

        // Transition back to world screen on same map (1)
        fu.a = fu.c;
        call("sessionTick");

        check("movement lock bq.m cleared", !bq.m);
        check("path buffer cn.g.cO cleared", cn.g.cO == null);
        check("travelMapSeen reset to sentinel", ((Integer) get("travelMapSeen")).intValue() == Integer.MIN_VALUE);
        check("travelHops reset on new session", ((Integer) get("travelHops")).intValue() == 0);

        // Tick to settle on same map (1)
        for (int i = 0; i < 15; i++) {
            call("sessionTick");
        }
        check("mapStable() becomes true on same map", boolCall("mapStable"));

        // Travel should now reinitialize travelMapSeen to 1 without skipping
        set("atkMode", Integer.valueOf(1));
        set("atkMap", Integer.valueOf(1)); // already at destination
        set("atkX", Integer.valueOf(100));
        set("atkY", Integer.valueOf(100));
        call("travel");
        check("travelMapSeen updated to current map", ((Integer) get("travelMapSeen")).intValue() == 1);

        // =====================================================================
        // Test 2: cross_map_reconnect_reset
        // =====================================================================
        System.out.println("--- Test 2: cross_map_reconnect_reset ---");
        setupWorldState(5);
        set("travelHops", Integer.valueOf(18));
        set("travelStoneTried", Integer.valueOf(2));
        set("travelStallTicks", Integer.valueOf(40));
        set("atkMode", Integer.valueOf(1));
        set("atkMap", Integer.valueOf(20));
        set("atkX", Integer.valueOf(500));
        set("atkY", Integer.valueOf(600));
        set("navTarget", Integer.valueOf(8));
        set("navDone", Boolean.TRUE);

        // Reconnect into map 10
        fu.a = fu.i;
        call("sessionTick");
        setupWorldState(10);
        fu.a = fu.c;
        call("sessionTick");

        check("cross-map travelHops reset to 0", ((Integer) get("travelHops")).intValue() == 0);
        check("cross-map travelStoneTried reset to 0", ((Integer) get("travelStoneTried")).intValue() == 0);
        check("cross-map travelStallTicks reset to 0", ((Integer) get("travelStallTicks")).intValue() == 0);
        check("durable atkMode preserved", ((Integer) get("atkMode")).intValue() == 1);
        check("durable atkMap preserved", ((Integer) get("atkMap")).intValue() == 20);
        check("durable atkX preserved", ((Integer) get("atkX")).intValue() == 500);
        check("durable atkY preserved", ((Integer) get("atkY")).intValue() == 600);
        check("durable navDone preserved", ((Boolean) get("navDone")).booleanValue());
        check("goal() returns Auto Farm destination (20)", intCall("goal") == 20);

        // =====================================================================
        // Test 3: map_stability
        // =====================================================================
        System.out.println("--- Test 3: map_stability ---");
        setupWorldState(7);
        call("sessionReset");

        // Tick 5 times on map 7
        for (int i = 0; i < 5; i++) {
            call("sessionTick");
        }
        check("mapStable() false after only 5 ticks", !boolCall("mapStable"));

        // Switch map to 8 on next tick
        setupWorldState(8);
        call("sessionTick");
        check("map change resets mapStable()", !boolCall("mapStable"));

        // Tick 8 more times on map 8 (total 9 ticks on map 8)
        for (int i = 0; i < 8; i++) {
            call("sessionTick");
        }
        check("mapStable() still false after 9 ticks", !boolCall("mapStable"));

        // 10th tick on map 8
        call("sessionTick");
        check("mapStable() true after 10 consecutive ticks on same map", boolCall("mapStable"));

        // =====================================================================
        // Test 4: portal_transition
        // =====================================================================
        System.out.println("--- Test 4: portal_transition ---");
        // Start stable on map 8
        check("initially stable on map 8", boolCall("mapStable"));

        // Portal transition: scene becomes not ready or map changes
        cs.i = cs.j; // scene not ready
        call("sessionTick");
        check("portal transition (scene loading) resets mapStable()", !boolCall("mapStable"));

        // New map loaded: map 9, scene ready
        setupWorldState(9);
        call("sessionTick");
        check("new map tick 1 not stable", !boolCall("mapStable"));

        for (int i = 0; i < 9; i++) {
            call("sessionTick");
        }
        check("new map settled (10 ticks) becomes mapStable()", boolCall("mapStable"));

        // =====================================================================
        // Test 5: character_select_retry_success
        // =====================================================================
        System.out.println("--- Test 5: character_select_retry_success ---");
        call("sessionReset");
        if (fu.i == null) {
            fu.i = new x();
        }
        if (x.a == null) {
            x.a = new et("chars");
        }
        if (x.a.c() == 0) {
            x.a.a(new bm(1, (byte) 0, "hero", 0, 0));
        }
        fu.a = fu.i;
        ah.k = false;
        set("armed", Boolean.FALSE);

        // Initial entry to char-select
        call("auth");
        check("initial auth submit sets ah.k = true", ah.k);
        check("initial auth arms armed = true", ((Boolean) get("armed")).booleanValue());

        // Simulate client processing the submit flag but remaining on char-select
        ah.k = false;

        // Ticking fewer than retry interval should NOT re-submit
        for (int i = 0; i < 30; i++) {
            call("auth");
        }
        check("no rapid duplicate submit during interval", !ah.k);

        // Tick past interval (threshold = 75 ticks)
        for (int i = 0; i < 50; i++) {
            call("auth");
        }
        check("retry submit triggered after interval", ah.k);

        // Leaving char select resets retry state
        ah.k = false;
        fu.a = fu.c;
        call("auth");
        check("leaving char-select disarms armed", !((Boolean) get("armed")).booleanValue());

        // =====================================================================
        // Test 6: character_select_retry_exhaustion
        // =====================================================================
        System.out.println("--- Test 6: character_select_retry_exhaustion ---");
        fu.a = fu.i;
        ah.k = false;

        // Attempt 1
        call("auth");
        check("attempt 1 submitted", ah.k);
        ah.k = false;

        // Wait interval -> Attempt 2
        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("attempt 2 submitted", ah.k);
        ah.k = false;

        // Wait interval -> Attempt 3
        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("attempt 3 submitted", ah.k);
        ah.k = false;

        // Wait further interval -> Exhausted! No attempt 4
        for (int i = 0; i < 150; i++) {
            call("auth");
        }
        check("after 3 attempts, retries exhausted and ah.k NOT set", !ah.k);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
