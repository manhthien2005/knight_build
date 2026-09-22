import java.lang.reflect.Field;
import java.lang.reflect.Method;

public class DialogGateTest {

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

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static class TestTarget extends cg {
        boolean pressed = false;
        int pressCount = 0;
        boolean dismissOnPress = true;

        public void a(int id, int h) {
            pressed = true;
            pressCount++;
            if (dismissOnPress) {
                fu.s = null;
            }
        }
    }

    static ah makeDialog(String text, TestTarget target, String caption) {
        ah dialog = new ah();
        dialog.q = new String[] { text };
        et list = new et("buttons");
        bt button = new bt(caption, 1, target);
        list.a(button);
        dialog.C = list;
        return dialog;
    }

    static ah makeTwoButtonDialog(String text, TestTarget target1, String cap1, TestTarget target2, String cap2) {
        ah dialog = new ah();
        dialog.q = new String[] { text };
        et list = new et("buttons");
        bt b1 = new bt(cap1, 1, target1);
        bt b2 = new bt(cap2, 2, target2);
        list.a(b1);
        list.a(b2);
        dialog.C = list;
        return dialog;
    }

    static void setupWorldState() {
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
                // fallback
            }
        }
        if (fu.q != null) {
            fu.q.d = 1; // map id 1
        }
        cs.i = 10;
        cs.j = 20; // cs.i != cs.j
        if (cn.g == null) {
            cn.g = new bq(100, (byte) 0, "hero", 0, 0);
        }
        cn.g.cG = (byte) 0; // alive
        cn.g.cx = 100;
        cn.g.cy = 100;
        cn.i = null; // no captcha
        fu.s = null;
        fu.t = null;
        if (fu.p != null) {
            fu.p.a = false;
        }
    }

    public static void main(String[] args) throws Exception {
        setupWorldState();

        // =====================================================================
        // Test 1: Harmless informational dialog dismissed without ready() true
        // =====================================================================
        System.out.println("--- Test 1: Safe Informational Dialog Dismissal ---");
        TestTarget targetSafe = new TestTarget();
        ah safeDialog = makeDialog("Thong bao tu server: Bao tri hoan tat.", targetSafe, "Đóng");
        fu.s = safeDialog;

        // Verify ready() is false initially because fu.s != null
        check("ready() is false while dialog open", !boolCall("ready"));
        check("gameReady() is false while dialog open", !boolCall("gameReady"));

        // Tick through debounce
        for (int i = 0; i < 5; i++) {
            call("tick");
        }
        check("safe dialog button pressed", targetSafe.pressed);
        check("safe dialog dismissed (fu.s == null)", fu.s == null);

        // =====================================================================
        // Test 2: Unknown dialog fails closed (untouched, automation blocked)
        // =====================================================================
        System.out.println("--- Test 2: Unknown Dialog Fails Closed ---");
        TestTarget targetUnknown = new TestTarget();
        ah unknownDialog = makeDialog("Nap the nhan khuyen mai 500% cuc hot", targetUnknown, "Đóng");
        fu.s = unknownDialog;

        for (int i = 0; i < 10; i++) {
            call("tick");
        }
        check("unknown dialog NOT pressed", !targetUnknown.pressed);
        check("unknown dialog remains open", fu.s == unknownDialog);
        check("ready() remains false", !boolCall("ready"));
        check("gameReady() remains false", !boolCall("gameReady"));
        fu.s = null; // clear for next test

        // =====================================================================
        // Test 3: Dangerous/confirmation dialog never auto-confirmed
        // =====================================================================
        System.out.println("--- Test 3: Dangerous Dialog Safety ---");
        TestTarget targetYes = new TestTarget();
        TestTarget targetNo = new TestTarget();
        ah deleteDialog = makeTwoButtonDialog("Ban co muon xoa nhan vat nay khong?", targetYes, "Đồng ý", targetNo, "Không");
        fu.s = deleteDialog;

        for (int i = 0; i < 10; i++) {
            call("tick");
        }
        check("dangerous 2-button dialog NOT pressed", !targetYes.pressed && !targetNo.pressed);
        check("dangerous dialog remains open", fu.s == deleteDialog);
        check("gameReady() blocked by dangerous dialog", !boolCall("gameReady"));
        fu.s = null;

        // Single button with "Đồng ý" caption should also NEVER be confirmed
        TestTarget targetDongY = new TestTarget();
        ah confirmSingle = makeDialog("Xac nhan mua vat pham voi gia 1000 ngoc?", targetDongY, "Đồng ý");
        fu.s = confirmSingle;
        for (int i = 0; i < 10; i++) {
            call("tick");
        }
        check("single-button Dong Y dialog NOT confirmed", !targetDongY.pressed);
        check("single-button Dong Y dialog remains open", fu.s == confirmSingle);
        fu.s = null;

        // =====================================================================
        // Test 4: Bounded retries on stubborn dialog
        // =====================================================================
        System.out.println("--- Test 4: Bounded Retries ---");
        TestTarget targetStubborn = new TestTarget();
        targetStubborn.dismissOnPress = false; // Dialog refuses to close
        ah stubbornDialog = makeDialog("Thong bao: Su kien dua top.", targetStubborn, "OK");
        fu.s = stubbornDialog;

        for (int i = 0; i < 20; i++) {
            call("tick");
        }
        check("stubborn dialog retry count is bounded (<= 3)", targetStubborn.pressCount <= 3);
        check("stubborn dialog still blocks gameReady()", !boolCall("gameReady"));
        fu.s = null;

        // =====================================================================
        // Test 5: Internal readiness during login/char-select/world loading
        // =====================================================================
        System.out.println("--- Test 5: Internal Readiness Login/Loading ---");
        if (fu.i == null) {
            fu.i = new x();
        }
        fu.a = fu.i; // char select
        call("tick");
        check("gameReady() false on char-select", !boolCall("gameReady"));

        fu.a = null; // uninitialized
        check("gameReady() false when fu.a is null", !boolCall("gameReady"));

        // =====================================================================
        // Test 6: Internal readiness settles in world and becomes true
        // =====================================================================
        System.out.println("--- Test 6: Settling to Game Ready ---");
        setupWorldState();
        // Reset session state
        call("sessionReset");
        check("gameReady() false immediately after session reset", !boolCall("gameReady"));

        // Tick through settle period (10 ticks)
        for (int i = 0; i < 12; i++) {
            call("tick");
        }
        check("gameReady() becomes true after settling without dialog", boolCall("gameReady"));

        // =====================================================================
        // Test 7: Auto Farm and Travel regression checks
        // =====================================================================
        System.out.println("--- Test 7: Auto Farm and Travel Semantics ---");
        set("atkMode", Integer.valueOf(1)); // Stand
        set("atkMap", Integer.valueOf(5));
        set("atkX", Integer.valueOf(120));
        set("atkY", Integer.valueOf(240));
        set("navTarget", Integer.valueOf(8));
        Method goalMethod = Class.forName("Zeus").getDeclaredMethod("goal");
        goalMethod.setAccessible(true);
        check("goal() returns Auto Farm map (5) when atkMode=1",
                ((Integer) goalMethod.invoke(null)).intValue() == 5);

        set("atkMode", Integer.valueOf(0)); // Off
        check("goal() returns navTarget (8) when atkMode=0",
                ((Integer) goalMethod.invoke(null)).intValue() == 8);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
