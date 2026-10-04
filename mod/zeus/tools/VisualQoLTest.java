import java.lang.reflect.Field;
import java.lang.reflect.Method;

public class VisualQoLTest {

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

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

    static boolean parseControl(String body) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod("parseControl", String.class);
        m.setAccessible(true);
        return ((Boolean) m.invoke(null, body)).booleanValue();
    }

    static String buildPayload(int version, int effects, int hidePlayers, String extraKey) {
        StringBuilder sb = new StringBuilder();
        sb.append("v=").append(version).append("\n");
        sb.append("atk.mode=0\n");
        sb.append("atk.map=0\n");
        sb.append("atk.zone=-1\n");
        sb.append("atk.x=-1\n");
        sb.append("atk.y=-1\n");
        sb.append("atk.radius=100\n");
        sb.append("atk.hpOn=0\n");
        sb.append("atk.hpPct=50\n");
        sb.append("atk.mpOn=0\n");
        sb.append("atk.mpPct=50\n");
        sb.append("revive.mode=1\n");
        sb.append("atk.buffs=000\n");
        sb.append("atk.zoneMode=0\n");
        sb.append("atk.zonePick=1\n");
        sb.append("item.rank=5\n");
        sb.append("item.mphp=3\n");
        sb.append("item.gold=1\n");
        sb.append("mount.on=0\n");
        sb.append("mount.id=0\n");
        sb.append("item.medalDialog=0\n");
        sb.append("item.dropsOn=0\n");
        sb.append("item.drops=000000\n");
        sb.append("nav.target=-1\n");
        sb.append("ui.ring=0\n");
        sb.append("atk.farmOnArrival=0\n");
        sb.append("nav.detectSpots=0\n");
        sb.append("revive.delay=0\n");
        sb.append("revive.on=0\n");
        sb.append("enhance.on=0\n");
        sb.append("enhance.maxLv=10\n");
        sb.append("enhance.charm=0\n");
        sb.append("dungeon.on=0\n");
        sb.append("dungeon.max=-1\n");
        sb.append("dungeon.startMin=-1\n");
        sb.append("dungeon.endMin=-1\n");
        if (effects >= 0) {
            sb.append("ui.effects=").append(effects).append("\n");
        }
        if (hidePlayers >= 0) {
            sb.append("ui.hidePlayers=").append(hidePlayers).append("\n");
        }
        if (extraKey != null) {
            sb.append(extraKey).append("\n");
        }
        return sb.toString();
    }

    public static void main(String[] args) throws Exception {
        System.out.println("=== VisualQoLTest ===");

        // Test 1: Canonical Control v15 with exactly 38 keys parses and succeeds
        System.out.println("--- Test 1: Canonical Control v15 with 38 keys ---");
        String v15Valid = buildPayload(15, 1, 0, null);
        boolean ok1 = parseControl(v15Valid);
        check("Control v15 accepts exactly 38 valid keys", ok1);
        check("ui.effects=1 maps to fa.ch=0", fa.ch == 0);
        check("ui.hidePlayers=0 maps to cn.aN=false", !cn.aN);
        check("ui.hidePlayers=0 maps to cn.aO=false", !cn.aO);

        // Test 2: ui.effects = 0 maps to fa.ch = 1
        System.out.println("--- Test 2: ui.effects=0 ---");
        String v15Effects0 = buildPayload(15, 0, 0, null);
        boolean ok2 = parseControl(v15Effects0);
        check("parseControl succeeds for ui.effects=0", ok2);
        check("ui.effects=0 maps to fa.ch=1", fa.ch == 1);

        // Test 3: ui.effects = 1 maps to fa.ch = 0
        System.out.println("--- Test 3: ui.effects=1 ---");
        String v15Effects1 = buildPayload(15, 1, 0, null);
        boolean ok3 = parseControl(v15Effects1);
        check("parseControl succeeds for ui.effects=1", ok3);
        check("ui.effects=1 maps to fa.ch=0", fa.ch == 0);

        // Test 4: ui.hidePlayers = 1 maps to cn.aN=true, cn.aO=false
        System.out.println("--- Test 4: ui.hidePlayers=1 ---");
        String v15Hide1 = buildPayload(15, 1, 1, null);
        boolean ok4 = parseControl(v15Hide1);
        check("parseControl succeeds for ui.hidePlayers=1", ok4);
        check("ui.hidePlayers=1 maps to cn.aN=true", cn.aN);
        check("ui.hidePlayers=1 maps to cn.aO=false", !cn.aO);
        check("no true/true state in mode 1", !(cn.aN && cn.aO));

        // Test 5: ui.hidePlayers = 2 maps to cn.aN=false, cn.aO=true
        System.out.println("--- Test 5: ui.hidePlayers=2 ---");
        String v15Hide2 = buildPayload(15, 1, 2, null);
        boolean ok5 = parseControl(v15Hide2);
        check("parseControl succeeds for ui.hidePlayers=2", ok5);
        check("ui.hidePlayers=2 maps to cn.aN=false", !cn.aN);
        check("ui.hidePlayers=2 maps to cn.aO=true", cn.aO);
        check("no true/true state in mode 2", !(cn.aN && cn.aO));

        // Test 6: Direct transition from mode 1 to mode 2 never produces true/true
        System.out.println("--- Test 6: Direct transition between hide modes ---");
        parseControl(buildPayload(15, 1, 1, null));
        check("Pre-condition mode 1 cn.aN=true", cn.aN && !cn.aO);
        parseControl(buildPayload(15, 1, 2, null));
        check("Post-transition mode 2 cn.aN=false", !cn.aN && cn.aO);
        check("Never true/true", !(cn.aN && cn.aO));

        // Test 7: Invalid ui.effects value is rejected (fails closed)
        System.out.println("--- Test 7: Invalid ui.effects ---");
        String v15InvalidEffects = buildPayload(15, 2, 0, null);
        boolean ok7 = parseControl(v15InvalidEffects);
        check("ui.effects=2 is rejected", !ok7);

        // Test 8: Invalid ui.hidePlayers value is rejected (fails closed)
        System.out.println("--- Test 8: Invalid ui.hidePlayers ---");
        String v15InvalidHide = buildPayload(15, 1, 3, null);
        boolean ok8 = parseControl(v15InvalidHide);
        check("ui.hidePlayers=3 is rejected", !ok8);

        // Test 9: Unknown key fails closed
        System.out.println("--- Test 9: Unknown key ---");
        String v15Unknown = buildPayload(15, 1, 0, "unknown.key=1");
        boolean ok9 = parseControl(v15Unknown);
        check("Unknown key is rejected", !ok9);

        // Test 10: Stale v14 payload fails closed on v15 Zeus
        System.out.println("--- Test 10: Stale v14 payload fails closed ---");
        String v14Payload = buildPayload(14, 1, 0, null);
        boolean ok10 = parseControl(v14Payload);
        check("Stale v14 payload is rejected by v15 Zeus", !ok10);

        // Test 11: Reconnect / lifecycle reconciliation restores desired state
        System.out.println("--- Test 11: Lifecycle reconciliation preserves desired visual state ---");
        boolean ok11 = parseControl(buildPayload(15, 0, 2, null));
        check("parseControl succeeds for Test 11", ok11);
        check("Desired state set: fa.ch=1, cn.aO=true", fa.ch == 1 && cn.aO);
        // Simulate client reset on screen change or reconnect
        fa.ch = 0;
        cn.aO = false;
        check("Client simulated reset: fa.ch=0, cn.aO=false", fa.ch == 0 && !cn.aO);
        // Call reconcileVisualQoL (as sessionTick does)
        call("reconcileVisualQoL");
        check("Lifecycle reconciliation restores fa.ch=1", fa.ch == 1);
        check("Lifecycle reconciliation restores cn.aO=true", cn.aO);
        check("Never true/true", !(cn.aN && cn.aO));

        // Test 12: allOff resets visual QoL to neutral defaults
        System.out.println("--- Test 12: allOff resets to neutral defaults ---");
        call("allOff");
        check("allOff restores fa.ch=0 (effects enabled default)", fa.ch == 0);
        check("allOff restores cn.aN=false, cn.aO=false (show all default)", !cn.aN && !cn.aO);

        System.out.println("\nTotal failures: " + failures);
        if (failures > 0) {
            System.exit(1);
        }
    }
}
