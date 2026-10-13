/**
 * Phase 3 Focused TDD Test Suite: Boss Hunt State Machine, Combat, Patrol, Recovery & Arbitration.
 * Verifies BH-06, BH-07, BH-08, BH-09, BH-10 domain behavior against ModByTminh v4.0.3 reference.
 */
public class BossHuntPhase3Test {

    private static int total = 0;
    private static int failures = 0;

    private static void check(String label, boolean ok) {
        total++;
        if (ok) {
            System.out.println("PASS: " + label);
        } else {
            failures++;
            System.err.println("FAIL: " + label);
        }
    }

    private static void checkEquals(String label, Object actual, Object expected) {
        total++;
        boolean ok = (expected == null && actual == null) || (expected != null && expected.equals(actual));
        if (ok) {
            System.out.println("PASS: " + label + " (" + actual + ")");
        } else {
            failures++;
            System.err.println("FAIL: " + label + " - Expected: [" + expected + "], Got: [" + actual + "]");
        }
    }

    public static void main(String[] args) {
        System.out.println("=== BossHuntPhase3Test Suite Starting ===");

        testInitialStateAndEnable();
        testScheduleIngestionAndWaitDepart();
        testDepartureAndTravelTarget();
        testArrivalAndZoneSwitching();
        testTargetDiscrimination();
        testNoDamageThreeSecondsRule();
        testBossDefeatAndRecoveryWait();
        testPriorityArbitrationForgeDungeonDeath();
        testStatusJsonEmission();

        System.out.println("\n=== Test Summary: " + (total - failures) + "/" + total + " passed ===");
        if (failures > 0) {
            System.err.println("FAILED: " + failures + " test(s) failed.");
            System.exit(1);
        } else {
            System.out.println("ALL TESTS PASSED.");
        }
    }

    private static void testInitialStateAndEnable() {
        System.out.println("\n--- testInitialStateAndEnable ---");
        BossHuntConfig config = new BossHuntConfig();
        BossHuntController controller = new BossHuntController(config);

        // Disabled initially
        checkEquals("Initial state is IDLE", controller.getState(), BossHuntState.IDLE);
        check("Does not control movement", !controller.controlsMovement());
        checkEquals("No target map", controller.getBossHuntTargetMapId(), -1);

        // Enable
        controller.setEnabled(true);
        checkEquals("State transitions to NEED_SCHEDULE upon enable", controller.getState(), BossHuntState.NEED_SCHEDULE);
        check("Does not control movement yet in NEED_SCHEDULE", !controller.controlsMovement());

        // Disable
        controller.setEnabled(false);
        checkEquals("State transitions back to IDLE upon disable", controller.getState(), BossHuntState.IDLE);
    }

    private static void testScheduleIngestionAndWaitDepart() {
        System.out.println("\n--- testScheduleIngestionAndWaitDepart ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setEnabled(true);
        BossHuntController controller = new BossHuntController(config);

        // Simulated time: 10:00:00 GMT+7
        long now = 1791514800000L;

        // Ingest schedule row for Dê Vàng (boss 1) at 10:30:00 (due in 30 min)
        controller.getScheduler().ingestRow("D\u00ea V\u00e0ng: 10:30:00. R\u1eebng cao nguy\u00ean K 1", now);

        // Mock map resolution for Rừng cao nguyên -> map ID 33
        controller.setMapResolver(new BossHuntController.MapResolver() {
            public int resolveMapId(String mapName) {
                if ("R\u1eebng cao nguy\u00ean".equals(mapName)) return 33;
                return -1;
            }
        });

        // Tick controller
        controller.tick(now, 1, 0, 1000, false, false, false);

        checkEquals("State is WAIT_DEPART (waiting for spawn time)", controller.getState(), BossHuntState.WAIT_DEPART);
        check("Active entry is boss 1", controller.getActiveEntry() != null && controller.getActiveEntry().bossId == 1);
        check("Does not control movement while waiting", !controller.controlsMovement());
    }

    private static void testDepartureAndTravelTarget() {
        System.out.println("\n--- testDepartureAndTravelTarget ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setEnabled(true);
        BossHuntController controller = new BossHuntController(config);

        controller.setMapResolver(new BossHuntController.MapResolver() {
            public int resolveMapId(String mapName) {
                if ("R\u1eebng cao nguy\u00ean".equals(mapName)) return 33;
                return -1;
            }
        });

        long now = 1791514800000L;
        // Ingest Dê Vàng due in 25 seconds (inside the 30s departure budget)
        controller.getScheduler().ingestRow("D\u00ea V\u00e0ng: 10:00:25. R\u1eebng cao nguy\u00ean K 1", now);

        // Tick controller while player is on town map 1
        controller.tick(now, 1, 0, 1000, false, false, false);

        checkEquals("State transitions to TRAVEL", controller.getState(), BossHuntState.TRAVEL);
        check("Controls movement during travel", controller.controlsMovement());
        checkEquals("Target map ID is 33", controller.getBossHuntTargetMapId(), 33);
    }

    private static void testArrivalAndZoneSwitching() {
        System.out.println("\n--- testArrivalAndZoneSwitching ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setEnabled(true);
        BossHuntController controller = new BossHuntController(config);

        controller.setMapResolver(new BossHuntController.MapResolver() {
            public int resolveMapId(String mapName) {
                if ("R\u1eebng cao nguy\u00ean".equals(mapName)) return 33;
                return -1;
            }
        });

        long now = 1791514800000L;
        // Ingest Dê Vàng in zone 2 ("Khu 2")
        controller.getScheduler().ingestRow("D\u00ea V\u00e0ng: 10:00:00. R\u1eebng cao nguy\u00ean K 2", now);

        // Player arrives at map 33, but is currently in zone index 0 (Khu 1)
        controller.tick(now, 33, 0, 1000, false, false, false);

        checkEquals("State transitions to SWITCH_ZONE", controller.getState(), BossHuntState.SWITCH_ZONE);
        checkEquals("Target zone number is 2", controller.getTargetZoneNumber(), 2);
        checkEquals("Target zone sub opcode byte is 1", controller.getTargetZoneSubIndex(), 1);

        // Player arrives in zone index 1 (Khu 2)
        controller.tick(now, 33, 1, 1000, false, false, false);

        // Boss is not yet in sight -> transitions to PATROL
        checkEquals("State transitions to PATROL when on correct map and zone", controller.getState(), BossHuntState.PATROL);
    }

    private static void testTargetDiscrimination() {
        System.out.println("\n--- testTargetDiscrimination ---");
        // Target discrimination: accepts boss name, rejects ordinary mobs, rejects corpses

        // Target: Dê Vàng (boss 1), monster (typeObject = 1), alive
        check("True boss accepted",
            BossHuntController.isValidBossTarget(1, "D\u00ea V\u00e0ng", 1, 50000, false, 0));

        // Target: Ordinary mob "Sói tuyết"
        check("Ordinary mob rejected",
            !BossHuntController.isValidBossTarget(1, "S\u00f3i tuy\u1ebft", 1, 500, false, 0));

        // Target: Wrong boss "Quỷ đầu bò" when hunting Dê Vàng
        check("Other boss rejected",
            !BossHuntController.isValidBossTarget(1, "Qu\u1ef7 \u0111\u1ea7u b\u00f2", 1, 80000, false, 0));

        // Target: Dead boss (hp = 0, isDie = true, or action = 4)
        check("Dead boss (hp=0) rejected",
            !BossHuntController.isValidBossTarget(1, "D\u00ea V\u00e0ng", 1, 0, false, 0));
        check("Dead boss (isDie=true) rejected",
            !BossHuntController.isValidBossTarget(1, "D\u00ea V\u00e0ng", 1, 50000, true, 0));
        check("Dead boss (action=4) rejected",
            !BossHuntController.isValidBossTarget(1, "D\u00ea V\u00e0ng", 1, 50000, false, 4));

        // Target: Player or NPC (typeObject != 1)
        check("Non-monster rejected",
            !BossHuntController.isValidBossTarget(1, "D\u00ea V\u00e0ng", 0, 50000, false, 0));

        // BHX-02: isBossDefeated verification
        check("Dead boss confirmed by isBossDefeated (hp=0)",
            BossHuntController.isBossDefeated(1, "D\u00ea V\u00e0ng", 1, 0, false, 0));
        check("Dead boss confirmed by isBossDefeated (isDie=true)",
            BossHuntController.isBossDefeated(1, "D\u00ea V\u00e0ng", 1, 50000, true, 0));
        check("Dead boss confirmed by isBossDefeated (action=4)",
            BossHuntController.isBossDefeated(1, "D\u00ea V\u00e0ng", 1, 50000, false, 4));
        check("Living boss rejected by isBossDefeated",
            !BossHuntController.isBossDefeated(1, "D\u00ea V\u00e0ng", 1, 50000, false, 0));
        check("Wrong boss rejected by isBossDefeated",
            !BossHuntController.isBossDefeated(1, "Qu\u1ef7 \u0111\u1ea7u b\u00f2", 1, 0, true, 4));

        // BHX-02: Target tracking methods
        BossHuntController tc = new BossHuntController(new BossHuntConfig());
        check("Initial tracked boss ID is -1", tc.getActiveBossTargetId() == -1);
        check("Initially no active boss target", !tc.hasActiveBossTarget());
        tc.setActiveBossTarget(777, "D\u00ea V\u00e0ng");
        checkEquals("Tracked boss ID", tc.getActiveBossTargetId(), 777);
        checkEquals("Tracked boss name", tc.getActiveBossTargetName(), "D\u00ea V\u00e0ng");
        check("Has active boss target after setting", tc.hasActiveBossTarget());
        tc.clearActiveBossTarget();
        check("Target cleared", tc.getActiveBossTargetId() == -1 && !tc.hasActiveBossTarget());
    }

    private static void testNoDamageThreeSecondsRule() {
        System.out.println("\n--- testNoDamageThreeSecondsRule ---");
        BossHuntController controller = new BossHuntController(new BossHuntConfig());

        long t0 = 1000000L;
        int targetId = 12345;
        int hp = 1000;

        // First hit at t0: timer starts
        check("Initial attack does not trigger no-damage",
            !controller.checkNoDamageFor3s(targetId, hp, t0));

        // At t0 + 2000ms: same target, same HP (2s elapsed) -> false
        check("At 2s without damage, no-damage not yet triggered",
            !controller.checkNoDamageFor3s(targetId, hp, t0 + 2000L));

        // At t0 + 3000ms: same target, same HP (3s elapsed) -> TRUE!
        check("At 3s without damage, no-damage rule triggers",
            controller.checkNoDamageFor3s(targetId, hp, t0 + 3000L));

        // If HP decreases at 2.5s, timer resets
        controller.resetNoDamageTimer();
        check("After reset, timer clean",
            !controller.checkNoDamageFor3s(targetId, hp, t0));
        // HP drops to 900
        check("HP drop resets timer",
            !controller.checkNoDamageFor3s(targetId, 900, t0 + 2500L));
        // At t0 + 4000ms (1.5s after drop) -> false
        check("1.5s after damage does not trigger",
            !controller.checkNoDamageFor3s(targetId, 900, t0 + 4000L));

        // BHX-08: onNoDamageTimeout transitions away from COMBAT to prevent same-tick re-fire
        controller.setState(BossHuntState.COMBAT);
        controller.setActiveBossTarget(targetId, "D\u00ea V\u00e0ng");
        controller.onNoDamageTimeout(t0 + 4000L);
        checkEquals("State transitions to PATROL on timeout", BossHuntState.PATROL, controller.getState());
        check("Controller no longer in COMBAT on timeout", controller.getState() != BossHuntState.COMBAT);
    }

    private static void testBossDefeatAndRecoveryWait() {
        System.out.println("\n--- testBossDefeatAndRecoveryWait ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setEnabled(true);
        BossHuntController controller = new BossHuntController(config);

        controller.setMapResolver(new BossHuntController.MapResolver() {
            public int resolveMapId(String mapName) {
                return 33;
            }
        });

        long now = 1791514800000L;
        controller.getScheduler().ingestRow("D\u00ea V\u00e0ng: 10:00:00. R\u1eebng cao nguy\u00ean K 1", now);
        controller.tick(now, 33, 0, 1000, false, false, false); // On map & zone

        // Boss defeated event (boss HP reaches 0)
        controller.onBossDefeated(now);
        checkEquals("State transitions to RECOVER", controller.getState(), BossHuntState.RECOVER);
        check("Does not control movement during recovery wait", !controller.controlsMovement());

        // At now + 5s: 10s wait not complete
        controller.tick(now + 5000L, 33, 0, 1000, false, false, false);
        checkEquals("Still in RECOVER at 5s", controller.getState(), BossHuntState.RECOVER);

        // At now + 10s: 10s wait complete -> transitions back to NEED_SCHEDULE to refresh schedule
        controller.tick(now + 10000L, 33, 0, 1000, false, false, false);
        checkEquals("Transitions to NEED_SCHEDULE after 10s recovery", controller.getState(), BossHuntState.NEED_SCHEDULE);
    }

    private static void testPriorityArbitrationForgeDungeonDeath() {
        System.out.println("\n--- testPriorityArbitrationForgeDungeonDeath ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setEnabled(true);
        BossHuntController controller = new BossHuntController(config);

        controller.setMapResolver(new BossHuntController.MapResolver() {
            public int resolveMapId(String mapName) { return 33; }
        });

        long now = 1791514800000L;
        controller.getScheduler().ingestRow("D\u00ea V\u00e0ng: 10:00:00. R\u1eebng cao nguy\u00ean K 1", now);
        controller.tick(now, 1, 0, 1000, false, false, false);
        checkEquals("Traveling", controller.getState(), BossHuntState.TRAVEL);

        // 1. Higher priority (Forge / Dungeon) active -> PAUSED
        controller.tick(now + 100L, 1, 0, 1000, false, false, true /* higherPriorityActive */);
        checkEquals("Higher priority task pauses Boss Hunt", controller.getState(), BossHuntState.PAUSED);
        check("Does not control movement while PAUSED", !controller.controlsMovement());

        // Resume when higher priority clears
        controller.tick(now + 200L, 1, 0, 1000, false, false, false);
        checkEquals("Resumes TRAVEL when higher priority clears", controller.getState(), BossHuntState.TRAVEL);

        // 2. Player death (isDead = true) -> PAUSED
        controller.tick(now + 300L, 1, 0, 0, true /* isDead */, false, false);
        checkEquals("Player death pauses Boss Hunt", controller.getState(), BossHuntState.PAUSED);

        // 3. Manual movement -> cancels active travel errand and yields
        controller.tick(now + 400L, 1, 0, 1000, false, true /* manualMovement */, false);
        checkEquals("Manual movement pauses Boss Hunt", controller.getState(), BossHuntState.PAUSED);
    }

    private static void testStatusJsonEmission() {
        System.out.println("\n--- testStatusJsonEmission ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setEnabled(true);
        BossHuntController controller = new BossHuntController(config);

        long now = 1791514800000L;
        controller.getScheduler().ingestRow("D\u00ea V\u00e0ng: 10:00:00. R\u1eebng cao nguy\u00ean K 1", now);

        String json = controller.toStatusJson(now);
        check("Status JSON non-empty", json != null && json.length() > 0);
        check("JSON contains state", json.indexOf("\"state\":") >= 0);
        check("JSON contains activeBoss", json.indexOf("\"activeBoss\":") >= 0);
        check("JSON contains clockPolicy", json.indexOf("MINH_COMPAT_ABSOLUTE_GMT7") >= 0);
    }
}
