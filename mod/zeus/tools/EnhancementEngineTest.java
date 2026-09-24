import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.Vector;

/**
 * Focused test suite for Safe Single-Item Enhancement Runtime Engine (ENHANCE-04).
 * Tests all locked contracts:
 *  - Target safety (wire identity, count, fingerprint, level)
 *  - Auto charm truth table (levels 0..14)
 *  - Payment preflight (Gold vs Gem mode)
 *  - Execute semantics (sent at most once, bounded attempts)
 *  - Result classification (success, protected failure, degraded failure, destroyed, ambiguous)
 *  - Authoritative before/after accounting
 *  - Highlighting logic
 */
public class EnhancementEngineTest {

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

    @SuppressWarnings("unchecked")
    static Vector<Object> queue() throws Exception {
        Object link = l.a();
        Field o = f(l.class, "o");
        Object sender = o.get(link);
        Field a = f(sender.getClass(), "a");
        return (Vector<Object>) a.get(sender);
    }

    static void clearQueue() throws Exception {
        queue().removeAllElements();
    }

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static j makeItem(int id, int category, String name, String baseName, int level, int tier) {
        j it = new j();
        it.O = id;
        it.u = category;
        it.g = name;
        it.i = baseName;
        it.z = (byte) level;
        it.N = tier;
        it.K = 1;
        it.v = (short) 100;
        it.B = 1;
        it.t = 10;
        return it;
    }

    static j makeCharm(int id, String name, String baseName, int charmType) {
        j it = new j();
        it.O = id;
        it.u = 7; // category 7
        it.A = 11; // charm family
        it.g = name;
        it.i = baseName;
        it.z = 0;
        it.N = 0;
        it.K = 10;
        return it;
    }

    static et bag(bw... items) {
        et v = new et("bag");
        for (int i = 0; i < items.length; i++) {
            v.a(items[i]);
        }
        return v;
    }

    public static void main(String[] args) throws Exception {
        System.out.println("=== EnhancementEngineTest ===");

        // Precondition setup
        clearQueue();
        if (cn.g == null) {
            cn.g = new bq(100, (byte) 0, "hero", 0, 0);
        }

        // ---------------------------------------------------------------------
        // Test 1: Auto Charm Truth Table
        // ---------------------------------------------------------------------
        System.out.println("--- Test 1: Auto charm truth table ---");
        Method autoCharmMethod = Class.forName("Zeus").getDeclaredMethod("resolveAutoCharm", int.class);
        autoCharmMethod.setAccessible(true);

        for (int lv = 0; lv <= 5; lv++) {
            int charm = ((Integer) autoCharmMethod.invoke(null, lv)).intValue();
            check("level " + lv + " resolves no charm (0)", charm == 0);
        }
        for (int lv = 6; lv <= 10; lv++) {
            int charm = ((Integer) autoCharmMethod.invoke(null, lv)).intValue();
            check("level " + lv + " resolves Co 3 la (1)", charm == 1);
        }
        for (int lv = 11; lv <= 14; lv++) {
            int charm = ((Integer) autoCharmMethod.invoke(null, lv)).intValue();
            check("level " + lv + " resolves Co 4 la (2)", charm == 2);
        }

        // ---------------------------------------------------------------------
        // Test 2: Target Validation & Wire Uniqueness Rule
        // ---------------------------------------------------------------------
        System.out.println("--- Test 2: Target safety & wire uniqueness ---");
        Method validateTargetMethod = Class.forName("Zeus").getDeclaredMethod("validateEnhancementTarget");
        validateTargetMethod.setAccessible(true);

        // Subtest 2.1: 0 matching items -> ITEM_MISSING_OR_CHANGED
        bw.V = bag(makeItem(102, 3, "Kiem khac", "Kiem khac", 0, 1));
        set("enhTemplateId", 101);
        set("enhCategory", 3);
        set("enhBaseName", "Kiem ngan");
        set("enhTier", 2);
        set("enhExpectedLevel", 5);
        set("enhState", 3); // VALIDATING_TARGET

        validateTargetMethod.invoke(null);
        int state = ((Integer) get("enhState")).intValue();
        check("0 matching O+u items enters ITEM_MISSING_OR_CHANGED (20)", state == 20);

        // Subtest 2.2: 2 matching items with same O+u -> AMBIGUOUS_WIRE_TARGET
        j swordA = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        j swordB = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        bw.V = bag(swordA, swordB);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("2 matching O+u items enters AMBIGUOUS_WIRE_TARGET (21)", state == 21);

        // Subtest 2.3: Fingerprint mismatch (different base_name) -> ITEM_MISSING_OR_CHANGED
        j swordWrongName = makeItem(101, 3, "Kiem dai +5", "Kiem dai", 5, 2);
        bw.V = bag(swordWrongName);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Fingerprint name mismatch enters ITEM_MISSING_OR_CHANGED", state == 20);

        // Subtest 2.4: Fingerprint mismatch (different tier) -> ITEM_MISSING_OR_CHANGED
        j swordWrongTier = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 1);
        bw.V = bag(swordWrongTier);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Fingerprint tier mismatch enters ITEM_MISSING_OR_CHANGED", state == 20);

        // Subtest 2.5: Level mismatch -> ITEM_MISSING_OR_CHANGED
        j swordWrongLevel = makeItem(101, 3, "Kiem ngan +4", "Kiem ngan", 4, 2);
        bw.V = bag(swordWrongLevel);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Current level mismatch enters ITEM_MISSING_OR_CHANGED", state == 20);

        // Subtest 2.6: Exactly 1 valid match -> proceeds to LOCATING_BLACKSMITH (4)
        j swordValid = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        bw.V = bag(swordValid);
        set("enhState", 3);

        validateTargetMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("1 exact match proceeds to LOCATING_BLACKSMITH (4)", state == 4);
        int activeSlot = ((Integer) get("enhActiveTargetSlot")).intValue();
        check("Active target slot recorded for highlight", activeSlot == 0);

        // ---------------------------------------------------------------------
        // Test 3: Charm Resolution
        // ---------------------------------------------------------------------
        System.out.println("--- Test 3: Charm resolution & safety ---");
        Method resolveCharmMethod = Class.forName("Zeus").getDeclaredMethod("resolveEnhancementCharm");
        resolveCharmMethod.setAccessible(true);

        // Subtest 3.1: Mode 0 requires no charm
        set("enhConfiguredCharmMode", 0);
        set("enhState", 8); // RESOLVING_CHARM
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        int resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Mode 0 proceeds to VERIFYING_RESOURCES with resolved_charm 0", state == 10 && resolvedCharm == 0);

        // Subtest 3.2: Mode 1 missing charm -> CHARM_MISSING (24)
        bw.V = bag(swordValid);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Mode 1 with missing charm enters CHARM_MISSING (24)", state == 24);

        // Subtest 3.3: Mode 1 with valid charm -> proceeds to INSERTING_CHARM (9)
        j charm3 = makeCharm(501, "Cỏ 3 lá", "Cỏ 3 lá", 1);
        bw.V = bag(swordValid, charm3);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        resolvedCharm = ((Integer) get("enhResolvedCharmMode")).intValue();
        check("Mode 1 with valid charm proceeds to INSERTING_CHARM (9)", state == 9 && resolvedCharm == 1);

        // Subtest 3.4: Ambiguous charm candidates (multiple templates for same semantic type) -> AMBIGUOUS_CHARM (22)
        j charm3_dup = makeCharm(502, "Cỏ 3 lá", "Cỏ 3 lá", 1);
        bw.V = bag(swordValid, charm3, charm3_dup);
        set("enhConfiguredCharmMode", 1);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Ambiguous charm templates enters AMBIGUOUS_CHARM (22)", state == 22);

        // Subtest 3.5: No silent fallback (Mode 2 when only Co 3 la is present) -> CHARM_MISSING
        bw.V = bag(swordValid, charm3);
        set("enhConfiguredCharmMode", 2);
        set("enhState", 8);
        resolveCharmMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Mode 2 cannot fall back to Co 3 la -> CHARM_MISSING (24)", state == 24);

        // ---------------------------------------------------------------------
        // Test 4: Payment Preflight
        // ---------------------------------------------------------------------
        System.out.println("--- Test 4: Payment preflight ---");
        Method verifyResMethod = Class.forName("Zeus").getDeclaredMethod("verifyEnhancementResources");
        verifyResMethod.setAccessible(true);

        // Setup mock forge cost structures
        c.k = new b[16];
        for (int i = 0; i < 16; i++) {
            c.k[i] = new b();
            c.k[i].c = 50000; // quoted gold
            c.k[i].d = 20;    // quoted gems
            c.k[i].e = new byte[]{2, 1, 0, 0}; // mandatory materials
        }
        c.q = new short[]{301, 302, 303, 304}; // material template IDs
        c.p = new int[]{5, 5, 5, 5}; // bag counts
        c.j = new String[]{"Da", "Sat", "Dong", "Vang"};
        c.l = swordValid;

        // Subtest 4.1: Gold mode with sufficient gold but 0 gems -> SUCCESS (READY_FOR_ATTEMPT)
        cn.g.bD = 100000; // gold
        cn.g.bC = 0;      // gems
        set("enhPaymentType", 0);
        set("enhResolvedCharmMode", 0);
        set("enhState", 10);
        bw.V = bag(swordValid, makeItem(301, 7, "Da", "Da", 0, 0), makeItem(302, 7, "Sat", "Sat", 0, 0));

        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 0 succeeds with 0 gems if gold is sufficient", state == 11);

        // Subtest 4.2: Gold mode with insufficient gold -> INSUFFICIENT_GOLD (25)
        cn.g.bD = 1000;
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 0 with low gold enters INSUFFICIENT_GOLD (25)", state == 25);

        // Subtest 4.3: Gem mode with sufficient gems but 0 gold -> SUCCESS (READY_FOR_ATTEMPT)
        cn.g.bD = 0;
        cn.g.bC = 50;
        set("enhPaymentType", 1);
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 1 succeeds with 0 gold if gems are sufficient", state == 11);

        // Subtest 4.4: Gem mode with insufficient gems -> INSUFFICIENT_GEMS (26)
        cn.g.bC = 5;
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Payment type 1 with low gems enters INSUFFICIENT_GEMS (26)", state == 26);

        // Subtest 4.5: Missing mandatory material -> INSUFFICIENT_MATERIALS (27)
        cn.g.bC = 50;
        c.p[0] = 0; // missing material 0
        set("enhState", 10);
        verifyResMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Missing mandatory material enters INSUFFICIENT_MATERIALS (27)", state == 27);
        c.p[0] = 5; // restore

        // ---------------------------------------------------------------------
        // Test 5: Execution Semantics & Attempt Limits
        // ---------------------------------------------------------------------
        System.out.println("--- Test 5: Execution semantics & attempt limits ---");
        Method executeAttemptMethod = Class.forName("Zeus").getDeclaredMethod("executeEnhancementAttempt");
        executeAttemptMethod.setAccessible(true);

        clearQueue();
        set("enhState", 11); // READY_FOR_ATTEMPT
        set("enhAttemptCount", 0);
        set("enhMaxAttempts", 2);
        set("enhPaymentType", 0);

        // Attempt 1: should send Opcode 67 sub-action 2 exactly once
        executeAttemptMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        int attempts = ((Integer) get("enhAttemptCount")).intValue();
        Vector<Object> q = queue();
        check("Execute packet increments attempt count to 1", attempts == 1);
        check("State becomes ATTEMPTING / WAITING_RESULT", state == 12 || state == 13);
        check("Opcode 67 emitted exactly once", q.size() == 1);
        ep pkt = (ep) q.elementAt(0);
        check("Emitted packet opcode is 67", pkt.a == 67);

        // Subtest 5.2: Exceeding max_attempts enters ATTEMPT_LIMIT_REACHED (18)
        set("enhState", 11);
        set("enhAttemptCount", 2);
        set("enhMaxAttempts", 2);
        executeAttemptMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Reaching max_attempts blocks execution and enters ATTEMPT_LIMIT_REACHED (18)", state == 18);

        // ---------------------------------------------------------------------
        // Test 6: Result Classification & Authoritative Accounting
        // ---------------------------------------------------------------------
        System.out.println("--- Test 6: Result classification & accounting ---");
        Method settleMethod = Class.forName("Zeus").getDeclaredMethod("settleEnhancementResult");
        settleMethod.setAccessible(true);

        // Pre-execute snapshot values
        set("snapGoldBefore", 100000L);
        set("snapGemBefore", 50L);
        set("snapMaterialsBefore", new long[]{5L, 5L, 0L, 0L});
        set("snapCharmBefore", 2L);
        set("snapTargetLevelBefore", 5);
        set("snapSelectedCharmTemplateId", 501);
        set("enhResolvedCharmMode", 1);
        set("enhTargetLevel", 7);
        set("enhAttemptCount", 1);
        set("enhMaxAttempts", 10);
        set("enhActualGoldSpent", 0L);
        set("enhActualGemSpent", 0L);
        set("enhActualMaterialsSpent", new long[]{0L, 0L, 0L, 0L});
        set("enhActualCharmsSpent", 0L);

        // Simulate after-state: gold dropped by 50000, 1 charm consumed, level upgraded to 6
        cn.g.bD = 50000;
        cn.g.bC = 50;
        j swordPlus6 = makeItem(101, 3, "Kiem ngan +6", "Kiem ngan", 6, 2);
        bw.V = bag(swordPlus6, makeItem(501, 7, "Cỏ 3 lá", "Cỏ 3 lá", 0, 0)); // 1 charm left
        c.C = 3; // Success

        settleMethod.invoke(null);
        long actualGold = ((Long) get("enhActualGoldSpent")).longValue();
        long actualCharms = ((Long) get("enhActualCharmsSpent")).longValue();
        int currentLv = ((Integer) get("enhCurrentLevel")).intValue();
        check("Actual gold spent measured from live delta (50000)", actualGold == 50000);
        check("Actual charm spent measured from live delta (1)", actualCharms == 1);
        check("Authoritative level updated to 6", currentLv == 6);

        // Subtest 6.2: Protected failure (c.C == 4, same level) -> FAILURE_PROTECTED
        c.C = 4;
        set("snapTargetLevelBefore", 6);
        j swordStill6 = makeItem(101, 3, "Kiem ngan +6", "Kiem ngan", 6, 2);
        bw.V = bag(swordStill6);
        settleMethod.invoke(null);
        String lastRes = (String) get("enhLastResult");
        check("c.C == 4 with unchanged level classifies as FAILURE_PROTECTED", "FAILURE_PROTECTED".equals(lastRes));

        // Subtest 6.3: Degraded failure (c.C == 4, dropped to level 5) -> FAILURE_DEGRADED
        set("snapTargetLevelBefore", 6);
        j swordDegraded5 = makeItem(101, 3, "Kiem ngan +5", "Kiem ngan", 5, 2);
        bw.V = bag(swordDegraded5);
        settleMethod.invoke(null);
        lastRes = (String) get("enhLastResult");
        currentLv = ((Integer) get("enhCurrentLevel")).intValue();
        check("c.C == 4 with lower level classifies as FAILURE_DEGRADED", "FAILURE_DEGRADED".equals(lastRes));
        check("Adopted authoritative lower level 5", currentLv == 5);

        // Subtest 6.4: Destruction (item missing) -> ITEM_DESTROYED (19)
        bw.V = bag(); // empty
        settleMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Missing item after attempt enters ITEM_DESTROYED (19)", state == 19);

        // Subtest 6.5: Target Reached (upgraded to target level 7)
        c.C = 3;
        set("snapTargetLevelBefore", 6);
        j swordTarget7 = makeItem(101, 3, "Kiem ngan +7", "Kiem ngan", 7, 2);
        bw.V = bag(swordTarget7);
        settleMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Reaching target level enters TARGET_REACHED (17)", state == 17);

        // ---------------------------------------------------------------------
        // Test 7: Restart / Manual Review Protection
        // ---------------------------------------------------------------------
        System.out.println("--- Test 7: Restart & manual review protection ---");
        Method recoverMethod = Class.forName("Zeus").getDeclaredMethod("recoverEnhancementSession");
        recoverMethod.setAccessible(true);

        // If in-flight execute was true when session reset/recovered:
        set("enhInFlightExecute", true);
        set("enhState", 12); // ATTEMPTING
        recoverMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Unsettled in-flight execute recovers safely into MANUAL_REVIEW_REQUIRED (33)", state == 33);
        boolean inFlight = ((Boolean) get("enhInFlightExecute")).booleanValue();
        check("In-flight flag cleared to prevent replay", !inFlight);

        // ---------------------------------------------------------------------
        // Test 8: Visual Highlighting
        // ---------------------------------------------------------------------
        System.out.println("--- Test 8: Target highlight ---");
        Method highlightMethod = Class.forName("Zeus").getDeclaredMethod("isEnhancementHighlightActive");
        highlightMethod.setAccessible(true);

        set("enhState", 4); // active state
        set("enhActiveTargetSlot", 2);
        boolean active = ((Boolean) highlightMethod.invoke(null)).booleanValue();
        check("Highlight is active for in-progress enhancement", active);

        set("enhState", 17); // TARGET_REACHED (terminal)
        active = ((Boolean) highlightMethod.invoke(null)).booleanValue();
        check("Highlight clears on terminal TARGET_REACHED", !active);

        System.out.println("=== EnhancementEngineTest Total Failures: " + failures + " ===");
        if (failures > 0) {
            System.exit(1);
        }
    }
}
