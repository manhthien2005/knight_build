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

        // ---------------------------------------------------------------------
        // Test 9: Blacksmith Cross-Map Routing, Ownership & Error Taxonomy
        // ---------------------------------------------------------------------
        System.out.println("--- Test 9: Blacksmith routing & error taxonomy ---");
        Method enhanceMethod = Class.forName("Zeus").getDeclaredMethod("enhance");
        enhanceMethod.setAccessible(true);
        Method goalMethod = Class.forName("Zeus").getDeclaredMethod("goal");
        goalMethod.setAccessible(true);
        Method formatStatusMethod = Class.forName("Zeus").getDeclaredMethod("formatEnhancementStatusJson");
        formatStatusMethod.setAccessible(true);

        setupWorldState(44);
        clearQueue();

        // Subtest 9.1: Auto Farm conflict -> ENHANCEMENT_TRAVEL_CONFLICT (34), preserves atk.map/x/y
        set("atkMode", 1);
        set("atkMap", 44);
        set("atkX", 100);
        set("atkY", 200);
        set("navTarget", -1);
        set("navDone", true);
        set("enhState", 4); // LOCATING_BLACKSMITH
        set("enhAttemptCount", 0);
        set("enhLastResult", null);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Auto Farm conflict transitions to ENHANCEMENT_TRAVEL_CONFLICT (34)", state == 34);
        check("Durable atkMap preserved", ((Integer) get("atkMap")).intValue() == 44);
        check("Durable atkX preserved", ((Integer) get("atkX")).intValue() == 100);
        check("Durable atkY preserved", ((Integer) get("atkY")).intValue() == 200);
        check("No packet sent during conflict", queue().size() == 0);

        // Subtest 9.2: Manual Travel conflict -> ENHANCEMENT_TRAVEL_CONFLICT (34), preserves navTarget/navDone
        set("atkMode", 0);
        set("atkMap", -1);
        set("navTarget", 8);
        set("navDone", false);
        set("enhState", 4);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Manual Travel conflict transitions to ENHANCEMENT_TRAVEL_CONFLICT (34)", state == 34);
        check("Durable navTarget preserved", ((Integer) get("navTarget")).intValue() == 8);
        check("Durable navDone preserved", !((Boolean) get("navDone")).booleanValue());

        // Subtest 9.3: Unavailable BFS route -> BLACKSMITH_ROUTE_UNAVAILABLE (35)
        set("atkMode", 0);
        set("navTarget", -1);
        set("navDone", true);
        setupWorldState(9999); // completely disconnected / unknown map
        set("enhState", 4);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Unavailable BFS route transitions to BLACKSMITH_ROUTE_UNAVAILABLE (35)", state == 35);
        check("Temporary routing cleaned on terminal", !((Boolean) get("enhNavigating")).booleanValue());

        // Subtest 9.4: Map 44 -> Map 1 routing state progression without live enhancement
        setupWorldState(44); // Cây cầu ma ám
        set("enhState", 4); // LOCATING_BLACKSMITH
        set("enhAttemptCount", 0);
        set("enhLastResult", null);
        clearQueue();

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        boolean navigating = ((Boolean) get("enhNavigating")).booleanValue();
        int activeGoal = ((Integer) goalMethod.invoke(null)).intValue();
        check("On Map 44, enhancement activates temporary navigation", navigating);
        check("goal() directs navigation to Map 1", activeGoal == 1);
        check("Remains in LOCATING_BLACKSMITH (4) while cross-map routing", state == 4);
        check("Zero execute packets sent while routing", queue().size() == 0);

        // Character arrives on Map 1, but far from anchor
        setupWorldState(1);
        cn.g.aZ = 100;
        cn.g.ba = 100;
        // Map 1 Pháp sư NPC present at anchor 324, 624
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Pháp sư", -36, 2, 324, 624));
        // Settle map
        Field mapStableField = Class.forName("Zeus").getDeclaredField("mapStableTicks");
        mapStableField.setAccessible(true);
        mapStableField.set(null, 15);
        Field stableMapField = Class.forName("Zeus").getDeclaredField("stableMapId");
        stableMapField.setAccessible(true);
        stableMapField.set(null, 1);

        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("On Map 1 with dist > 45, transitions to APPROACHING_BLACKSMITH (5)", state == 5);

        // Character approaches anchor (dist <= 45)
        cn.g.aZ = 320;
        cn.g.ba = 624;
        clearQueue();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("When near Pháp sư, transitions to OPENING_FORGE (6)", state == 6);
        check("Emitted NPC interaction packet (not opcode 67)", queue().size() == 1);
        ep talkPkt = (ep) queue().elementAt(0);
        check("Interaction packet is not opcode 67", talkPkt.a != 67);

        // Subtest 9.5: Current Map 1 near-anchor fast path
        clearQueue();
        setupWorldState(1);
        cn.g.aZ = 324;
        cn.g.ba = 624;
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Pháp sư", -36, 2, 324, 624));
        mapStableField.set(null, 15);
        stableMapField.set(null, 1);
        set("enhState", 4);
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Near-anchor fast path transitions directly from 4 to OPENING_FORGE (6)", state == 6);
        check("Fast path sends NPC interaction without cross-map routing", queue().size() == 1);

        // Subtest 9.6: Missing live Pháp sư after bounded scans
        setupWorldState(1);
        cn.g.aZ = 324;
        cn.g.ba = 624;
        // cn.j has NPC with cu=-36 but WRONG name / not Pháp sư
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Dan lang", -36, 2, 324, 624));
        set("enhState", 4);
        set("enhBlacksmithScanTicks", 0);
        mapStableField.set(null, 15);
        stableMapField.set(null, 1);

        // Scan past bound
        int maxScans = ((Integer) get("MAX_BLACKSMITH_SCANS")).intValue();
        for (int s = 0; s <= maxScans + 2; s++) {
            enhanceMethod.invoke(null);
        }
        state = ((Integer) get("enhState")).intValue();
        check("Missing live Pháp sư after bounded scans enters BLACKSMITH_NOT_FOUND (36)", state == 36);
        check("cu=-36 with wrong name was rejected (not accepted as sole identity)", state == 36);

        // Subtest 9.7: Forge open failure timeout
        set("enhState", 6); // OPENING_FORGE
        set("enhWait", 0);
        set("enhForgeOpenTries", 0);
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Pháp sư", -36, 2, 324, 624));
        // Retry until forge open fails
        for (int i = 0; i < 5; i++) {
            set("enhWait", 0);
            enhanceMethod.invoke(null);
        }
        state = ((Integer) get("enhState")).intValue();
        check("Forge open timeout enters FORGE_OPEN_FAILED (38)", state == 38);

        // Subtest 9.8: Local pre-execute failures leave structured server result unset
        int[] localErrorStates = new int[] { 34, 35, 36, 37, 38, 20, 21, 23, 24, 25, 26, 27 };
        for (int s : localErrorStates) {
            set("enhState", s);
            set("enhLastResult", null);
            set("enhAttemptCount", 0);
            String json = (String) formatStatusMethod.invoke(null);
            check("State " + s + " status JSON has last_result null", json.contains("\"last_result\": null"));
            check("State " + s + " status JSON has attempt_count 0", json.contains("\"attempt_count\": 0"));
        }

        // ---------------------------------------------------------------------
        // Test 10: Strict Blacksmith Identity & Menu Contract
        // ---------------------------------------------------------------------
        System.out.println("--- Test 10: Strict Blacksmith Identity & Menu Contract ---");
        Method findBsMethod = Class.forName("Zeus").getDeclaredMethod("findBlacksmithNpc");
        findBsMethod.setAccessible(true);
        setupWorldState(1);
        cn.g.aZ = 300;
        cn.g.ba = 600;

        // 10.1: cv=2, name "Pháp sư" => eligible
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Pháp sư", -36, 2, 324, 624));
        fa res = (fa) findBsMethod.invoke(null);
        check("cv=2 with name 'Pháp sư' is eligible", res != null && "Pháp sư".equals(res.cC));

        // 10.2: cv!=2, name "Pháp sư" => not eligible
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Pháp sư", -36, 1, 324, 624)); // cv = 1
        res = (fa) findBsMethod.invoke(null);
        check("cv!=2 with name 'Pháp sư' is NOT eligible", res == null);

        // 10.3: cv=2, name containing 'Cường hóa' but NOT 'Pháp sư' => NOT eligible
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Cường hóa", -36, 2, 324, 624));
        res = (fa) findBsMethod.invoke(null);
        check("cv=2 with name 'Cường hóa' but not 'Pháp sư' is NOT eligible", res == null);

        // 10.4: cu=-36 with non-Pháp-sư name => not eligible
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Thợ rèn", -36, 2, 324, 624));
        res = (fa) findBsMethod.invoke(null);
        check("cu=-36 with non-Pháp-sư name is NOT eligible", res == null);

        // 10.5: Pháp sư candidate with cu=-36 receives priority only after eligibility
        cn.j = new et("npcs");
        cn.j.a(makeNpc("Pháp sư tập sự", -10, 2, 305, 605)); // dist = 10
        cn.j.a(makeNpc("Pháp sư", -36, 2, 350, 650));         // dist = 100, but -10000 bonus
        res = (fa) findBsMethod.invoke(null);
        check("cu=-36 provides distance priority between valid Pháp sư candidates", res != null && res.cu == -36);

        // 10.6: Menu option 'Cường hóa' remains accepted in serverMenu
        Method serverMenuMethod = Class.forName("Zeus").getDeclaredMethod("serverMenu", et.class, int.class, int.class, String.class);
        serverMenuMethod.setAccessible(true);
        set("enhState", 6); // OPENING_FORGE
        clearQueue();
        et menuItems = new et("menu");
        bt opt1 = new bt("Nhiệm vụ", 0);
        bt opt2 = new bt("Cường hoá", 1);
        bt opt3 = new bt("Thoát", 2);
        menuItems.a(opt1);
        menuItems.a(opt2);
        menuItems.a(opt3);
        boolean menuTaken = ((Boolean) serverMenuMethod.invoke(null, menuItems, 10, -36, "Pháp sư")).booleanValue();
        check("serverMenu takes 'Cường hoá' option in state 6", menuTaken);
        check("serverMenu dispatches option pick packet", queue().size() == 1);
        ep pkt10 = (ep) queue().elementAt(0);
        check("serverMenu option pick opcode is -30 (not 67)", pkt10.a == (byte) -30);

        // 10.7: Focused wire test proving NPC=-36, menu=0, option=0 selects short-byte-byte overload
        clearQueue();
        set("enhState", 6);
        et liveMenuItems = new et("menu");
        liveMenuItems.a(new bt("Cường hóa", 0)); // option index 0
        boolean liveMenuTaken = ((Boolean) serverMenuMethod.invoke(null, liveMenuItems, 0, -36, "Pháp sư")).booleanValue();
        check("live-shape menu selection taken", liveMenuTaken);
        check("exactly 1 packet queued for menu selection", queue().size() == 1);
        ep wirePkt = (ep) queue().elementAt(0);
        check("menu-selection wire opcode is -30", wirePkt.a == (byte) -30);
        check("menu-selection produces zero Opcode 67 packets", wirePkt.a != (byte) 67);
        byte[] payload = wirePkt.a();
        java.io.DataInputStream dis = new java.io.DataInputStream(new java.io.ByteArrayInputStream(payload));
        short wireNpc = dis.readShort();
        byte wireMenu = dis.readByte();
        byte wireOption = dis.readByte();
        check("payload field order preserves NPC short -36", wireNpc == (short) -36);
        check("payload field order preserves menu byte 0", wireMenu == (byte) 0);
        check("payload field order preserves option byte 0", wireOption == (byte) 0);

        // ---------------------------------------------------------------------
        // Test 11: Deterministic Pre-Opcode-67 Validation Interlock & Guard
        // ---------------------------------------------------------------------
        System.out.println("--- Test 11: Deterministic Dry-Run Interlock & Structural Opcode 67 Guard ---");
        // Subtest 11.1: Forge-ready + validationOnly=true transitions directly to DRY_RUN_COMPLETE (39)
        setupWorldState(1);
        set("enhState", 6); // OPENING_FORGE
        set("enhValidationOnly", true);
        clearQueue();
        fu.a = makeForgePopup();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Forge-ready with validationOnly=true transitions to DRY_RUN_COMPLETE (39)", state == 39);
        check("State 39 is classified as terminal", ((Boolean) Class.forName("Zeus").getDeclaredMethod("isEnhancementStateTerminal", int.class).invoke(null, 39)).booleanValue());
        check("State 39 name is DRY_RUN_COMPLETE", "DRY_RUN_COMPLETE".equals(Class.forName("Zeus").getDeclaredMethod("getEnhancementStateName", int.class).invoke(null, 39)));
        check("Validation-only at forge-ready sends ZERO Opcode 67 packets", queue().size() == 0);

        // Subtest 11.2: Forge-ready + validationOnly=false still transitions to INSERTING_TARGET (7)
        setupWorldState(1);
        set("enhState", 6); // OPENING_FORGE
        set("enhValidationOnly", false);
        clearQueue();
        fu.a = makeForgePopup();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Forge-ready with validationOnly=false transitions to INSERTING_TARGET (7)", state == 7);

        // Subtest 11.3: Telemetry contract for DRY_RUN_COMPLETE
        set("enhState", 39);
        set("enhValidationOnly", true);
        set("enhErrorCode", null);
        set("enhErrorMessage", null);
        set("enhAttemptCount", 0);
        set("enhLastResult", null);
        String dryRunJson = (String) formatStatusMethod.invoke(null);
        check("Telemetry contains state DRY_RUN_COMPLETE", dryRunJson.indexOf("\"state\": \"DRY_RUN_COMPLETE\"") >= 0);
        check("Telemetry contains attempt_count 0", dryRunJson.indexOf("\"attempt_count\": 0") >= 0);
        check("Telemetry contains last_result null", dryRunJson.indexOf("\"last_result\": null") >= 0);
        check("Telemetry contains validation_only true", dryRunJson.indexOf("\"validation_only\": true") >= 0);
        check("Telemetry has no error_code for DRY_RUN_COMPLETE", dryRunJson.indexOf("\"error_code\"") < 0);

        // Subtest 11.4: Structural Guard at State 7 (INSERTING_TARGET)
        setupWorldState(1);
        set("enhState", 7);
        set("enhValidationOnly", true);
        clearQueue();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("State 7 structural guard intercepts and enters DRY_RUN_COMPLETE (39)", state == 39);
        check("State 7 structural guard sends zero packets", queue().size() == 0);

        // Subtest 11.5: Structural Guard at State 9 (INSERTING_CHARM)
        setupWorldState(1);
        set("enhState", 9);
        set("enhValidationOnly", true);
        clearQueue();
        enhanceMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("State 9 structural guard intercepts and enters DRY_RUN_COMPLETE (39)", state == 39);
        check("State 9 structural guard sends zero packets", queue().size() == 0);

        // Subtest 11.6: Structural Guard at executeEnhancementAttempt
        setupWorldState(1);
        set("enhState", 11);
        set("enhValidationOnly", true);
        clearQueue();
        executeAttemptMethod.invoke(null);
        state = ((Integer) get("enhState")).intValue();
        check("Execute structural guard intercepts and enters DRY_RUN_COMPLETE (39)", state == 39);
        check("Execute structural guard sends zero packets", queue().size() == 0);

        // Subtest 11.7: Request scoping and enhanceReset
        Method resetMethod = Class.forName("Zeus").getDeclaredMethod("enhanceReset");
        resetMethod.setAccessible(true);
        set("enhValidationOnly", true);
        resetMethod.invoke(null);
        boolean valOnlyAfterReset = ((Boolean) get("enhValidationOnly")).booleanValue();
        check("enhanceReset clears validationOnly flag", !valOnlyAfterReset);

        // ---------------------------------------------------------------------
        // Test 12: Interstitial Dialog Recovery & Native Dismissal
        // ---------------------------------------------------------------------
        System.out.println("--- Test 12: Interstitial dialog recovery & native dismissal ---");
        Method dialogRecoveryMethod = Class.forName("Zeus").getDeclaredMethod("dialogRecovery");
        dialogRecoveryMethod.setAccessible(true);

        // 12.1: fu.p single "Đóng" button is dismissed via native button action
        if (fu.p == null) {
            fu.p = new fr();
        }
        fu.s = null;
        fu.p.a = true;
        fr.d = 1; // notification mode
        dialogRecoveryMethod.invoke(null);
        check("fu.p notification (fr.d=1) is dismissed by dialogRecovery", !fu.p.a);

        // 12.2: fu.p menu mode (fr.d=0) is NOT auto-dismissed (fail-closed)
        fu.s = null;
        fu.p.a = true;
        fr.d = 0; // menu mode
        dialogRecoveryMethod.invoke(null);
        check("fu.p menu mode (fr.d=0) fails closed and remains open", fu.p.a);
        fu.p.a = false; // cleanup

        // 12.3: cleanEnhancementRouting clears fu.p and restores fu.a to fu.c
        setupWorldState(1);
        ev forgeScreen = makeForgePopup();
        fu.a = forgeScreen;
        fu.p.a = true;
        call("cleanEnhancementRouting");
        check("cleanEnhancementRouting dismisses active fu.p", !fu.p.a);
        check("cleanEnhancementRouting restores fu.a to fu.c", fu.a == fu.c);

        System.out.println("=== EnhancementEngineTest Total Failures: " + failures + " ===");
        if (failures > 0) {
            System.exit(1);
        }
    }

    static ev makeForgePopup() {
        ev popup = new ev();
        popup.b = new et("tabs");
        c forgeTab = new c("Cuong hoa", (byte) 0);
        popup.b.a(forgeTab);
        popup.a = 0;
        return popup;
    }

    static void setupWorldState(int mapId) {
        if (fu.c == null) {
            fu.c = new cn();
        }
        fu.a = fu.c;
        eh.h = true;
        fu.s = null;
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
        if (cn.g == null) {
            cn.g = new bq(100, (byte) 0, "hero", 0, 0);
        }
        cn.g.cG = 0; // alive (4 is dead)
        cn.g.cx = 0;
        cn.g.cy = 0;
        try {
            Field rst = Class.forName("Zeus").getDeclaredField("readySettleTicks");
            rst.setAccessible(true);
            rst.set(null, 15);
            Field mst = Class.forName("Zeus").getDeclaredField("mapStableTicks");
            mst.setAccessible(true);
            mst.set(null, 15);
            Field sm = Class.forName("Zeus").getDeclaredField("stableMapId");
            sm.setAccessible(true);
            sm.set(null, mapId);
        } catch (Throwable t) {
        }
    }

    static fa makeNpc(String name, int cu, int cv, int x, int y) {
        fa npc = new fa();
        npc.cC = name;
        npc.cu = cu;
        npc.cv = (byte) cv;
        npc.aZ = x;
        npc.ba = y;
        return npc;
    }
}
