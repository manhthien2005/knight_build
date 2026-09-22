import java.lang.reflect.Field;
import java.lang.reflect.Method;

public class CharacterSlotTest {

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

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static et makeCharList(int count) {
        et list = new et("TestChars");
        for (int i = 0; i < count; i++) {
            list.a(new bm(i + 1, (byte) 0, "char" + i, 0, 0));
        }
        return list;
    }

    public static void main(String[] args) throws Exception {
        if (fu.i == null) {
            fu.i = new x();
        }
        fu.a = fu.i;

        // ---------------------------------------------------------------------
        // Test 1: Internal slot 0 selects first character when >= 1 exists
        // ---------------------------------------------------------------------
        System.out.println("--- Test 1: slot 0 selects first character ---");
        System.setProperty("zeus.auth.slot", "0");
        x.a = makeCharList(1);
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("slot 0 sets ah.k=true", ah.k);
        check("slot 0 sets fu.i.k=0", fu.i.k == 0);

        // ---------------------------------------------------------------------
        // Test 2: Internal slot 1 selects second character when >= 2 exist
        // ---------------------------------------------------------------------
        System.out.println("--- Test 2: slot 1 selects second character ---");
        System.setProperty("zeus.auth.slot", "1");
        x.a = makeCharList(2);
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("slot 1 sets ah.k=true", ah.k);
        check("slot 1 sets fu.i.k=1", fu.i.k == 1);

        // ---------------------------------------------------------------------
        // Test 3: Internal slot 2 selects third character when 3 exist
        // ---------------------------------------------------------------------
        System.out.println("--- Test 3: slot 2 selects third character ---");
        System.setProperty("zeus.auth.slot", "2");
        x.a = makeCharList(3);
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("slot 2 sets ah.k=true", ah.k);
        check("slot 2 sets fu.i.k=2", fu.i.k == 2);

        // ---------------------------------------------------------------------
        // Test 4: Requested slot >= character count refuses world entry (no fallback to Slot 1)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 4: requested slot >= count refuses world entry ---");
        System.setProperty("zeus.auth.slot", "1");
        x.a = makeCharList(1); // count=1, slot 1 is empty/unavailable
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("unavailable slot 1 refuses ah.k", !ah.k);
        check("no fallback to slot 0 in fu.i.k", fu.i.k != 0);

        System.setProperty("zeus.auth.slot", "2");
        x.a = makeCharList(2); // count=2, slot 2 is empty/unavailable
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("unavailable slot 2 refuses ah.k", !ah.k);
        check("no fallback to slot 0 in fu.i.k", fu.i.k != 0);

        // ---------------------------------------------------------------------
        // Test 5: Null character object at requested position refuses world entry
        // ---------------------------------------------------------------------
        System.out.println("--- Test 5: null character object at target index refuses ---");
        System.setProperty("zeus.auth.slot", "0");
        et listWithNull = new et("TestNull");
        listWithNull.a(null);
        x.a = listWithNull;
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("null character object refuses ah.k", !ah.k);

        // ---------------------------------------------------------------------
        // Test 6: Malformed / out-of-range JVM property fails closed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 6: malformed / out-of-range property fails closed ---");
        x.a = makeCharList(3);

        System.setProperty("zeus.auth.slot", "3"); // outside 0..2
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("slot 3 fails closed (ah.k is false)", !ah.k);

        System.setProperty("zeus.auth.slot", "-1"); // negative
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("slot -1 fails closed (ah.k is false)", !ah.k);

        System.setProperty("zeus.auth.slot", "bad"); // non-numeric
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("slot bad fails closed (ah.k is false)", !ah.k);

        // ---------------------------------------------------------------------
        // Test 7: Absent property preserves legacy internal default 0
        // ---------------------------------------------------------------------
        System.out.println("--- Test 7: absent property preserves legacy internal default 0 ---");
        System.clearProperty("zeus.auth.slot");
        x.a = makeCharList(1);
        fu.i.k = -1;
        ah.k = false;
        call("authReset");
        call("auth");
        check("absent property selects slot 0", ah.k && fu.i.k == 0);

        // ---------------------------------------------------------------------
        // Test 8: Valid-slot bounded auth retry repeats identical slot
        // ---------------------------------------------------------------------
        System.out.println("--- Test 8: bounded auth retry repeats identical slot ---");
        System.setProperty("zeus.auth.slot", "1");
        x.a = makeCharList(2);
        call("authReset");
        ah.k = false;
        call("auth");
        check("initial attempt selects slot 1", ah.k && fu.i.k == 1);
        ah.k = false;

        // Tick past retry interval (75 ticks)
        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("retry attempt 2 re-submits slot 1", ah.k && fu.i.k == 1);
        ah.k = false;

        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("retry attempt 3 re-submits slot 1", ah.k && fu.i.k == 1);
        ah.k = false;

        for (int i = 0; i < 150; i++) {
            call("auth");
        }
        check("exhausted retries stops submitting", !ah.k);

        // ---------------------------------------------------------------------
        // Test 9: Unavailable slot does not generate repeated auth submissions
        // ---------------------------------------------------------------------
        System.out.println("--- Test 9: unavailable slot does not consume retry submissions ---");
        System.setProperty("zeus.auth.slot", "2");
        x.a = makeCharList(1); // slot 2 unavailable
        call("authReset");
        ah.k = false;
        call("auth");
        check("initial tick does not submit unavailable slot", !ah.k);

        for (int i = 0; i < 200; i++) {
            call("auth");
        }
        check("unavailable slot never submits after 200 ticks", !ah.k);
        check("unavailable slot does not exhaust authAttempts", ((Integer) get("authAttempts")).intValue() == 0);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
