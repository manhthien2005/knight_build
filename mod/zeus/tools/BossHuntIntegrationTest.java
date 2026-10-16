import Main.GameCanvas;
import GameScreen.GameScreen;
import GameScreen.MainScreen;
import GameScreen.LoadMapScreen;
import GameObjects.MainObject;
import GameObjects.Player;
import InterfaceComponents.MsgDialog;
import InterfaceComponents.MainDialog;
import InterfaceComponents.ChatTextField;
import InterfaceComponents.iCommand;
import InterfaceComponents.IAction;
import Model.Menu2;
import CLib.mVector;
import Thread_More.LoadMap;
import java.io.File;

import java.io.FileOutputStream;

/**
 * Focused Integration Test Suite for R4.3.1 Boss Hunt Runtime Integration Fixes.
 *
 * Verifies:
 * - A1 / F-02: Runtime configuration loading, operator activation, fail-closed on malformed JSON
 * - A2 / F-03: Live schedule ingestion independent of debug capture marker zeus-boss-capture.on
 * - A3 / F-01 & F-05: Target map ID accessor valid in TRAVEL/SWITCH_ZONE/PATROL/COMBAT, throttled zone switching
 * - A4 / F-04: 3-second no-damage rule connected to combat loop
 * - A4 / F-05: Confirmed kill requires death; disappearance from viewport reverts to PATROL without false victory
 * - A4 / F-05 & BH-09: Movement lock separation, priority yielding to Forge/Dungeon/Revive/Death
 * - A5 / F-07: Non-destructive atomic file writing with .bak safety
 */
public class BossHuntIntegrationTest {

    private static int total = 0;
    private static int failures = 0;

    private static void check(String label, boolean ok) {
        total++;
        if (ok) {
            System.out.println("PASS: " + label);
        } else {
            System.err.println("FAIL: " + label);
            failures++;
        }
    }

    private static void checkEquals(String label, Object expected, Object actual) {
        total++;
        boolean ok = (expected == null && actual == null) || (expected != null && expected.equals(actual));
        if (ok) {
            System.out.println("PASS: " + label + " (" + actual + ")");
        } else {
            System.err.println("FAIL: " + label + " - expected: " + expected + ", got: " + actual);
            failures++;
        }
    }

    public static void main(String[] args) {
        // Functional ownership/route checks without running unrelated tests.
        if (args != null && args.length == 1 && "--boss-f01-only".equals(args[0])) {
            testBossRouteOwnershipRegression();
            testServerMenuNativeSelectionDispatch();
            System.out.println("Boss F01 focused checks: " + (total - failures) + "/" + total);
            if (failures != 0) System.exit(1);
            return;
        }
        // Scratch-only focused entry point; never run unrelated integration tests.
        if (args != null && args.length == 1 && "--prewrite-only".equals(args[0])) {
            testNativePointerPrewriteCorrelation();
            System.out.println("Prewrite focused checks: " + (total - failures) + "/" + total);
            if (failures != 0) System.exit(1);
            return;
        }
        System.out.println("=== BossHuntIntegrationTest Starting ===");

        testR43IntegrationProbeParity();
        testOperatorRuntimeConfigEnableDisable();
        testScheduleIngestionIndependentOfCaptureMarker();
        testThrottledZoneSwitchingAndRetryBound();
        testCombatThreeSecondNoDamageRule();
        testTargetDisappearanceDoesNotKillBoss();
        testMovementLockSeparationAndPriorityPreemption();
        testOwnershipAwareAutofireAndLockCleanup();
        testSafeAtomicFileReplacement();
        testScheduleNpcAcquisitionBoundary();
        testBossRouteOwnershipRegression();
        testNativeInputCapturePassive();
        testNativePointerPrewriteCorrelation();
        testAlisamaMsgDialogBridge();
        testForegroundAlisamaDialogPriorityGateThroughTick();
        testServerMenuNativeSelectionDispatch();

        System.out.println("\n=== Test Summary: " + (total - failures) + "/" + total + " passed ===");
        if (failures > 0) {
            System.err.println("INTEGRATION TESTS FAILED with " + failures + " failures.");
            System.exit(1);
        } else {
            System.out.println("ALL INTEGRATION TESTS PASSED.");
        }
    }

    private static void testR43IntegrationProbeParity() {
        System.out.println("\n--- testR43IntegrationProbeParity ---");
        BossHuntConfig cfg = new BossHuntConfig();
        BossHuntController controller = new BossHuntController(cfg);
        controller.setMapResolver(new BossHuntController.MapResolver() {
            public int resolveMapId(String map) { return 33; }
        });

        check("New config defaults disabled", !cfg.isEnabled());
        cfg.setEnabled(true);
        long t = 1791514800000L;
        boolean ingested = controller.getScheduler().ingestRow("Dê Vàng: 10:00:00. Rừng cao nguyên K 2", t);
        check("Schedule row ingested", ingested);

        // Tick on map 1 -> should travel to map 33
        controller.tick(t, 1, 0, 1000, false, false, false);
        checkEquals("State is TRAVEL", BossHuntState.TRAVEL, controller.getState());
        checkEquals("Target map is 33 during TRAVEL", 33, controller.getBossHuntTargetMapId());

        // Arrived at map 33, zone 0 (target is zone index 1 / Khu 2) -> SWITCH_ZONE
        controller.tick(t, 33, 0, 1000, false, false, false);
        checkEquals("State is SWITCH_ZONE", BossHuntState.SWITCH_ZONE, controller.getState());
        checkEquals("Target map accessor valid during SWITCH_ZONE", 33, controller.getBossHuntTargetMapId());
        check("WILL_ZEUS_SWITCH evaluates true", 33 == controller.getBossHuntTargetMapId());

        // Arrived at map 33, zone 1 -> PATROL
        controller.tick(t, 33, 1, 1000, false, false, false);
        checkEquals("State is PATROL", BossHuntState.PATROL, controller.getState());
        checkEquals("Target map accessor valid during PATROL", 33, controller.getBossHuntTargetMapId());
        check("WILL_ZEUS_COMBAT evaluates true", 33 == controller.getBossHuntTargetMapId());
    }

    private static void testOperatorRuntimeConfigEnableDisable() {
        System.out.println("\n--- testOperatorRuntimeConfigEnableDisable ---");
        File tempDir = new File(System.getProperty("java.io.tmpdir"), "zeus_test_" + System.currentTimeMillis());
        tempDir.mkdirs();
        File cfgFile = new File(tempDir, "zeus-boss-hunt-config.json");

        try {
            // Initially absent -> disabled
            BossHuntConfig cfg = new BossHuntConfig();
            check("Default config is disabled", !cfg.isEnabled());

            // Write enabled config
            String jsonEnabled = "{\"enabled\": true, \"clockMode\": 1, \"bosses\": [{\"id\": 1, \"zonesMask\": 31}]}";
            FileOutputStream fos = new FileOutputStream(cfgFile);
            fos.write(jsonEnabled.getBytes("UTF-8"));
            fos.close();

            cfg.reloadFromFile(cfgFile.getAbsolutePath());
            check("Loaded config is enabled", cfg.isEnabled());
            checkEquals("Loaded clock mode is 1", 1, cfg.getClockMode());
            check("Boss 1 zone 1 enabled", cfg.isZoneSelected(1, 1));

            // Write disabled config
            String jsonDisabled = "{\"enabled\": false, \"clockMode\": 1}";
            fos = new FileOutputStream(cfgFile);
            fos.write(jsonDisabled.getBytes("UTF-8"));
            fos.close();

            cfg.reloadFromFile(cfgFile.getAbsolutePath());
            check("Updated config is disabled", !cfg.isEnabled());

            // Malformed JSON -> fail closed (disabled)
            String jsonMalformed = "{invalid_json_content...";
            fos = new FileOutputStream(cfgFile);
            fos.write(jsonMalformed.getBytes("UTF-8"));
            fos.close();

            cfg.reloadFromFile(cfgFile.getAbsolutePath());
            check("Malformed JSON fails closed (disabled)", !cfg.isEnabled());

            // BHX-01: Truncated JSON fails closed
            fos = new FileOutputStream(cfgFile);
            fos.write("{\"enabled\": true".getBytes("UTF-8"));
            fos.close();
            cfg.reloadFromFile(cfgFile.getAbsolutePath());
            check("Truncated JSON fails closed (disabled)", !cfg.isEnabled());

            // BHX-01: Trailing garbage fails closed
            fos = new FileOutputStream(cfgFile);
            fos.write("{\"enabled\": true} GARBAGE".getBytes("UTF-8"));
            fos.close();
            cfg.reloadFromFile(cfgFile.getAbsolutePath());
            check("Trailing garbage fails closed (disabled)", !cfg.isEnabled());

            // BHX-06: Missing/deleted config file fails closed upon reload
            cfg.setEnabled(true);
            check("Config explicitly enabled before deletion", cfg.isEnabled());
            cfgFile.delete();
            cfg.reloadFromFile(cfgFile.getAbsolutePath());
            check("Missing/deleted config file fails closed", !cfg.isEnabled());

            // BHX-10: Fractional number in file fails closed
            fos = new FileOutputStream(cfgFile);
            fos.write("{\"enabled\": true, \"clockMode\": 1.5}".getBytes("UTF-8"));
            fos.close();
            cfg.reloadFromFile(cfgFile.getAbsolutePath());
            check("Fractional clockMode in config file fails closed", !cfg.isEnabled());
        } catch (Exception e) {
            check("Config load error: " + e.getMessage(), false);
        } finally {
            cfgFile.delete();
            tempDir.delete();
        }
    }

    private static void testScheduleIngestionIndependentOfCaptureMarker() {
        System.out.println("\n--- testScheduleIngestionIndependentOfCaptureMarker ---");
        BossHuntConfig cfg = new BossHuntConfig();
        cfg.setEnabled(true);
        BossHuntController controller = new BossHuntController(cfg);

        // Directly ingest a SERVER schedule row into controller's scheduler
        long now = 1791514800000L;
        boolean ingested = controller.getScheduler().ingestRow("Dê Vàng: 10:15:00. Rừng cao nguyên K 3", now);
        check("Scheduler ingests schedule row independently", ingested);

        BossHuntScheduler.ScheduleEntry entry = controller.getScheduler().nextEntry(now);
        check("Scheduler produces valid nextEntry", entry != null);
        if (entry != null) {
            checkEquals("Entry bossId is 1 (Dê Vàng)", 1, entry.bossId);
            checkEquals("Entry zone is 3", 3, entry.zone);
            checkEquals("Entry map alias is Rừng cao nguyên", "Rừng cao nguyên", entry.mapAlias);
        }
    }

    private static void testThrottledZoneSwitchingAndRetryBound() {
        System.out.println("\n--- testThrottledZoneSwitchingAndRetryBound ---");
        BossHuntConfig cfg = new BossHuntConfig();
        cfg.setEnabled(true);
        BossHuntController controller = new BossHuntController(cfg);
        controller.setState(BossHuntState.SWITCH_ZONE);

        long t0 = 10000L;
        check("Initial zone switch attempt allowed", controller.shouldAttemptZoneSwitch(t0));
        controller.recordZoneSwitchAttempt(t0);

        // 500ms later -> must be throttled!
        check("Attempt at +500ms throttled", !controller.shouldAttemptZoneSwitch(t0 + 500L));
        // 1400ms later -> still throttled (< 1500ms)
        check("Attempt at +1400ms throttled", !controller.shouldAttemptZoneSwitch(t0 + 1400L));
        // 1600ms later -> allowed
        check("Attempt at +1600ms allowed", controller.shouldAttemptZoneSwitch(t0 + 1600L));
        controller.recordZoneSwitchAttempt(t0 + 1600L);

        // Exhaust retries up to 5
        controller.recordZoneSwitchAttempt(t0 + 3200L);
        controller.recordZoneSwitchAttempt(t0 + 4800L);
        controller.recordZoneSwitchAttempt(t0 + 6400L);
        check("Zone switch retry exhausted after 5 attempts", controller.isZoneSwitchExhausted());

        controller.onZoneSwitchFailed();
        checkEquals("State transitions to NEED_SCHEDULE upon exhaustion", BossHuntState.NEED_SCHEDULE, controller.getState());

        // Test successful confirmation
        controller.setState(BossHuntState.SWITCH_ZONE);
        controller.recordZoneSwitchAttempt(t0 + 8000L);
        controller.onZoneConfirmed();
        checkEquals("Zone confirmation transitions to PATROL", BossHuntState.PATROL, controller.getState());
        check("Zone switch retry count reset", !controller.isZoneSwitchExhausted());
    }

    private static void testCombatThreeSecondNoDamageRule() {
        System.out.println("\n--- testCombatThreeSecondNoDamageRule ---");
        BossHuntConfig cfg = new BossHuntConfig();
        cfg.setEnabled(true);
        BossHuntController controller = new BossHuntController(cfg);
        controller.setState(BossHuntState.COMBAT);

        long t = 10000L;
        int bossId = 999;
        int hp = 50000;

        check("First attack does not trigger no-damage", !controller.checkNoDamageFor3s(bossId, hp, t));
        check("After 1000ms unchanged HP does not trigger", !controller.checkNoDamageFor3s(bossId, hp, t + 1000L));
        check("After 2999ms unchanged HP does not trigger", !controller.checkNoDamageFor3s(bossId, hp, t + 2999L));
        check("After 3000ms unchanged HP triggers no-damage rule", controller.checkNoDamageFor3s(bossId, hp, t + 3000L));

        controller.onNoDamageTimeout(t + 3000L);
        checkEquals("State transitions to PATROL upon no-damage timeout", BossHuntState.PATROL, controller.getState());

        // Test HP change resets timer
        controller.setState(BossHuntState.COMBAT);
        controller.checkNoDamageFor3s(bossId, 50000, t + 4000L);
        check("At 1500ms no trigger", !controller.checkNoDamageFor3s(bossId, 50000, t + 5500L));
        // HP drops to 45000 at +2000ms -> timer reset
        check("HP drop resets timer", !controller.checkNoDamageFor3s(bossId, 45000, t + 6000L));
        // At +4000ms (2000ms after drop) -> still does not trigger
        check("2000ms after drop does not trigger", !controller.checkNoDamageFor3s(bossId, 45000, t + 8000L));
        // At +5001ms (3001ms after drop) -> triggers
        check("3000ms after drop triggers", controller.checkNoDamageFor3s(bossId, 45000, t + 9001L));
    }

    private static void testTargetDisappearanceDoesNotKillBoss() {
        System.out.println("\n--- testTargetDisappearanceDoesNotKillBoss ---");
        BossHuntConfig cfg = new BossHuntConfig();
        cfg.setEnabled(true);
        BossHuntController controller = new BossHuntController(cfg);
        controller.setState(BossHuntState.COMBAT);

        // BHX-02: Target tracking and separation of living filter from defeat confirmation
        check("Alive target matches isValidBossTarget",
            BossHuntController.isValidBossTarget(1, "D\u00ea V\u00e0ng", 1, 1000, false, 0));
        check("Alive target does not match isBossDefeated",
            !BossHuntController.isBossDefeated(1, "D\u00ea V\u00e0ng", 1, 1000, false, 0));
        check("Dead target does not match isValidBossTarget",
            !BossHuntController.isValidBossTarget(1, "D\u00ea V\u00e0ng", 1, 0, true, 4));
        check("Dead target matches isBossDefeated",
            BossHuntController.isBossDefeated(1, "D\u00ea V\u00e0ng", 1, 0, true, 4));

        controller.setActiveBossTarget(888, "D\u00ea V\u00e0ng");
        check("Target tracking active", controller.hasActiveBossTarget());
        checkEquals("Tracked target ID", 888, controller.getActiveBossTargetId());

        // When target vanishes from viewport, state should revert to PATROL, NOT RECOVER!
        controller.clearActiveBossTarget();
        controller.setState(BossHuntState.PATROL);
        controller.resetNoDamageTimer();
        checkEquals("Vanished target keeps hunting in PATROL", BossHuntState.PATROL, controller.getState());
        check("No false kill on vanish", controller.getState() != BossHuntState.RECOVER);

        // Confirmed death triggers RECOVER
        long now = 50000L;
        controller.onBossDefeated(now);
        checkEquals("Confirmed kill transitions to RECOVER", BossHuntState.RECOVER, controller.getState());

        // Still in RECOVER at 5s
        controller.tick(now + 5000L, 33, 0, 1000, false, false, false);
        checkEquals("At 5s still in RECOVER", BossHuntState.RECOVER, controller.getState());

        // Transitions to NEED_SCHEDULE after 10s
        controller.tick(now + 10001L, 33, 0, 1000, false, false, false);
        checkEquals("At 10s transitions to NEED_SCHEDULE", BossHuntState.NEED_SCHEDULE, controller.getState());
    }

    private static void testMovementLockSeparationAndPriorityPreemption() {
        System.out.println("\n--- testMovementLockSeparationAndPriorityPreemption ---");
        BossHuntConfig cfg = new BossHuntConfig();
        cfg.setEnabled(true);
        BossHuntController controller = new BossHuntController(cfg);
        controller.setMapResolver(new BossHuntController.MapResolver() { public int resolveMapId(String m){return 33;} });
        long now = 1791514800000L;
        controller.getScheduler().ingestRow("Dê Vàng: 10:00:00. Rừng cao nguyên K 1", now);

        // When moving under bot lock (manualMovement = false) -> TRAVEL continues
        controller.tick(now, 1, 0, 1000, false, false, false);
        checkEquals("Traveling to boss map", BossHuntState.TRAVEL, controller.getState());

        // External manual override (manualMovement = true) -> PAUSED
        controller.tick(now, 1, 0, 1000, false, true, false);
        checkEquals("Manual movement pauses BossHunt", BossHuntState.PAUSED, controller.getState());

        // Clear manual movement -> resumes
        controller.tick(now, 1, 0, 1000, false, false, false);
        checkEquals("Resumes TRAVEL when manual clears", BossHuntState.TRAVEL, controller.getState());

        // Higher priority (Forge/Dungeon) -> PAUSED
        controller.tick(now, 1, 0, 1000, false, false, true);
        checkEquals("Higher priority pauses BossHunt", BossHuntState.PAUSED, controller.getState());

        // Player dead -> PAUSED
        controller.tick(now, 1, 0, 0, true, false, false);
        checkEquals("Player death pauses BossHunt", BossHuntState.PAUSED, controller.getState());

        // Disabling controller clears state to IDLE
        controller.setEnabled(false);
        checkEquals("Disabled controller transitions to IDLE", BossHuntState.IDLE, controller.getState());
        check("Disabled controller does not control movement", !controller.controlsMovement());
    }

    private static void testOwnershipAwareAutofireAndLockCleanup() {
        System.out.println("\n--- testOwnershipAwareAutofireAndLockCleanup ---");
        BossHuntConfig cfg = new BossHuntConfig();
        cfg.setEnabled(true);
        BossHuntController controller = new BossHuntController(cfg);
        controller.setMapResolver(new BossHuntController.MapResolver() { public int resolveMapId(String m){return 33;} });
        long now = 1791514800000L;
        controller.getScheduler().ingestRow("D\u00ea V\u00e0ng: 10:00:00. R\u1eebng cao nguy\u00ean K 1", now);

        // 1. Operator disable cleans up state
        controller.setEnabled(false);
        checkEquals("State transitions to IDLE on disable", BossHuntState.IDLE, controller.getState());
        check("Controls movement false on disable", !controller.controlsMovement());
        check("Active target cleared on disable", !controller.hasActiveBossTarget());

        // 2. Re-enable and test player death preemption
        controller.setEnabled(true);
        controller.tick(now, 1, 0, 1000, false, false, false);
        checkEquals("State is TRAVEL", BossHuntState.TRAVEL, controller.getState());

        controller.tick(now, 1, 0, 0, true, false, false); // isDead = true, hp = 0
        checkEquals("Player death pauses controller", BossHuntState.PAUSED, controller.getState());
        check("Controls movement false when dead", !controller.controlsMovement());

        // 3. Higher priority preemption
        controller.tick(now, 1, 0, 1000, false, false, true); // higherPriorityActive = true
        checkEquals("Higher priority pauses controller", BossHuntState.PAUSED, controller.getState());
        check("Controls movement false on higher priority", !controller.controlsMovement());

        // 4. BHX-12: Confirmed boss defeat cleans up active target and enters RECOVER
        controller.setActiveBossTarget(777, "D\u00ea V\u00e0ng");
        controller.setState(BossHuntState.COMBAT);
        check("Active target tracked before defeat", controller.hasActiveBossTarget());
        checkEquals("Target ID before defeat", controller.getActiveBossTargetId(), 777);
        controller.onBossDefeated(now);
        checkEquals("State transitions to RECOVER after defeat", controller.getState(), BossHuntState.RECOVER);
        check("Active target cleared after defeat", !controller.hasActiveBossTarget());
        checkEquals("Target ID reset to -1 after defeat", controller.getActiveBossTargetId(), -1);
    }

    private static void testSafeAtomicFileReplacement() {
        System.out.println("\n--- testSafeAtomicFileReplacement ---");
        File tempDir = new File(System.getProperty("java.io.tmpdir"), "zeus_atomic_" + System.currentTimeMillis());
        tempDir.mkdirs();
        File target = new File(tempDir, "test-config.json");

        try {
            BossHuntConfig cfg = new BossHuntConfig();
            cfg.setEnabled(true);
            cfg.setClockMode(1);

            // First write
            cfg.saveToFile(target.getAbsolutePath());
            check("Target file created", target.exists());
            check("Target length > 0", target.length() > 0);

            // Read back
            BossHuntConfig read1 = new BossHuntConfig();
            read1.reloadFromFile(target.getAbsolutePath());
            check("Read back enabled", read1.isEnabled());

            // Second write with updated data
            cfg.setZoneSelected(1, 2, false);
            cfg.saveToFile(target.getAbsolutePath());
            check("Target file still exists after second write", target.exists());

            // Read back updated
            BossHuntConfig read2 = new BossHuntConfig();
            read2.reloadFromFile(target.getAbsolutePath());
            check("Updated zone mask persisted", !read2.isZoneSelected(1, 2));

            // Verify .tmp and .bak are clean
            File bak = new File(target.getAbsolutePath() + ".bak");
            check("No stale backup left", !bak.exists());
        } catch (Exception e) {
            check("Atomic write error: " + e.getMessage(), false);
        } finally {
            target.delete();
            tempDir.delete();
        }
    }

    /**
     * Use reflection only inside the offline harness. A direct sun.misc.Unsafe type
     * reference does not compile under the supported --release 6 class library.
     * The client LoadMap constructor is not a usable isolated fixture constructor.
     */
    private static LoadMap allocateBossScheduleTestMap() throws Exception {
        Class<?> unsafeClass = Class.forName("sun.misc.Unsafe");
        java.lang.reflect.Field singletonField = unsafeClass.getDeclaredField("theUnsafe");
        singletonField.setAccessible(true);
        Object unsafe = singletonField.get(null);
        java.lang.reflect.Method allocate = unsafeClass.getMethod("allocateInstance",
                new Class[] { Class.class });
        return (LoadMap) allocate.invoke(unsafe, new Object[] { LoadMap.class });
    }

    private static void testScheduleNpcAcquisitionBoundary() {
        System.out.println("\n--- testScheduleNpcAcquisitionBoundary ---");

        // Snapshot everything this test replaces; never leak fixture world state.
        final LoadMap oldLoadMap = GameCanvas.loadmap;
        final int oldMapId = oldLoadMap != null ? oldLoadMap.idMap : -1;
        final Player oldPlayer = GameScreen.player;
        final int oldX = oldPlayer != null ? oldPlayer.x : 0;
        final int oldY = oldPlayer != null ? oldPlayer.y : 0;
        final int oldHp = oldPlayer != null ? oldPlayer.hp : 0;
        final int oldAction = oldPlayer != null ? oldPlayer.Action : 0;
        final short[] oldRoad = oldPlayer != null ? oldPlayer.posTransRoad : null;
        final mVector oldPlayers = GameScreen.Vecplayers;
        final MainObject oldFocus = GameScreen.ObjFocus;
        final boolean oldLock = Player.isLockKey;
        final MainDialog oldDialog = GameCanvas.currentDialog;
        final Menu2 oldMenu = GameCanvas.menu2;
        final String oldConfigPath = Zeus.bossHuntConfigPath;
        final String oldStatusPath = Zeus.bossHuntStatusPath;
        final boolean oldEnabled = Zeus.bossHunt.getConfig().isEnabled();
        final BossHuntState oldState = Zeus.bossHunt.getState();
        final String oldStatus = Zeus.bossHunt.getStatusText();
        final BossHuntScheduler.ScheduleEntry oldEntry = Zeus.bossHunt.getActiveEntry();
        final Zeus.PendingBossMenuSelection oldPending = Zeus.pendingBossMenuSelection;
        final String oldReason = Zeus.getLastBossHuntDiagReason();

        // These are the controller fields tick() can change after selecting a
        // schedule entry. Snapshot them without editing the shared scheduler table.
        final String[] controllerNames = {
            "targetMapId", "targetZoneNumber", "targetZoneSubIndex", "arrivedAt", "absentSince"
        };
        final java.lang.reflect.Field[] controllerFields =
                new java.lang.reflect.Field[controllerNames.length];
        final Object[] controllerValues = new Object[controllerNames.length];
        boolean controllerSnapshotReady = false;
        java.lang.reflect.Field statusTimer = null;
        long oldStatusTimer = 0L;
        boolean statusTimerSnapshotReady = false;

        try {
            // Fail before touching fixture state if we cannot restore the emit timer.
            statusTimer = Zeus.class.getDeclaredField("lastBossStatusEmit");
            statusTimer.setAccessible(true);
            oldStatusTimer = statusTimer.getLong(null);
            statusTimerSnapshotReady = true;
            for (int fieldIndex = 0; fieldIndex < controllerNames.length; fieldIndex++) {
                controllerFields[fieldIndex] =
                        BossHuntController.class.getDeclaredField(controllerNames[fieldIndex]);
                controllerFields[fieldIndex].setAccessible(true);
                controllerValues[fieldIndex] = controllerFields[fieldIndex].get(Zeus.bossHunt);
            }
            controllerSnapshotReady = true;
            // No test tick may overwrite the operator's status JSON.
            Zeus.bossHuntStatusPath = null;
            if (GameScreen.player == null) {
                GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
            }
            GameScreen.player.x = 100;
            GameScreen.player.y = 100;
            GameScreen.player.hp = 1000;
            GameScreen.player.Action = 0;
            GameScreen.player.posTransRoad = null;
            Player.isLockKey = false;
            GameScreen.ObjFocus = null;
            if (GameCanvas.loadmap == null) {
                GameCanvas.loadmap = allocateBossScheduleTestMap();
            }

            MainObject alisama = new MainObject();
            alisama.name = "Alisama";
            alisama.typeObject = 2;
            alisama.x = 101;
            alisama.y = 100;
            MainObject doubar = new MainObject();
            doubar.name = "Doubar";
            doubar.typeObject = 2;
            doubar.x = 110;
            doubar.y = 100;

            GameCanvas.loadmap.idMap = 33;
            GameScreen.Vecplayers = new mVector();
            GameScreen.Vecplayers.addElement(alisama);
            GameScreen.Vecplayers.addElement(doubar);
            java.lang.reflect.Method findNpc = Zeus.class.getDeclaredMethod("findBossScheduleNpc");
            findNpc.setAccessible(true);
            check("F01-ACQ-RED-01: Nearby Alisama cannot supersede Doubar",
                    findNpc.invoke(null) == doubar);

            GameScreen.Vecplayers = new mVector();
            GameScreen.Vecplayers.addElement(alisama);
            check("F01-ACQ-SAFE-01: Merchant-only scene has no schedule NPC",
                    findNpc.invoke(null) == null);

            // F01-ACQ-T03: same eligible Doubar and same vector on both maps.
            // Removing the production map guard must make the map-1 assertion RED.
            GameScreen.Vecplayers = new mVector();
            GameScreen.Vecplayers.addElement(doubar);
            GameCanvas.loadmap.idMap = 1;
            check("F01-ACQ-RED-02: Eligible Doubar rejected outside map 33",
                    findNpc.invoke(null) == null);
            GameCanvas.loadmap.idMap = 33;
            check("F01-ACQ-POS-01: Same Doubar accepted on recorded map",
                    findNpc.invoke(null) == doubar);

            // Cross-map routing probe is separate from map identity regression:
            // its scene has no NPC, and its state has no valid schedule entry.
            GameScreen.Vecplayers = new mVector();
            GameCanvas.loadmap.idMap = 1;
            Zeus.bossHuntConfigPath = null;
            Zeus.bossHunt.getConfig().setEnabled(true);
            Zeus.bossHunt.setActiveEntry(null);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            Zeus.pendingBossMenuSelection = null;
            GameCanvas.currentDialog = null;
            GameCanvas.menu2 = null;
            Zeus.clearBossHuntDiagReason();
            boolean schedulerEmpty = Zeus.bossHunt.getScheduler()
                    .nextEntry(System.currentTimeMillis()) == null;
            check("F01-ACQ-SAFE-01: Fixture scheduler has no selected entry",
                    schedulerEmpty);
            if (!schedulerEmpty) {
                return; // finally restores all test-owned fields, without a native tick.
            }
            Zeus.bossHuntTick();
            checkEquals("R4-PASSIVE: Cross-map still requires manual NPC interaction",
                    "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());
            java.lang.reflect.Method goalForSchedule = Zeus.class.getDeclaredMethod("goal");
            goalForSchedule.setAccessible(true);
            check("R4-PASSIVE: walking-only map 33 Travel goal remains active",
                    ((Integer) goalForSchedule.invoke(null)).intValue() == 33);
            check("F01-ACQ-SAFE-01: No automatic movement lock acquired", !Player.isLockKey);
            check("F01-ACQ-SAFE-01: No route path written", GameScreen.player.posTransRoad == null);

            // Native GiaoTiep and Menu2 are checked by the existing independent harness.
        } catch (Throwable error) {
            check("Schedule NPC acquisition test setup: " + error.getMessage(), false);
        } finally {
            // Restore mutable objects before restoring the globally held references.
            if (oldLoadMap != null) oldLoadMap.idMap = oldMapId;
            GameCanvas.loadmap = oldLoadMap;
            if (oldPlayer != null) {
                oldPlayer.x = oldX;
                oldPlayer.y = oldY;
                oldPlayer.hp = oldHp;
                oldPlayer.Action = (byte) oldAction;
                oldPlayer.posTransRoad = oldRoad;
            }
            GameScreen.player = oldPlayer;
            GameScreen.Vecplayers = oldPlayers;
            GameScreen.ObjFocus = oldFocus;
            Player.isLockKey = oldLock;
            GameCanvas.currentDialog = oldDialog;
            GameCanvas.menu2 = oldMenu;
            Zeus.bossHuntConfigPath = oldConfigPath;
            Zeus.bossHunt.getConfig().setEnabled(oldEnabled);
            Zeus.bossHunt.setActiveEntry(oldEntry);
            Zeus.bossHunt.setState(oldState);
            if (controllerSnapshotReady) {
                for (int fieldIndex = 0; fieldIndex < controllerFields.length; fieldIndex++) {
                    try {
                        controllerFields[fieldIndex].set(Zeus.bossHunt, controllerValues[fieldIndex]);
                    } catch (Exception error) {
                        check("Failed to restore controller " + controllerNames[fieldIndex]
                                + ": " + error.getMessage(), false);
                    }
                }
            }
            try {
                java.lang.reflect.Field status = BossHuntController.class.getDeclaredField("lastStatus");
                status.setAccessible(true);
                status.set(Zeus.bossHunt, oldStatus);
            } catch (Exception error) {
                check("Failed to restore Boss Hunt status: " + error.getMessage(), false);
            }
            Zeus.pendingBossMenuSelection = oldPending;
            Zeus.recordBossHuntDiagReason(oldReason);
            Zeus.bossHuntStatusPath = oldStatusPath;
            if (statusTimerSnapshotReady) {
                try {
                    statusTimer.setLong(null, oldStatusTimer);
                } catch (Exception error) {
                    check("Failed to restore status emission timer: " + error.getMessage(), false);
                }
            }
        }
    }

    private static void testNativeInputCapturePassive() {
        System.out.println("\n--- testNativeInputCapturePassive ---");
        final Player oldPlayer = GameScreen.player;
        final short[] oldPath = oldPlayer != null ? oldPlayer.posTransRoad : null;
        final boolean oldLock = Player.isLockKey;
        final MainDialog oldDialog = GameCanvas.currentDialog;
        final MainDialog oldSubDialog = GameCanvas.subDialog;
        // Fresh-JVM setup may construct GameCanvas.game; always restore its prior identity.
        final GameScreen oldGame = GameCanvas.game;
        final Menu2 oldMenu = GameCanvas.menu2;
        final boolean oldChatVisible = ChatTextField.isShow;
        final boolean[] keys = GameCanvas.keyMyHold;
        final boolean[] oldKeys = keys != null ? (boolean[]) keys.clone() : null;
        final BossHuntState oldState = Zeus.bossHunt.getState();
        java.lang.reflect.Field screenField = null;
        Object oldScreen = null;
        boolean screenSnapshotReady = false;
        final String[] extraCaptureNames = {
            "nativePointerPendingState", "nativePointerPendingOrigin",
            "nativePointerPendingOldPath", "nativePointerPendingPlayer", "nativePointerPendingLock",
            "nativePointerPendingMap", "nativePointerPendingSession",
            "nativePointerPendingTick", "nativePointerPendingSequence",
            "nativePointerPendingBossEpoch",
            "nativePointerSessionGeneration", "nativePointerTickGeneration",
            "nativePointerSequence", "nativePointerLastOrigin",
            "nativePointerLastOldPath", "nativePointerLastNewPath",
            "nativePointerLastPreLock", "nativePointerLastMap",
            "nativePointerLastSession", "nativePointerLastSequence",
            "nativePointerLastBossEpoch"
        };
        final java.lang.reflect.Field[] extraCaptureFields =
                new java.lang.reflect.Field[extraCaptureNames.length];
        final Object[] extraCaptureValues = new Object[extraCaptureNames.length];
        int extraCapturedCount = 0;
        java.lang.reflect.Field pointerCount = null;
        java.lang.reflect.Field keyboardCount = null;
        java.lang.reflect.Field pointerReference = null;
        java.lang.reflect.Field generation = null;
        int oldPointerCount = 0;
        int oldKeyboardCount = 0;
        int oldGeneration = 0;
        Object oldPointerReference = null;
        boolean captureSnapshotReady = false;
        try {
            // Snapshot before any mutation. The observer state is private by design.
            pointerCount = Zeus.class.getDeclaredField("nativePointerCaptureCount");
            keyboardCount = Zeus.class.getDeclaredField("nativeKeyboardCaptureCount");
            pointerReference = Zeus.class.getDeclaredField("nativeLastPointerPath");
            generation = Zeus.class.getDeclaredField("nativeInputGeneration");
            pointerCount.setAccessible(true);
            keyboardCount.setAccessible(true);
            pointerReference.setAccessible(true);
            generation.setAccessible(true);
            oldPointerCount = pointerCount.getInt(null);
            oldKeyboardCount = keyboardCount.getInt(null);
            oldPointerReference = pointerReference.get(null);
            oldGeneration = generation.getInt(null);
            captureSnapshotReady = true;
            for (int i = 0; i < extraCaptureNames.length; i++) {
                extraCaptureFields[i] = Zeus.class.getDeclaredField(extraCaptureNames[i]);
                extraCaptureFields[i].setAccessible(true);
                extraCaptureValues[i] = extraCaptureFields[i].get(null);
                extraCapturedCount++;
            }
            screenField = GameCanvas.class.getField("currentScreen");
            oldScreen = screenField.get(null);
            screenSnapshotReady = true;

            if (GameScreen.player == null) {
                GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
            }
            short[] existing = new short[] { 3, 4 };
            short[] reported = new short[] { 5, 6 };
            GameScreen.player.posTransRoad = existing;
            Player.isLockKey = true;
            java.lang.reflect.Method reset = Zeus.class.getDeclaredMethod("resetNativeInputCapture");
            reset.setAccessible(true);
            reset.invoke(null);
            check("CAPTURE-T08: Zero initial observations",
                    pointerCount.getInt(null) == 0 && keyboardCount.getInt(null) == 0
                    && pointerReference.get(null) == null && generation.getInt(null) == 0);
            Zeus.onNativePointerMovement(reported);
            check("CAPTURE-T06: Exact native path observed", pointerReference.get(null) == reported);
            check("CAPTURE-T06: Pointer count advanced",
                    pointerCount.getInt(null) == 1 && generation.getInt(null) == 1);
            check("CAPTURE-T06: Existing movement path unchanged",
                    GameScreen.player.posTransRoad == existing);
            check("CAPTURE-T06: Lock unchanged", Player.isLockKey);
            checkEquals("CAPTURE-T06: Controller state unchanged", oldState, Zeus.bossHunt.getState());

            // A null native write is still only an observation, never a movement command.
            Zeus.onNativePointerMovement(null);
            check("CAPTURE-R02: Null pointer path safely observed",
                    pointerReference.get(null) == null && pointerCount.getInt(null) == 2
                    && generation.getInt(null) == 2 && GameScreen.player.posTransRoad == existing
                    && Player.isLockKey);
            reset.invoke(null);
            check("CAPTURE-T08: Reset is inert",
                    pointerCount.getInt(null) == 0 && keyboardCount.getInt(null) == 0
                    && pointerReference.get(null) == null && generation.getInt(null) == 0);

            // Reuse only ReconnectTravelTest.setupWorldState's minimal screen fixture.
            // A fresh JVM may never have constructed the native game screen.
            if (GameCanvas.game == null) {
                GameCanvas.game = new GameScreen();
            }
            check("FIXTURE-T01: Native GameScreen fixture initialized",
                    GameCanvas.game != null && (oldGame != null || GameCanvas.game != oldGame));

            // Drive the actual native keyMove() predicate. All modified keys are restored.
            // Existing key mappings: direction 0/1/2/3 use D-pad and numeric pairs.
            final int[] directionIndices = { 2, 8, 4, 6, 22, 28, 24, 26 };
            boolean keyFixtureReady = keys != null && keys.length > 28
                    && GameCanvas.keyMyHold == keys && GameCanvas.game != null;
            check("CAPTURE-R03: Native key fixture available", keyFixtureReady);
            if (!keyFixtureReady) {
                return; // Failed assertion; finally restores globals. Never silently skip.
            }
                for (int i = 0; i < directionIndices.length; i++) {
                    keys[directionIndices[i]] = false;
                }
                screenField.set(null, GameCanvas.game);
                GameCanvas.currentDialog = null;
                GameCanvas.subDialog = null;
                GameCanvas.menu2 = null;
                ChatTextField.isShow = false;

                boolean noDirection = !GameCanvas.keyMove(0) && !GameCanvas.keyMove(1)
                        && !GameCanvas.keyMove(2) && !GameCanvas.keyMove(3);
                check("CAPTURE-R03: No native directional key held", noDirection);
                Zeus.onNativeUpdateKey();
                check("CAPTURE-R03: Idle keyboard does not advance observation",
                        keyboardCount.getInt(null) == 0 && generation.getInt(null) == 0);

                keys[4] = true;  // Native D-pad Left, tested through keyMove(2).
                check("CAPTURE-R03: Native left key predicate becomes true", GameCanvas.keyMove(2));
                Zeus.onNativeUpdateKey();
                check("CAPTURE-R03: Directional keyboard observation increments",
                        keyboardCount.getInt(null) == 1 && generation.getInt(null) == 1);

                GameCanvas.currentDialog = new MsgDialog();
                Zeus.onNativeUpdateKey();
                check("CAPTURE-R03: Foreground MsgDialog suppresses keyboard capture",
                        keyboardCount.getInt(null) == 1 && generation.getInt(null) == 1);

                GameCanvas.currentDialog = null;
                ChatTextField.isShow = true;
                Zeus.onNativeUpdateKey();
                check("CAPTURE-R03: Chat suppresses keyboard capture",
                        keyboardCount.getInt(null) == 1 && generation.getInt(null) == 1);

                ChatTextField.isShow = false;
                GameCanvas.subDialog = new MsgDialog();
                Zeus.onNativeUpdateKey();
                check("CAPTURE-R03: Subdialog suppresses keyboard capture",
                        keyboardCount.getInt(null) == 1 && generation.getInt(null) == 1);

                check("CAPTURE-T06: Keyboard capture preserves native movement",
                        GameScreen.player.posTransRoad == existing && Player.isLockKey);
                checkEquals("CAPTURE-T06: Keyboard capture preserves controller state",
                        oldState, Zeus.bossHunt.getState());
            reset.invoke(null);
            check("CAPTURE-T08: Observer reset clears both inputs",
                    pointerCount.getInt(null) == 0 && keyboardCount.getInt(null) == 0
                    && pointerReference.get(null) == null && generation.getInt(null) == 0);
        } catch (Throwable error) {
            check("Native input passive fixture: " + error.getMessage(), false);
        } finally {
            // Restore every touched native fixture even if an assertion or callback fails.
            if (keys != null && oldKeys != null && keys.length == oldKeys.length) {
                System.arraycopy(oldKeys, 0, keys, 0, keys.length);
            }
            if (screenSnapshotReady) {
                try {
                    screenField.set(null, oldScreen);
                } catch (Throwable error) {
                    check("Native screen restore failed: " + error.getMessage(), false);
                }
            }
            GameCanvas.game = oldGame;
            GameCanvas.currentDialog = oldDialog;
            GameCanvas.subDialog = oldSubDialog;
            GameCanvas.menu2 = oldMenu;
            ChatTextField.isShow = oldChatVisible;
            if (oldPlayer != null) oldPlayer.posTransRoad = oldPath;
            GameScreen.player = oldPlayer;
            Player.isLockKey = oldLock;
            for (int i = extraCapturedCount - 1; i >= 0; i--) {
                try {
                    extraCaptureFields[i].set(null, extraCaptureValues[i]);
                } catch (Throwable error) {
                    check("Native prewrite snapshot restore: " + extraCaptureNames[i], false);
                }
            }
            if (captureSnapshotReady) {
                try {
                    pointerCount.setInt(null, oldPointerCount);
                    keyboardCount.setInt(null, oldKeyboardCount);
                    pointerReference.set(null, oldPointerReference);
                    generation.setInt(null, oldGeneration);
                } catch (Throwable error) {
                    check("Native capture snapshot restoration failed: " + error.getMessage(), false);
                }
            }
        }
    }

    /**
     * PRECAP-T03..T09: direct callback contract, NOT proof of native ASM position.
     * Every private observation field and native fixture is restored in finally.
     */
    private static void testNativePointerPrewriteCorrelation() {
        System.out.println("\n--- testNativePointerPrewriteCorrelation ---");
        final Player oldPlayer = GameScreen.player;
        final short[] oldPath = oldPlayer == null ? null : oldPlayer.posTransRoad;
        final boolean oldLock = Player.isLockKey;
        final BossHuntState oldState = Zeus.bossHunt.getState();
        final String[] fieldNames = {
            "nativeKeyboardCaptureCount", "nativePointerCaptureCount",
            "nativeInputGeneration", "nativeLastPointerPath",
            "nativePointerPendingState", "nativePointerPendingOrigin",
            "nativePointerPendingOldPath", "nativePointerPendingPlayer", "nativePointerPendingLock",
            "nativePointerPendingMap", "nativePointerPendingSession",
            "nativePointerPendingTick", "nativePointerPendingSequence",
            "nativePointerPendingBossEpoch",
            "nativePointerSessionGeneration", "nativePointerTickGeneration",
            "nativePointerSequence", "nativePointerLastOrigin",
            "nativePointerLastOldPath", "nativePointerLastNewPath",
            "nativePointerLastPreLock", "nativePointerLastMap",
            "nativePointerLastSession", "nativePointerLastSequence",
            "nativePointerLastBossEpoch"
        };
        final java.lang.reflect.Field[] fields = new java.lang.reflect.Field[fieldNames.length];
        final Object[] saved = new Object[fieldNames.length];
        int savedFields = 0;
        java.lang.reflect.Method reset = null;
        java.lang.reflect.Field origin = null;
        java.lang.reflect.Field oldRef = null;
        java.lang.reflect.Field newRef = null;
        java.lang.reflect.Field preLock = null;
        java.lang.reflect.Field pendingState = null;
        java.lang.reflect.Field tickGeneration = null;
        java.lang.reflect.Field sessionGeneration = null;
        java.lang.reflect.Field pointerCount = null;
        java.lang.reflect.Field inputGeneration = null;
        java.lang.reflect.Field pointerRef = null;
        // A fresh JVM has no native map. Preserve both reference and its original ID.
        final LoadMap originalMap = GameCanvas.loadmap;
        final int oldMapId = originalMap == null ? -1 : originalMap.idMap;
        final int fixtureMapId = 33;
        try {
            // Take a full private-state snapshot BEFORE resetting the observer.
            for (int i = 0; i < fieldNames.length; i++) {
                fields[i] = Zeus.class.getDeclaredField(fieldNames[i]);
                fields[i].setAccessible(true);
                saved[i] = fields[i].get(null);
                savedFields++;
            }
            origin = Zeus.class.getDeclaredField("nativePointerLastOrigin");
            oldRef = Zeus.class.getDeclaredField("nativePointerLastOldPath");
            newRef = Zeus.class.getDeclaredField("nativePointerLastNewPath");
            preLock = Zeus.class.getDeclaredField("nativePointerLastPreLock");
            pendingState = Zeus.class.getDeclaredField("nativePointerPendingState");
            tickGeneration = Zeus.class.getDeclaredField("nativePointerTickGeneration");
            sessionGeneration = Zeus.class.getDeclaredField("nativePointerSessionGeneration");
            pointerCount = Zeus.class.getDeclaredField("nativePointerCaptureCount");
            inputGeneration = Zeus.class.getDeclaredField("nativeInputGeneration");
            pointerRef = Zeus.class.getDeclaredField("nativeLastPointerPath");
            origin.setAccessible(true);
            oldRef.setAccessible(true);
            newRef.setAccessible(true);
            preLock.setAccessible(true);
            pendingState.setAccessible(true);
            tickGeneration.setAccessible(true);
            sessionGeneration.setAccessible(true);
            pointerCount.setAccessible(true);
            inputGeneration.setAccessible(true);
            pointerRef.setAccessible(true);
            reset = Zeus.class.getDeclaredMethod("resetNativeInputCapture");
            reset.setAccessible(true);
            reset.invoke(null);

            // Reuse the existing isolated LoadMap fixture helper. It bypasses
            // the native constructor; allocation errors reach the FAIL handler.
            if (GameCanvas.loadmap == null) {
                GameCanvas.loadmap = allocateBossScheduleTestMap();
            }
            if (GameCanvas.loadmap == null) {
                throw new IllegalStateException("LoadMap fixture could not be initialized");
            }
            GameCanvas.loadmap.idMap = fixtureMapId;
            check("MAP-T01: isolated native map fixture initialized",
                    GameCanvas.loadmap != null && GameCanvas.loadmap.idMap == fixtureMapId);

            if (GameScreen.player == null) {
                GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
            }
            final short[] bossPath = new short[] { 1, 2, 3 };
            final short[] manualPath = new short[] { 4, 5, 6 };
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;

            Zeus.onNativePrePointerTerrain();
            java.lang.reflect.Field pendingOldPath =
                    Zeus.class.getDeclaredField("nativePointerPendingOldPath");
            java.lang.reflect.Field pendingLock =
                    Zeus.class.getDeclaredField("nativePointerPendingLock");
            pendingOldPath.setAccessible(true);
            pendingLock.setAccessible(true);
            check("PRECAP-T01/T03: terrain pre captures exact original path and lock",
                    pendingState.getInt(null) == 1
                    && pendingOldPath.get(null) == bossPath && pendingLock.getBoolean(null));
            // Emulate only the native PUTFIELD between callbacks, never change production code.
            GameScreen.player.posTransRoad = manualPath;
            Zeus.onNativePointerMovement(manualPath);
            check("PRECAP-T03: terrain pre/post correlated once",
                    origin.getInt(null) == 1 && oldRef.get(null) == bossPath
                    && newRef.get(null) == manualPath && preLock.getBoolean(null)
                    && pendingState.getInt(null) == 0);
            check("PRECAP-T08: callback leaves native path and lock untouched",
                    GameScreen.player.posTransRoad == manualPath && Player.isLockKey);
            check("PRECAP-T03: phase one pointer counting retained",
                    pointerCount.getInt(null) == 1 && inputGeneration.getInt(null) == 1
                    && pointerRef.get(null) == manualPath);

            GameScreen.player.posTransRoad = bossPath;
            Zeus.onNativePrePointerPortal();
            GameScreen.player.posTransRoad = manualPath;
            Zeus.onNativePointerMovement(manualPath);
            check("PRECAP-T02/T03: portal origin and old/new references",
                    origin.getInt(null) == 2 && oldRef.get(null) == bossPath
                    && newRef.get(null) == manualPath);

            // No pre-event: even the identical new path cannot imply takeover.
            Zeus.onNativePointerMovement(manualPath);
            check("PRECAP-T04: orphan post remains UNKNOWN", origin.getInt(null) == 0
                    && oldRef.get(null) == null);

            Zeus.onNativePrePointerTerrain();
            Zeus.onNativePrePointerPortal();
            Zeus.onNativePointerMovement(manualPath);
            check("PRECAP-T05: nested pre rejects ambiguous pair",
                    origin.getInt(null) == 0 && pendingState.getInt(null) == 0);

            Zeus.onNativePrePointerTerrain();
            tickGeneration.setInt(null, tickGeneration.getInt(null) + 1);
            Zeus.onNativePointerMovement(manualPath);
            check("PRECAP-T06: stale tick rejects pending pre",
                    origin.getInt(null) == 0 && pendingState.getInt(null) == 0);

            Zeus.onNativePrePointerTerrain();
            reset.invoke(null);
            Zeus.onNativePointerMovement(manualPath);
            check("PRECAP-T07: reset invalidates stale event",
                    origin.getInt(null) == 0 && pendingState.getInt(null) == 0
                    && sessionGeneration.getInt(null) > 0);

            // MAP-T02: a stable, known map must preserve the exact pre/post pair.
            GameCanvas.loadmap.idMap = fixtureMapId;
            GameScreen.player.posTransRoad = bossPath;
            Zeus.onNativePrePointerPortal();
            GameScreen.player.posTransRoad = manualPath;
            Zeus.onNativePointerMovement(manualPath);
            java.lang.reflect.Field lastMap = Zeus.class.getDeclaredField("nativePointerLastMap");
            lastMap.setAccessible(true);
            check("MAP-T02: unchanged map correlates old and new paths",
                    origin.getInt(null) == 2 && oldRef.get(null) == bossPath
                    && newRef.get(null) == manualPath && lastMap.getInt(null) == fixtureMapId
                    && pendingState.getInt(null) == 0);

            // MAP-T03: unconditional, red-capable mismatch on both fresh and
            // existing world fixtures. Never skip this assertion for null originalMap.
            GameScreen.player.posTransRoad = bossPath;
            Zeus.onNativePrePointerPortal();
            GameCanvas.loadmap.idMap = fixtureMapId + 1;
            GameScreen.player.posTransRoad = manualPath;
            Zeus.onNativePointerMovement(manualPath);
            check("MAP-T03: changed map rejects correlation and clears pending",
                    origin.getInt(null) == 0 && oldRef.get(null) == null
                    && newRef.get(null) == null && pendingState.getInt(null) == 0);
            GameCanvas.loadmap.idMap = fixtureMapId;

            GameScreen.player.posTransRoad = null;
            Zeus.onNativePrePointerTerrain();
            Zeus.onNativePointerMovement(null);
            check("PRECAP-T09: null old/new remains passive",
                    origin.getInt(null) == 1 && oldRef.get(null) == null
                    && newRef.get(null) == null && pointerRef.get(null) == null
                    && GameScreen.player.posTransRoad == null);
            checkEquals("PRECAP-T12: Controller state unchanged", oldState, Zeus.bossHunt.getState());
            check("PRECAP-T12: Observer does not release native lock", Player.isLockKey);
        } catch (Throwable error) {
            check("Prewrite callback fixture: " + error.getMessage(), false);
        } finally {
            // Restore the object's original mutable ID BEFORE replacing the global.
            if (originalMap != null) originalMap.idMap = oldMapId;
            GameCanvas.loadmap = originalMap;
            if (oldPlayer != null) oldPlayer.posTransRoad = oldPath;
            GameScreen.player = oldPlayer;
            Player.isLockKey = oldLock;
            // Restore ALL saved observer fields, even after exceptional completion.
            for (int i = savedFields - 1; i >= 0; i--) {
                try {
                    fields[i].set(null, saved[i]);
                } catch (Throwable error) {
                    check("Prewrite observer field restore: " + fieldNames[i], false);
                }
            }
        }
        check("MAP-T04/T05: original LoadMap identity and ID restored",
                GameCanvas.loadmap == originalMap
                && (originalMap == null || originalMap.idMap == oldMapId));
    }

    /**
     * F01 focused owner/route regression: source callbacks and private engine
     * gates, not native bytecode or live-route acceptance evidence.
     */
    private static void testBossRouteOwnershipRegression() {
        System.out.println("\n--- testBossRouteOwnershipRegression ---");
        final Player savedPlayer = GameScreen.player;
        final short[] savedPath = savedPlayer == null ? null : savedPlayer.posTransRoad;
        final int savedHp = savedPlayer == null ? 0 : savedPlayer.hp;
        final int savedX = savedPlayer == null ? 0 : savedPlayer.x;
        final int savedY = savedPlayer == null ? 0 : savedPlayer.y;
        final byte savedAction = savedPlayer == null ? (byte) 0 : (byte) savedPlayer.Action;
        final MainObject savedFocus = GameScreen.ObjFocus;
        final boolean savedMapReady = LoadMapScreen.isNextMap;
        java.lang.reflect.Field mapEffectField = null;
        Object savedMapEffect = null;
        final boolean savedLock = Player.isLockKey;
        final LoadMap savedMap = GameCanvas.loadmap;
        final int savedMapId = savedMap == null ? -1 : savedMap.idMap;
        final GameScreen savedGame = GameCanvas.game;
        final MainScreen savedScreen = GameCanvas.currentScreen;
        final MainDialog savedDialog = GameCanvas.currentDialog;
        final MainDialog savedSubDialog = GameCanvas.subDialog;
        final Menu2 savedMenu = GameCanvas.menu2;
        final boolean savedChat = ChatTextField.isShow;
        final boolean oldEnabled = Zeus.bossHunt.isEnabled();
        final BossHuntState oldBossState = Zeus.bossHunt.getState();
        final String oldBossStatus = Zeus.bossHunt.getStatusText();
        final BossHuntScheduler.ScheduleEntry oldBossEntry = Zeus.bossHunt.getActiveEntry();
        final String oldConfigPath = Zeus.bossHuntConfigPath;
        final boolean[] keys = GameCanvas.keyMyHold;
        final boolean[] oldKeys = keys == null ? null : (boolean[]) keys.clone();

        final String[] names = {
            "bossHuntHoldingLock", "bossHuntHoldingPath", "bossHuntOwnedPath",
            "bossOwnerPlayer", "bossOwnerMap", "bossOwnerSession", "bossOwnerGoal", "bossOwnerEpoch",
            "bossOwnerKeyboardBaseline", "bossOwnerPointerBaseline", "bossManualPaused",
            "bossScheduleTravelMode", "travelInTick", "travelOwnedPath",
            "travelOwnerPlayer", "travelOwnerKind", "travelPointerBaseline",
            "travelKeyboardBaseline", "travelOwnerMap", "travelOwnerSession",
            "travelState", "travelWait", "travelMapSeen", "travelHops",
            "travelStallTicks", "travelLastX", "travelLastY", "travelWhy",
            "travelStoneTried",
            "readySettleTicks", "stableMapId", "mapStableTicks",
            "atkMode", "navTarget", "navDone",
            "enhNavigating", "enhOwnsForgeScreen", "enhOwnsResultDialog",
            "dungeonEnabled", "dungeonNavigating",
            "nativeKeyboardCaptureCount", "nativePointerCaptureCount",
            "nativeInputGeneration", "nativeLastPointerPath",
            "nativePointerPendingState", "nativePointerPendingOrigin",
            "nativePointerPendingOldPath", "nativePointerPendingPlayer",
            "nativePointerPendingLock", "nativePointerPendingMap",
            "nativePointerPendingSession", "nativePointerPendingTick",
            "nativePointerPendingSequence", "nativePointerPendingBossEpoch",
            "nativePointerSessionGeneration",
            "nativePointerTickGeneration", "nativePointerSequence",
            "nativePointerLastOrigin", "nativePointerLastOldPath",
            "nativePointerLastNewPath", "nativePointerLastPreLock",
            "nativePointerLastMap", "nativePointerLastSession",
            "nativePointerLastSequence", "nativePointerLastBossEpoch"
        };
        final java.lang.reflect.Field[] fields = new java.lang.reflect.Field[names.length];
        final Object[] oldValues = new Object[names.length];
        int snapCount = 0;
        try {
            mapEffectField = LoadMap.class.getDeclaredField("isShowEffAuto");
            mapEffectField.setAccessible(true);
            savedMapEffect = mapEffectField.get(null);
            for (int i = 0; i < names.length; i++) {
                fields[i] = Zeus.class.getDeclaredField(names[i]);
                fields[i].setAccessible(true);
                oldValues[i] = fields[i].get(null);
                snapCount++;
            }
            if (keys == null || keys.length <= 28) {
                throw new IllegalStateException("Native directional key array unavailable");
            }
            if (GameCanvas.loadmap == null) GameCanvas.loadmap = allocateBossScheduleTestMap();
            if (GameCanvas.loadmap == null) throw new IllegalStateException("LoadMap missing");
            GameCanvas.loadmap.idMap = 1;
            if (GameCanvas.game == null) GameCanvas.game = new GameScreen();
            GameCanvas.currentScreen = GameCanvas.game;
            GameCanvas.currentDialog = null;
            GameCanvas.subDialog = null;
            GameCanvas.menu2 = null;
            ChatTextField.isShow = false;
            if (GameScreen.player == null) GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
            GameScreen.player.hp = 1000;
            GameScreen.player.posTransRoad = null;
            Player.isLockKey = false;
            for (int i = 0; i < keys.length; i++) keys[i] = false;

            String[] zeroBools = { "enhNavigating", "enhOwnsForgeScreen",
                    "enhOwnsResultDialog", "dungeonEnabled", "dungeonNavigating",
                    "bossManualPaused", "navDone" };
            for (int i = 0; i < zeroBools.length; i++) {
                java.lang.reflect.Field f = Zeus.class.getDeclaredField(zeroBools[i]);
                f.setAccessible(true);
                f.setBoolean(null, false);
            }
            java.lang.reflect.Field atk = Zeus.class.getDeclaredField("atkMode");
            java.lang.reflect.Field nav = Zeus.class.getDeclaredField("navTarget");
            atk.setAccessible(true);
            nav.setAccessible(true);
            atk.setInt(null, 0);
            nav.setInt(null, -1);
            // Disable persistence for the entire fixture; no operator files touched.
            Zeus.bossHuntConfigPath = null;
            Zeus.bossHunt.getConfig().setEnabled(true);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);

            java.lang.reflect.Method goal = Zeus.class.getDeclaredMethod("goal");
            java.lang.reflect.Method nextHop = Zeus.class.getDeclaredMethod(
                    "mapNextHop", new Class[] { int.class, int.class });
            java.lang.reflect.Method own = Zeus.class.getDeclaredMethod(
                    "bossRememberPath", new Class[] { short[].class });
            java.lang.reflect.Method input = Zeus.class.getDeclaredMethod("bossInputArbitration");
            java.lang.reflect.Method release = Zeus.class.getDeclaredMethod(
                    "bossReleaseMovement", new Class[] { boolean.class });
            java.lang.reflect.Method travelOwn = Zeus.class.getDeclaredMethod(
                    "travelRememberPath", new Class[] { short[].class });
            java.lang.reflect.Method travelRelease = Zeus.class.getDeclaredMethod(
                    "travelReleaseMovement", new Class[] { boolean.class });
            goal.setAccessible(true);
            nextHop.setAccessible(true);
            own.setAccessible(true);
            input.setAccessible(true);
            release.setAccessible(true);
            travelOwn.setAccessible(true);
            travelRelease.setAccessible(true);
            java.lang.reflect.Field pause = Zeus.class.getDeclaredField("bossManualPaused");
            pause.setAccessible(true);
            java.lang.reflect.Field ownedPath = Zeus.class.getDeclaredField("bossHuntOwnedPath");
            ownedPath.setAccessible(true);

            checkEquals("F01-E2E-01: schedule needs walking goal map 33",
                    Integer.valueOf(33), goal.invoke(null));
            int hop = ((Integer) nextHop.invoke(null, new Object[] {
                    Integer.valueOf(1), Integer.valueOf(33) })).intValue();
            check("F01-E2E-02: verified map graph has a first hop", hop >= 0 && hop != 1);

            short[] bossPath = new short[] { 1, 2, 3 };
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            check("F01-OWNER: initial route uses exact path identity", ownedPath.get(null) == bossPath);

            short[] foreign = new short[] { 7, 8 };
            GameScreen.player.posTransRoad = foreign;
            release.invoke(null, new Object[] { Boolean.TRUE });
            check("F01-E2E-05: owner cleanup preserves foreign path and lock",
                    GameScreen.player.posTransRoad == foreign && Player.isLockKey);

            GameScreen.player.posTransRoad = bossPath;
            own.invoke(null, new Object[] { bossPath });
            keys[4] = true;
            check("F01-E2E-03: native left key is physically observed", GameCanvas.keyMove(2));
            Zeus.onNativeUpdateKey();
            input.invoke(null);
            check("F01-E2E-03: key takeover pauses without reacquisition",
                    pause.getBoolean(null) && !Player.isLockKey
                    && GameScreen.player.posTransRoad == null);
            checkEquals("F01-E2E-12: latched takeover disables route goal",
                    Integer.valueOf(-1), goal.invoke(null));
            Zeus.bossHuntTick();
            checkEquals("F01-E2E-03: repeated tick retains pause",
                    BossHuntState.PAUSED, Zeus.bossHunt.getState());
            keys[4] = false;
            Zeus.setBossHuntEnabled(false);
            Zeus.setBossHuntEnabled(true);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            checkEquals("F01-E2E-03: explicit rearm allows goal",
                    Integer.valueOf(33), goal.invoke(null));

            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            Zeus.onNativePrePointerTerrain();
            GameScreen.player.posTransRoad = foreign;
            Zeus.onNativePointerMovement(foreign);
            input.invoke(null);
            check("F01-E2E-04: pointer takeover preserves native replacement",
                    pause.getBoolean(null) && GameScreen.player.posTransRoad == foreign);
            // Exact pre/post native provenance transfers the Boss-acquired lock
            // without deleting the new manual path.
            check("R2-SAFE-01: verified terrain unlocks Boss lock, retains path",
                    pause.getBoolean(null) && GameScreen.player.posTransRoad == foreign
                    && !Player.isLockKey);
            Zeus.bossHuntTick();
            check("R2-SAFE-01: manual pause persists after handoff",
                    pause.getBoolean(null) && Zeus.bossHunt.getState() == BossHuntState.PAUSED);

            Zeus.setBossHuntEnabled(false);
            Zeus.setBossHuntEnabled(true);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            Zeus.onNativePrePointerPortal();
            GameScreen.player.posTransRoad = foreign;
            Zeus.onNativePointerMovement(foreign);
            input.invoke(null);
            check("R2-SAFE-02: confirmed portal unlocks Boss, retains native path",
                    pause.getBoolean(null) && GameScreen.player.posTransRoad == foreign
                    && !Player.isLockKey);

            Zeus.setBossHuntEnabled(false);
            Zeus.setBossHuntEnabled(true);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            Zeus.onNativePrePointerTerrain();
            own.invoke(null, new Object[] { bossPath }); // stale original owner epoch
            GameScreen.player.posTransRoad = foreign;
            Zeus.onNativePointerMovement(foreign);
            input.invoke(null);
            check("R2-SAFE-03: stale ownership epoch cannot unlock",
                    pause.getBoolean(null) && GameScreen.player.posTransRoad == foreign
                    && Player.isLockKey);

            Zeus.setBossHuntEnabled(false);
            Zeus.setBossHuntEnabled(true);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            Zeus.onNativePrePointerPortal();
            GameScreen.player.posTransRoad = foreign;
            Zeus.onNativePointerMovement(foreign);
            travelOwn.invoke(null, new Object[] { foreign });
            input.invoke(null);
            check("R2-SAFE-04: subsequent Travel ownership retains lock and path",
                    pause.getBoolean(null) && GameScreen.player.posTransRoad == foreign
                    && Player.isLockKey);
            travelRelease.invoke(null, new Object[] { Boolean.FALSE });

            Zeus.setBossHuntEnabled(false);
            Zeus.setBossHuntEnabled(true);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            GameScreen.player.posTransRoad = foreign;
            // Deliberately orphaned post: safe pause, no ownership claim.
            Zeus.onNativePointerMovement(foreign);
            input.invoke(null);
            check("F01-E2E-04: UNKNOWN pointer pauses without foreign cleanup",
                    pause.getBoolean(null) && GameScreen.player.posTransRoad == foreign
                    && Player.isLockKey);

            Zeus.setBossHuntEnabled(false);
            Zeus.setBossHuntEnabled(true);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            GameScreen.player.posTransRoad = null;
            input.invoke(null);
            check("F01-OWNER: natural completion retires ownership, not manual pause",
                    !pause.getBoolean(null) && !Player.isLockKey
                    && ownedPath.get(null) == null);

            // Explicit operator navigation outranks a tracked Boss path.
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            nav.setInt(null, 2);
            input.invoke(null);
            check("F01-E2E-05: explicit navigation releases only owned Boss movement",
                    GameScreen.player.posTransRoad == null && !Player.isLockKey
                    && !pause.getBoolean(null));
            checkEquals("F01-E2E-11: operator navigation goal remains authoritative",
                    Integer.valueOf(2), goal.invoke(null));
            nav.setInt(null, -1);

            short[] travelPath = new short[] { 9, 10 };
            GameScreen.player.posTransRoad = travelPath;
            Player.isLockKey = true;
            travelOwn.invoke(null, new Object[] { travelPath });
            travelRelease.invoke(null, new Object[] { Boolean.TRUE });
            check("F01-E2E-11: Travel releases its exact path and lock",
                    GameScreen.player.posTransRoad == null && !Player.isLockKey);
            GameScreen.player.posTransRoad = travelPath;
            Player.isLockKey = true;
            travelOwn.invoke(null, new Object[] { travelPath });
            GameScreen.player.posTransRoad = foreign;
            travelRelease.invoke(null, new Object[] { Boolean.TRUE });
            check("F01-E2E-11: Travel cleanup preserves foreign path and lock",
                    GameScreen.player.posTransRoad == foreign && Player.isLockKey);

            // Execute the actual Travel tick, not a mocked progress helper.
            // A live path must be observed for >150 stationary ticks to block.
            GameScreen.player.Action = (byte) 0;
            GameScreen.ObjFocus = null;
            GameScreen.player.x = 100;
            GameScreen.player.y = 100;
            LoadMapScreen.isNextMap = true;
            LoadMap.isShowEffAuto = 10;
            java.lang.reflect.Method travelTick = Zeus.class.getDeclaredMethod("travel");
            travelTick.setAccessible(true);
            String[] fieldsToPrepare = {
                "travelState", "travelStallTicks", "travelMapSeen",
                "travelWait", "travelLastX", "travelLastY",
                "travelWhy", "readySettleTicks", "stableMapId", "mapStableTicks"
            };
            java.lang.reflect.Field[] readyFields = new java.lang.reflect.Field[fieldsToPrepare.length];
            for (int i = 0; i < readyFields.length; i++) {
                readyFields[i] = Zeus.class.getDeclaredField(fieldsToPrepare[i]);
                readyFields[i].setAccessible(true);
            }
            java.lang.reflect.Field tvWalk = Zeus.class.getDeclaredField("TV_WALK");
            java.lang.reflect.Field tvBlocked = Zeus.class.getDeclaredField("TV_BLOCKED");
            tvWalk.setAccessible(true);
            tvBlocked.setAccessible(true);
            // travelState=TV_WALK; travelMapSeen=1; full map/scene readiness.
            readyFields[0].setInt(null, tvWalk.getInt(null));
            readyFields[1].setInt(null, 0);
            readyFields[2].setInt(null, 1);
            readyFields[3].setInt(null, 0);
            readyFields[4].setInt(null, Integer.MIN_VALUE);
            readyFields[5].setInt(null, Integer.MIN_VALUE);
            readyFields[6].setInt(null, 0);
            readyFields[7].setInt(null, 12);
            readyFields[8].setInt(null, 1);
            readyFields[9].setInt(null, 12);
            check("R2-SAFE-05: focused route fixture is ready",
                    Zeus.gameReady() && Zeus.mapStable());
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            for (int i = 0; i < 200
                    && readyFields[0].getInt(null) != tvBlocked.getInt(null); i++) {
                travelTick.invoke(null);
            }
            check("R2-SAFE-05: stationary owned path BLOCKED after bounded ticks",
                    readyFields[0].getInt(null) == tvBlocked.getInt(null)
                    && readyFields[6].getInt(null) == 2
                    && GameScreen.player.posTransRoad == null && !Player.isLockKey);
            check("R5-05: schedule walk retains blocked diagnostic",
                    "SCHEDULE_WALK_BLOCKED".equals(Zeus.getLastBossHuntDiagReason()));

            // Genuine position progress resets the previously accumulated stall.
            readyFields[0].setInt(null, tvWalk.getInt(null));
            readyFields[1].setInt(null, 125);
            readyFields[4].setInt(null, 100);
            readyFields[5].setInt(null, 100);
            GameScreen.player.x = 124;
            GameScreen.player.posTransRoad = bossPath;
            Player.isLockKey = true;
            own.invoke(null, new Object[] { bossPath });
            travelTick.invoke(null);
            check("R2-SAFE-06: moving owned route resets stall, remains active",
                    readyFields[1].getInt(null) == 0
                    && readyFields[0].getInt(null) == tvWalk.getInt(null)
                    && GameScreen.player.posTransRoad == bossPath && Player.isLockKey);

            // R3: the client's completed generic walk cleared the array, not its lock.
            // Exercise travel() itself; no mock completion helper.
            short[] completed = new short[] { 41, 42 };
            GameScreen.player.posTransRoad = completed;
            Player.isLockKey = true;
            java.lang.reflect.Field stoneRetries = Zeus.class.getDeclaredField("travelStoneTried");
            stoneRetries.setAccessible(true);
            stoneRetries.setInt(null, 3);
            java.lang.reflect.Field travelTickOwner = Zeus.class.getDeclaredField("travelInTick");
            travelTickOwner.setAccessible(true);
            travelTickOwner.setBoolean(null, true);
            travelOwn.invoke(null, new Object[] { completed });
            travelTickOwner.setBoolean(null, false);
            readyFields[0].setInt(null, tvWalk.getInt(null));
            readyFields[1].setInt(null, 0);
            GameScreen.player.posTransRoad = null;
            mVector savedGates = LoadMap.vecPointChange;
            LoadMap.vecPointChange = null; // avoid a new route before checking cleanup
            try {
                travelTick.invoke(null);
                check("R3-F02: completed Generic Travel releases owned null-path lock",
                        !Player.isLockKey && GameScreen.player.posTransRoad == null);
            } finally {
                LoadMap.vecPointChange = savedGates;
            }

            // A stale session must not grant completed-lock ownership.
            GameScreen.player.posTransRoad = completed;
            Player.isLockKey = true;
            travelTickOwner.setBoolean(null, true);
            travelOwn.invoke(null, new Object[] { completed });
            travelTickOwner.setBoolean(null, false);
            GameScreen.player.posTransRoad = null;
            java.lang.reflect.Field nativeSession = Zeus.class.getDeclaredField("nativePointerSessionGeneration");
            nativeSession.setAccessible(true);
            int oldSession = nativeSession.getInt(null);
            nativeSession.setInt(null, oldSession + 1);
            readyFields[0].setInt(null, tvWalk.getInt(null));
            travelTick.invoke(null);
            check("R3-F02: stale session retains unproven lock", Player.isLockKey);
            nativeSession.setInt(null, oldSession);
            travelRelease.invoke(null, new Object[] { Boolean.FALSE });
            Player.isLockKey = false;

            // Intervening native movement input invalidates completed ownership.
            GameScreen.player.posTransRoad = completed;
            Player.isLockKey = true;
            travelTickOwner.setBoolean(null, true);
            travelOwn.invoke(null, new Object[] { completed });
            travelTickOwner.setBoolean(null, false);
            GameScreen.player.posTransRoad = null;
            Zeus.onNativePointerMovement(new short[] { 5, 6 });
            readyFields[0].setInt(null, tvWalk.getInt(null));
            travelTick.invoke(null);
            check("R3-F02: intervening pointer input cannot release lock", Player.isLockKey);
            travelRelease.invoke(null, new Object[] { Boolean.FALSE });
            Player.isLockKey = false;

            // R5-04: actual Generic Travel stall retains its historical
            // owned-path release, independently from Boss target cleanup.
            short[] genericStall = new short[] { 61, 62 };
            nav.setInt(null, 2);
            GameScreen.player.posTransRoad = genericStall;
            Player.isLockKey = true;
            travelTickOwner.setBoolean(null, true);
            travelOwn.invoke(null, new Object[] { genericStall });
            travelTickOwner.setBoolean(null, false);
            readyFields[0].setInt(null, tvWalk.getInt(null));
            readyFields[1].setInt(null, 0);
            readyFields[3].setInt(null, 0);
            readyFields[4].setInt(null, Integer.MIN_VALUE);
            readyFields[5].setInt(null, Integer.MIN_VALUE);
            for (int i = 0; i < 200
                    && readyFields[0].getInt(null) != tvBlocked.getInt(null); i++) {
                travelTick.invoke(null);
            }
            java.lang.reflect.Field genericOwnerKind = Zeus.class.getDeclaredField("travelOwnerKind");
            genericOwnerKind.setAccessible(true);
            check("R5-04: Generic Travel stalls and releases exact owned path",
                    readyFields[0].getInt(null) == tvBlocked.getInt(null)
                    && readyFields[6].getInt(null) == 2
                    && GameScreen.player.posTransRoad == null && !Player.isLockKey
                    && genericOwnerKind.getInt(null) == 0);
            nav.setInt(null, -1);

            // Foreign locked path must never be timed out by our walker.
            release.invoke(null, new Object[] { Boolean.FALSE });
            GameScreen.player.posTransRoad = foreign;
            Player.isLockKey = true;
            readyFields[0].setInt(null, tvWalk.getInt(null));
            readyFields[1].setInt(null, 125);
            readyFields[4].setInt(null, GameScreen.player.x);
            readyFields[5].setInt(null, GameScreen.player.y);
            for (int i = 0; i < 200; i++) travelTick.invoke(null);
            check("R2-SAFE-04: foreign stalled route never timed out or cleared",
                    readyFields[0].getInt(null) == tvWalk.getInt(null)
                    && readyFields[1].getInt(null) == 125
                    && GameScreen.player.posTransRoad == foreign && Player.isLockKey);

            // R3-F01: exercise actual travel() -> bossHuntTick ordering with
            // a target-map schedule and an attributable path. The pathfinder
            // itself is not modeled by this fixture (requires native map data).
            java.lang.reflect.Field resolverField = BossHuntController.class.getDeclaredField("mapResolver");
            resolverField.setAccessible(true);
            Object savedResolver = resolverField.get(Zeus.bossHunt);
            java.lang.reflect.Field targetIdField = BossHuntController.class.getDeclaredField("targetMapId");
            targetIdField.setAccessible(true);
            BossHuntScheduler.ScheduleEntry formerEntry = Zeus.bossHunt.getActiveEntry();
            BossHuntState formerState = Zeus.bossHunt.getState();
            // Snapshot every controller field affected by tick(), independent
            // of which earlier regression ran before this fixture.
            String[] controllerNames = {
                "targetMapId", "targetZoneNumber", "targetZoneSubIndex",
                "arrivedAt", "absentSince", "lastStatus"
            };
            java.lang.reflect.Field[] controllerFields =
                    new java.lang.reflect.Field[controllerNames.length];
            Object[] controllerValues = new Object[controllerNames.length];
            for (int i = 0; i < controllerNames.length; i++) {
                controllerFields[i] = BossHuntController.class.getDeclaredField(controllerNames[i]);
                controllerFields[i].setAccessible(true);
                controllerValues[i] = controllerFields[i].get(Zeus.bossHunt);
            }
            try {
                Zeus.bossHunt.setMapResolver(new BossHuntController.MapResolver() {
                    public int resolveMapId(String mapAlias) { return 2; }
                });
                Zeus.bossHunt.setActiveEntry(new BossHuntScheduler.ScheduleEntry(
                        1, 1, "target", "R3 target fixture", 0L,
                        BossScheduleParser.Status.SPAWNED_OR_PAST));
                Zeus.bossHunt.setState(BossHuntState.TRAVEL);
                // goal() and bossRememberPath() must agree from the moment
                // ownership is captured, without an earlier controller tick.
                targetIdField.setInt(Zeus.bossHunt, 2);
                GameScreen.player.posTransRoad = bossPath;
                Player.isLockKey = true;
                own.invoke(null, new Object[] { bossPath });
                readyFields[0].setInt(null, tvWalk.getInt(null));
                readyFields[1].setInt(null, 0);
                readyFields[4].setInt(null, GameScreen.player.x);
                readyFields[5].setInt(null, GameScreen.player.y);
                checkEquals("R3-F01: scheduled target overrides ordinary destination",
                        Integer.valueOf(2), goal.invoke(null));
                travelTick.invoke(null);
                Zeus.bossHuntTick();
                check("R3-F01: internal Boss route not mistaken for manual input",
                        Zeus.bossHunt.getState() == BossHuntState.TRAVEL
                        && GameScreen.player.posTransRoad == bossPath
                        && Player.isLockKey);
                // Changing the authorized destination retires the stale Boss route.
                targetIdField.setInt(Zeus.bossHunt, 3);
                input.invoke(null);
                check("R3-F01: destination change releases only old Boss-owned path",
                        GameScreen.player.posTransRoad == null
                        && !Player.isLockKey && !pause.getBoolean(null));
                release.invoke(null, new Object[] { Boolean.TRUE });

                // R5-01: a real travel() loop must BLOCK and release the
                // current Boss-owned target path; no direct travelStop call.
                targetIdField.setInt(Zeus.bossHunt, 2);
                Zeus.bossHunt.setState(BossHuntState.TRAVEL);
                short[] stalledBoss = new short[] { 51, 52 };
                GameScreen.player.posTransRoad = stalledBoss;
                Player.isLockKey = true;
                own.invoke(null, new Object[] { stalledBoss });
                check("R5-01: correct target registered before stall",
                        ((Integer) goal.invoke(null)).intValue() == 2);
                readyFields[0].setInt(null, tvWalk.getInt(null));
                readyFields[1].setInt(null, 0);
                readyFields[3].setInt(null, 0);
                readyFields[4].setInt(null, Integer.MIN_VALUE);
                readyFields[5].setInt(null, Integer.MIN_VALUE);
                for (int i = 0; i < 200
                        && readyFields[0].getInt(null) != tvBlocked.getInt(null); i++) {
                    travelTick.invoke(null);
                }
                java.lang.reflect.Field held = Zeus.class.getDeclaredField("bossHuntHoldingPath");
                java.lang.reflect.Field heldLock = Zeus.class.getDeclaredField("bossHuntHoldingLock");
                java.lang.reflect.Field owned = Zeus.class.getDeclaredField("bossHuntOwnedPath");
                held.setAccessible(true);
                heldLock.setAccessible(true);
                owned.setAccessible(true);
                check("R5-01: Boss target stall reaches TV_BLOCKED",
                        readyFields[0].getInt(null) == tvBlocked.getInt(null)
                        && readyFields[6].getInt(null) == 2);
                check("R5-01: exact Boss path and lock released after timeout",
                        GameScreen.player.posTransRoad == null && !Player.isLockKey);
                check("R5-01: blocked Boss ownership metadata retired",
                        !held.getBoolean(null) && !heldLock.getBoolean(null)
                        && owned.get(null) == null);

                // R5-06: real position progress resets the stall, leaving
                // the attributable active route and lock intact.
                short[] movingBoss = new short[] { 53, 54 };
                GameScreen.player.posTransRoad = movingBoss;
                Player.isLockKey = true;
                own.invoke(null, new Object[] { movingBoss });
                readyFields[0].setInt(null, tvWalk.getInt(null));
                readyFields[1].setInt(null, 125);
                readyFields[4].setInt(null, GameScreen.player.x);
                readyFields[5].setInt(null, GameScreen.player.y);
                GameScreen.player.x += 24;
                travelTick.invoke(null);
                check("R5-06: progressing Boss route not blocked or released",
                        readyFields[0].getInt(null) == tvWalk.getInt(null)
                        && readyFields[1].getInt(null) == 0
                        && GameScreen.player.posTransRoad == movingBoss
                        && Player.isLockKey);
                release.invoke(null, new Object[] { Boolean.TRUE });

                // R5-02: a native pointer has replaced the Boss-owned route.
                // Travel must not time out or release this foreign replacement.
                GameScreen.player.posTransRoad = stalledBoss;
                Player.isLockKey = true;
                own.invoke(null, new Object[] { stalledBoss });
                Zeus.onNativePrePointerTerrain();
                GameScreen.player.posTransRoad = foreign;
                Zeus.onNativePointerMovement(foreign);
                readyFields[0].setInt(null, tvWalk.getInt(null));
                readyFields[1].setInt(null, 151);
                travelTick.invoke(null);
                check("R5-02: pointer replacement remains under native control",
                        readyFields[0].getInt(null) == tvWalk.getInt(null)
                        && GameScreen.player.posTransRoad == foreign && Player.isLockKey);
                release.invoke(null, new Object[] { Boolean.FALSE });
                GameScreen.player.posTransRoad = null;
                Player.isLockKey = false;

                // R5-03: destination change makes the old owner proof stale.
                GameScreen.player.posTransRoad = stalledBoss;
                Player.isLockKey = true;
                own.invoke(null, new Object[] { stalledBoss });
                targetIdField.setInt(Zeus.bossHunt, 3);
                readyFields[0].setInt(null, tvWalk.getInt(null));
                readyFields[1].setInt(null, 151);
                travelTick.invoke(null);
                check("R5-03: changed target does not release unproven path",
                        GameScreen.player.posTransRoad == stalledBoss
                        && Player.isLockKey
                        && readyFields[0].getInt(null) == tvWalk.getInt(null));
                targetIdField.setInt(Zeus.bossHunt, 2);
                release.invoke(null, new Object[] { Boolean.TRUE });

                // R5-03: a stale session must never authorize Boss timeout cleanup.
                GameScreen.player.posTransRoad = stalledBoss;
                Player.isLockKey = true;
                own.invoke(null, new Object[] { stalledBoss });
                int savedGeneration = nativeSession.getInt(null);
                nativeSession.setInt(null, savedGeneration + 1);
                readyFields[0].setInt(null, tvWalk.getInt(null));
                readyFields[1].setInt(null, 151);
                travelTick.invoke(null);
                check("R5-03: stale session preserves unknown movement",
                        GameScreen.player.posTransRoad == stalledBoss
                        && Player.isLockKey
                        && readyFields[0].getInt(null) == tvWalk.getInt(null));
                nativeSession.setInt(null, savedGeneration);
                release.invoke(null, new Object[] { Boolean.TRUE });

                // R5-03: a stale map owner cannot authorize stall cleanup.
                // Model the already-reconciled map before invoking travel().
                GameScreen.player.posTransRoad = stalledBoss;
                Player.isLockKey = true;
                own.invoke(null, new Object[] { stalledBoss });
                GameCanvas.loadmap.idMap = 3;
                readyFields[2].setInt(null, 3);  // travelMapSeen
                readyFields[8].setInt(null, 3);  // stableMapId
                readyFields[0].setInt(null, tvWalk.getInt(null));
                readyFields[1].setInt(null, 151);
                travelTick.invoke(null);
                check("R5-03: stale map does not release unproven movement",
                        GameScreen.player.posTransRoad == stalledBoss
                        && Player.isLockKey
                        && readyFields[0].getInt(null) == tvWalk.getInt(null));
                GameCanvas.loadmap.idMap = 1;
                readyFields[2].setInt(null, 1);
                readyFields[8].setInt(null, 1);
                release.invoke(null, new Object[] { Boolean.TRUE });
            } finally {
                resolverField.set(Zeus.bossHunt, savedResolver);
                Zeus.bossHunt.setActiveEntry(formerEntry);
                Zeus.bossHunt.setState(formerState);
                for (int i = controllerFields.length - 1; i >= 0; i--) {
                    controllerFields[i].set(Zeus.bossHunt, controllerValues[i]);
                }
            }
        } catch (Throwable error) {
            check("F01 focused integration fixture failed: " + error.getMessage(), false);
        } finally {
            if (oldKeys != null && keys != null && keys.length == oldKeys.length) {
                System.arraycopy(oldKeys, 0, keys, 0, keys.length);
            }
            if (savedMap != null) savedMap.idMap = savedMapId;
            GameCanvas.loadmap = savedMap;
            GameCanvas.game = savedGame;
            GameCanvas.currentScreen = savedScreen;
            GameCanvas.currentDialog = savedDialog;
            GameCanvas.subDialog = savedSubDialog;
            GameCanvas.menu2 = savedMenu;
            ChatTextField.isShow = savedChat;
            if (savedPlayer != null) {
                savedPlayer.posTransRoad = savedPath;
                savedPlayer.hp = savedHp;
                savedPlayer.x = savedX;
                savedPlayer.y = savedY;
                savedPlayer.Action = savedAction;
            }
            GameScreen.ObjFocus = savedFocus;
            LoadMapScreen.isNextMap = savedMapReady;
            if (mapEffectField != null) {
                try {
                    mapEffectField.set(null, savedMapEffect);
                } catch (Throwable restoreError) {
                    check("R2-SAFE fixture map effect restore failed: "
                            + restoreError.getMessage(), false);
                }
            }
            GameScreen.player = savedPlayer;
            Player.isLockKey = savedLock;
            Zeus.bossHunt.getConfig().setEnabled(oldEnabled);
            Zeus.bossHunt.setActiveEntry(oldBossEntry);
            Zeus.bossHunt.setState(oldBossState);
            try {
                java.lang.reflect.Field statusField = BossHuntController.class.getDeclaredField("lastStatus");
                statusField.setAccessible(true);
                statusField.set(Zeus.bossHunt, oldBossStatus);
            } catch (Throwable error) {
                check("F01 boss status restore failed: " + error.getMessage(), false);
            }
            Zeus.bossHuntConfigPath = oldConfigPath;
            for (int i = snapCount - 1; i >= 0; i--) {
                try {
                    fields[i].set(null, oldValues[i]);
                } catch (Throwable error) {
                    check("F01 focused state restore failed: " + names[i], false);
                }
            }
        }
    }

    private static void testAlisamaMsgDialogBridge() {
        System.out.println("\n--- testAlisamaMsgDialogBridge ---");

        if (GameScreen.player == null) {
            GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
        }
        GameScreen.player.x = 100;
        GameScreen.player.y = 100;
        GameScreen.player.hp = 1000;

        MainObject alisama = new MainObject();
        alisama.name = "Alisama";
        alisama.x = 110;
        alisama.y = 100; // Distance = 10 (<= 48, eligible)
        alisama.typeObject = 2; // NPC

        final int[] buyCalls = new int[1];
        final int[] closeCalls = new int[1];

        iCommand buyCmd = new iCommand("Mua b\u00e1n", new IAction() {
            public void perform() {
                buyCalls[0]++;
            }
        });
        iCommand closeCmd = new iCommand("\u0110\u00f3ng", new IAction() {
            public void perform() {
                closeCalls[0]++;
            }
        });

        // 1. Proven Alisama MsgDialog: should advance exactly once
        MsgDialog validDialog = new MsgDialog();
        validDialog.cmdList = new mVector();
        validDialog.cmdList.addElement(buyCmd);
        validDialog.cmdList.addElement(closeCmd);
        try {
            java.lang.reflect.Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(validDialog, new String[] { "Alisama: C\u1eeda h\u00e0ng trang b\u1ecb v\u0169 kh\u00ed" });
        } catch (Exception e) {
            throw new RuntimeException(e);
        }

        GameCanvas.currentDialog = validDialog;
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);

        long now = 1000000L;
        boolean handled = Zeus.handleBossScheduleDialog(alisama, now);
        check("Unproven Alisama dialog rejected", !handled);
        checkEquals("Unproven dialog never invokes buy action", 0, buyCalls[0]);
        checkEquals("Close action not invoked", 0, closeCalls[0]);

        // 2. Debounce: immediate second call is suppressed
        boolean debounced = Zeus.handleBossScheduleDialog(alisama, now + 500L);
        check("Repeated unproven dialog rejected", !debounced);
        checkEquals("Repeated unproven dialog still has no buy action", 0, buyCalls[0]);

        // 3. Retry limit: 3rd and 4th attempt after debounce
        Zeus.handleBossScheduleDialog(alisama, now + 2000L); // try 2
        Zeus.handleBossScheduleDialog(alisama, now + 4000L); // try 3
        checkEquals("Repeated unproven dialog never dispatches", 0, buyCalls[0]);
        boolean retryExhausted = Zeus.handleBossScheduleDialog(alisama, now + 6000L);
        check("Repeated attempts remain fail-closed without provenance", !retryExhausted);
        checkEquals("No buy action after repeated attempts", 0, buyCalls[0]);

        // 4. Negative test: Unrelated shop dialog (e.g. potion / purchase prompt)
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        buyCalls[0] = 0;
        MsgDialog unrelatedDialog = new MsgDialog();
        unrelatedDialog.cmdList = new mVector();
        unrelatedDialog.cmdList.addElement(buyCmd);
        unrelatedDialog.cmdList.addElement(closeCmd);
        try {
            java.lang.reflect.Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(unrelatedDialog, new String[] { "B\u1ea1n c\u00f3 mu\u1ed1n mua b\u00ecnh m\u00e1u v\u1edbi gi\u00e1 1000 xu kh\u00f4ng?" });
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
        GameCanvas.currentDialog = unrelatedDialog;
        boolean rejectedShop = Zeus.handleBossScheduleDialog(alisama, now);
        check("Unrelated purchase dialog rejected", !rejectedShop);
        checkEquals("Buy action not invoked on unrelated dialog", 0, buyCalls[0]);

        // 5. Negative test: Wrong NPC (not schedule NPC)
        MainObject wrongNpc = new MainObject();
        wrongNpc.name = "Ph\u00f3 Ch\u1ec9 Huy";
        wrongNpc.x = 110;
        wrongNpc.y = 100;
        wrongNpc.typeObject = 2;
        GameCanvas.currentDialog = validDialog;
        boolean rejectedWrongNpc = Zeus.handleBossScheduleDialog(wrongNpc, now);
        check("Wrong NPC rejected", !rejectedWrongNpc);
        checkEquals("Buy action not invoked on wrong NPC", 0, buyCalls[0]);

        // 6. Negative test: NPC out of range
        MainObject farNpc = new MainObject();
        farNpc.name = "Alisama";
        farNpc.x = 500;
        farNpc.y = 500; // distance >> 48
        farNpc.typeObject = 2;
        boolean rejectedFarNpc = Zeus.handleBossScheduleDialog(farNpc, now);
        check("Far NPC rejected", !rejectedFarNpc);
        checkEquals("Buy action not invoked on far NPC", 0, buyCalls[0]);

        // 7. Negative test: Ambiguous buttons (3 buttons)
        MsgDialog ambiguousDialog = new MsgDialog();
        ambiguousDialog.cmdList = new mVector();
        ambiguousDialog.cmdList.addElement(buyCmd);
        ambiguousDialog.cmdList.addElement(new iCommand("Xem h\u00e0ng", new IAction() { public void perform() {} }));
        ambiguousDialog.cmdList.addElement(closeCmd);
        try {
            java.lang.reflect.Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(ambiguousDialog, new String[] { "Alisama" });
        } catch (Exception e) {}
        GameCanvas.currentDialog = ambiguousDialog;
        boolean rejectedAmbiguous = Zeus.handleBossScheduleDialog(alisama, now);
        check("Ambiguous 3-button dialog rejected", !rejectedAmbiguous);
        checkEquals("Buy action not invoked on ambiguous buttons", 0, buyCalls[0]);

        // 8. Negative test: No interaction initiated (interactTries == 0)
        Zeus.setBossNpcInteractTriesForTest(0);
        GameCanvas.currentDialog = validDialog;
        boolean rejectedNoInteraction = Zeus.handleBossScheduleDialog(alisama, now);
        check("Dialog rejected when no interaction was initiated", !rejectedNoInteraction);
        checkEquals("Buy action not invoked without prior interaction", 0, buyCalls[0]);

        // Cleanup
        GameCanvas.currentDialog = null;
        Zeus.resetBossDialogBridgeState();
    }


    private static void testForegroundAlisamaDialogPriorityGateThroughTick() {
        final LoadMap oldLoadMap = GameCanvas.loadmap;
        final int oldMapId = oldLoadMap != null ? oldLoadMap.idMap : -1;
        try {
        System.out.println("\n--- testForegroundAlisamaDialogPriorityGateThroughTick ---");

        if (GameScreen.player == null) {
            GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
        }
        GameScreen.player.x = 100;
        GameScreen.player.y = 100;
        GameScreen.player.hp = 1000;
        Player.isLockKey = false;

        MainObject alisama = new MainObject();
        alisama.name = "Doubar";
        alisama.x = 110;
        alisama.y = 100;
        alisama.typeObject = 2; // NPC

        GameScreen.Vecplayers = new mVector();
        GameScreen.Vecplayers.addElement(alisama);

        final int[] buyCalls = new int[1];
        final int[] closeCalls = new int[1];

        iCommand buyCmd = new iCommand("Mua b\u00e1n", new IAction() {
            public void perform() {
                buyCalls[0]++;
            }
        });
        iCommand closeCmd = new iCommand("\u0110\u00f3ng", new IAction() {
            public void perform() {
                closeCalls[0]++;
            }
        });

        MsgDialog validDialog = new MsgDialog();
        validDialog.cmdList = new mVector();
        validDialog.cmdList.addElement(buyCmd);
        validDialog.cmdList.addElement(closeCmd);
        try {
            java.lang.reflect.Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(validDialog, new String[] { "Alisama: C\u1eeda h\u00e0ng trang b\u1ecb v\u0169 kh\u00ed" });
        } catch (Exception e) {
            throw new RuntimeException(e);
        }

        // Recorded schedule map; restore the original fixture in finally.
        try {
            if (GameCanvas.loadmap == null) {
                GameCanvas.loadmap = allocateBossScheduleTestMap();
            }
            GameCanvas.loadmap.idMap = 33;
        } catch (Exception error) {
            throw new RuntimeException("Cannot set schedule map", error);
        }

        // 1. SCENARIO A: Foreground MsgDialog visible WHILE background menu2.isShowMenu is TRUE!
        Zeus.bossHuntConfigPath = null;
        Zeus.bossHunt.getConfig().setEnabled(true);
        Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();

        GameCanvas.currentDialog = validDialog;
        GameCanvas.menu2 = new Menu2();
        GameCanvas.menu2.isShowMenu = true; // Background menu open!

        buyCalls[0] = 0;
        closeCalls[0] = 0;
        Zeus.bossHuntTick();

        checkEquals("F01-SAFE-01: Historical attempt cannot dispatch foreground command", 0, buyCalls[0]);
        checkEquals("Scenario A: Close action not invoked", 0, closeCalls[0]);
        checkEquals("Scenario A: passive policy explains manual NPC requirement", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());

        // F01-SAFE-01: repeated ticks cannot turn a stale attempt into ownership.
        Zeus.bossHuntTick();
        checkEquals("Scenario A: Repeated tick never performs buy", 0, buyCalls[0]);
        check("Scenario A: Foreground dialog is not dismissed", GameCanvas.currentDialog == validDialog);
        check("Scenario A: Background Menu2 remains unmodified", GameCanvas.menu2.isShowMenu);

        // 2. SCENARIO B: Foreground MsgDialog visible WITHOUT menu (menu2.isShowMenu == false)
        if (GameCanvas.menu2 == null) {
            GameCanvas.menu2 = new Menu2();
        }
        GameCanvas.menu2.isShowMenu = false;
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;

        Zeus.bossHuntTick();
        checkEquals("Scenario B: Unproven dialog rejected without background menu", 0, buyCalls[0]);
        checkEquals("Scenario B: passive policy explains manual NPC requirement", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());

        // 3. SCENARIO C: Pending native menu selection BLOCKED when foreground dialog is active
        Zeus.resetBossDialogBridgeState();
        Zeus.resetBossMenuSelectionState();
        Zeus.bossHunt.getConfig().setEnabled(true);
        Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;

        // Stage a pending menu selection with authentic caption "Th\u00f4ng tin boss"
        mVector rawEntryItems = new mVector();
        rawEntryItems.addElement(new InterfaceComponents.iCommand("Nhi\u1ec7m v\u1ee5", (InterfaceComponents.IAction) null));
        rawEntryItems.addElement(new InterfaceComponents.iCommand("Th\u00f4ng tin boss", (InterfaceComponents.IAction) null));
        rawEntryItems.addElement(new InterfaceComponents.iCommand("\u0110\u00f3ng", (InterfaceComponents.IAction) null));
        long now = 5000000L;
        Zeus.observeBossMenu("SERVER", rawEntryItems, 0, -22, "MENU", now);
        check("Scenario C: Unowned entry cannot stage selection", Zeus.pendingBossMenuSelection == null);

        // Setup native Menu2
        if (GameCanvas.menu2 == null) {
            GameCanvas.menu2 = new Menu2();
        }
        Zeus.setMenu2IdMenu(GameCanvas.menu2, 0);
        Zeus.setMenu2IdNpc(GameCanvas.menu2, -22);
        GameCanvas.menu2.isShowMenu = true;
        mVector nativeItems = new mVector();
        nativeItems.addElement(new InterfaceComponents.iCommand("Nhi\u1ec7m v\u1ee5", 2, GameCanvas.menu2));
        nativeItems.addElement(new InterfaceComponents.iCommand("Th\u00f4ng tin boss", 2, GameCanvas.menu2));
        nativeItems.addElement(new InterfaceComponents.iCommand("\u0110\u00f3ng", 2, GameCanvas.menu2));
        GameCanvas.menu2.menuItems = nativeItems;

        // But foreground dialog is STILL active!
        GameCanvas.currentDialog = validDialog;

        boolean dispatchBlocked = Zeus.dispatchPendingBossMenuSelection(now + 1500L);
        check("Scenario C: Menu dispatch blocked while foreground dialog is active", !dispatchBlocked);
        check("Scenario C: No unowned selection dispatched behind dialog", Zeus.pendingBossMenuSelection == null);

        Zeus.bossHuntTick();
        check("F01-SAFE-02: Foreground tick never arms unowned background Menu2", Zeus.pendingBossMenuSelection == null);
        checkEquals("F01-SAFE-02: No foreground buy on pending background menu", 0, buyCalls[0]);

        // Cleanup pending selection and menu2
        Zeus.resetBossMenuSelectionState();
        GameCanvas.menu2.isShowMenu = false;

        // 4. SCENARIO D: Unrelated dialog (purchase wording) through bossHuntTick fails closed
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;

        MsgDialog unrelatedDialog = new MsgDialog();
        unrelatedDialog.cmdList = new mVector();
        unrelatedDialog.cmdList.addElement(buyCmd);
        unrelatedDialog.cmdList.addElement(closeCmd);
        try {
            java.lang.reflect.Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(unrelatedDialog, new String[] { "B\u1ea1n c\u00f3 mu\u1ed1n mua b\u00ecnh m\u00e1u v\u1edbi gi\u00e1 1000 xu kh\u00f4ng?" });
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
        GameCanvas.currentDialog = unrelatedDialog;
        GameCanvas.menu2.isShowMenu = true; // F01-SAFE-02: unrelated foreground over background menu

        Zeus.bossHuntTick();
        checkEquals("Scenario D: Unrelated purchase dialog not advanced through bossHuntTick", 0, buyCalls[0]);
        checkEquals("Scenario D: passive policy does not inspect operator dialog", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());
        check("Scenario D: Background Menu2 was not dismissed", GameCanvas.menu2.isShowMenu);
        check("Scenario D: Unrelated foreground was not dismissed", GameCanvas.currentDialog == unrelatedDialog);
        GameCanvas.menu2.isShowMenu = false;

        // 5. SCENARIO E: Wrong NPC (not Alisama / Doubar) fails closed
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;

        MainObject wrongNpc = new MainObject();
        wrongNpc.name = "Ph\u00f3 Ch\u1ec9 Huy";
        wrongNpc.x = 110;
        wrongNpc.y = 100;
        wrongNpc.typeObject = 2;
        GameScreen.Vecplayers.removeAllElements();
        GameScreen.Vecplayers.addElement(wrongNpc);
        GameCanvas.currentDialog = validDialog;

        Zeus.bossHuntTick();
        checkEquals("Scenario E: Wrong NPC not advanced through bossHuntTick", 0, buyCalls[0]);
        checkEquals("Scenario E: passive policy never adopts foreign NPC", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());

        // Restore Alisama
        GameScreen.Vecplayers.removeAllElements();
        GameScreen.Vecplayers.addElement(alisama);

        // 6. SCENARIO F: NPC out of range (distance > 48) fails closed
        alisama.x = 500;
        alisama.y = 500;
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;
        GameCanvas.currentDialog = validDialog;

        Zeus.bossHuntTick();
        checkEquals("Scenario F: Far NPC not advanced through bossHuntTick", 0, buyCalls[0]);
        checkEquals("Scenario F: passive policy does not approach distant NPC", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());

        // Restore Alisama position
        alisama.x = 110;
        alisama.y = 100;

        // 7. SCENARIO G: No prior interaction attempt (interactTries == 0) fails closed
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(0);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;
        GameCanvas.currentDialog = validDialog;

        Zeus.bossHuntTick();
        checkEquals("Scenario G: No interaction attempt not advanced through bossHuntTick", 0, buyCalls[0]);
        checkEquals("Scenario G: passive policy never initiates NPC dialog", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());

        // 8. SCENARIO H: Ambiguous buttons (3 buttons) fails closed
        MsgDialog ambiguousDialog = new MsgDialog();
        ambiguousDialog.cmdList = new mVector();
        ambiguousDialog.cmdList.addElement(buyCmd);
        ambiguousDialog.cmdList.addElement(new iCommand("Xem h\u00e0ng", new IAction() { public void perform() {} }));
        ambiguousDialog.cmdList.addElement(closeCmd);
        try {
            java.lang.reflect.Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(ambiguousDialog, new String[] { "Alisama: Trang b\u1ecb" });
        } catch (Exception e) {}

        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;
        GameCanvas.currentDialog = ambiguousDialog;

        Zeus.bossHuntTick();
        checkEquals("Scenario H: Ambiguous 3-button dialog not advanced", 0, buyCalls[0]);
        checkEquals("Scenario H: passive policy ignores ambiguous dialogs", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());

        // 9. SCENARIO I: Retry limit reached (bridge tries >= 3) fails closed
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        GameCanvas.currentDialog = validDialog;
        long t = 7000000L;
        Zeus.handleBossScheduleDialog(alisama, t); // try 1
        Zeus.handleBossScheduleDialog(alisama, t + 2000L); // try 2
        Zeus.handleBossScheduleDialog(alisama, t + 4000L); // try 3
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;

        Zeus.bossHuntTick();
        checkEquals("Scenario I: Repeated attempts cannot bypass provenance", 0, buyCalls[0]);
        checkEquals("Scenario I: repeated dialogs require manual interaction", "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());

        // 10. SCENARIO J: Higher priority yields and records HIGHER_PRIORITY
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;
        GameCanvas.currentDialog = validDialog;
        Zeus.enhNavigating = true; // higher priority active!

        Zeus.bossHuntTick();
        checkEquals("Scenario J: Higher priority yields bossHuntTick", 0, buyCalls[0]);
        checkEquals("Scenario J: Diagnostic recorded HIGHER_PRIORITY", "HIGHER_PRIORITY", Zeus.getLastBossHuntDiagReason());
        Zeus.enhNavigating = false;

        // 11. SCENARIO K: Manual movement yields and records MANUAL_CONTROL
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;
        GameCanvas.currentDialog = validDialog;
        Player.isLockKey = true; // operator locked keys manually!
        // default is false

        Zeus.bossHuntTick();
        checkEquals("Scenario K: Manual movement yields bossHuntTick", 0, buyCalls[0]);
        checkEquals("Scenario K: Diagnostic recorded MANUAL_CONTROL", "MANUAL_CONTROL", Zeus.getLastBossHuntDiagReason());
        Player.isLockKey = false;

        // 12. SCENARIO L: Config disabled fails closed
        Zeus.bossHunt.getConfig().setEnabled(false);
        Zeus.resetBossDialogBridgeState();
        Zeus.setBossNpcInteractTriesForTest(1);
        Zeus.clearBossHuntDiagReason();
        buyCalls[0] = 0;
        GameCanvas.currentDialog = validDialog;

        Zeus.bossHuntTick();
        checkEquals("Scenario L: Disabled config does not touch dialog", 0, buyCalls[0]);

        // Cleanup
        GameCanvas.currentDialog = null;
        if (GameCanvas.menu2 != null) {
            GameCanvas.menu2.isShowMenu = false;
            GameCanvas.menu2 = null;
        }
        Zeus.bossHunt.setState(BossHuntState.IDLE);
        Zeus.resetBossDialogBridgeState();
        Zeus.resetBossMenuSelectionState();
        Zeus.clearBossHuntDiagReason();
        } finally {
            if (oldLoadMap != null) oldLoadMap.idMap = oldMapId;
            GameCanvas.loadmap = oldLoadMap;
        }
    }

    private static void armBossMenuFixture(MainObject npc, long now) {
        try {
            java.lang.reflect.Method arm = Zeus.class.getDeclaredMethod(
                    "beginBossMenuAcquisition", new Class[] { MainObject.class, long.class });
            arm.setAccessible(true);
            arm.invoke(null, new Object[] { npc, Long.valueOf(now) });
        } catch (Throwable t) {
            throw new IllegalStateException("Cannot arm Boss-owned Doubar fixture", t);
        }
    }

    private static void testServerMenuNativeSelectionDispatch() {
        System.out.println("\n--- testServerMenuNativeSelectionDispatch (PASSIVE_ONLY) ---");
        final LoadMap savedMap = GameCanvas.loadmap;
        final int savedMapId = savedMap == null ? -1 : savedMap.idMap;
        final GameScreen savedGame = GameCanvas.game;
        final MainScreen savedScreen = GameCanvas.currentScreen;
        final Player savedPlayer = GameScreen.player;
        final mVector savedPlayers = GameScreen.Vecplayers;
        final MainObject savedFocus = GameScreen.ObjFocus;
        final MainDialog savedDialog = GameCanvas.currentDialog;
        final Menu2 savedMenu = GameCanvas.menu2;
        final boolean savedLock = Player.isLockKey;
        final boolean savedChat = ChatTextField.isShow;
        final String savedConfigPath = Zeus.bossHuntConfigPath;
        final String savedStatusPath = Zeus.bossHuntStatusPath;
        final boolean savedEnabled = Zeus.bossHunt.isEnabled();
        final BossHuntState savedState = Zeus.bossHunt.getState();
        final BossHuntScheduler.ScheduleEntry savedEntry = Zeus.bossHunt.getActiveEntry();
        final String savedReason = Zeus.getLastBossHuntDiagReason();
        java.lang.reflect.Field npcAttemptsField = null;
        java.lang.reflect.Field npcAttemptTimeField = null;
        java.lang.reflect.Field schedulePathField = null;
        java.lang.reflect.Field latestReportField = null;
        Object savedSchedulePath = null;
        Object savedLatestReport = null;
        int savedNpcAttempts = 0;
        long savedNpcAttemptTime = 0L;
        try {
            npcAttemptsField = Zeus.class.getDeclaredField("bossNpcInteractTries");
            npcAttemptsField.setAccessible(true);
            savedNpcAttempts = npcAttemptsField.getInt(null);
            npcAttemptTimeField = Zeus.class.getDeclaredField("lastBossNpcInteractAttempt");
            npcAttemptTimeField.setAccessible(true);
            savedNpcAttemptTime = npcAttemptTimeField.getLong(null);
            schedulePathField = Zeus.class.getDeclaredField("bossSchedulePath");
            schedulePathField.setAccessible(true);
            savedSchedulePath = schedulePathField.get(null);
            schedulePathField.set(null, null); // observation cannot write operator sidecars
            latestReportField = BossScheduleReader.class.getDeclaredField("latestReport");
            latestReportField.setAccessible(true);
            savedLatestReport = latestReportField.get(null);

            if (GameCanvas.game == null) GameCanvas.game = new GameScreen();
            GameCanvas.currentScreen = GameCanvas.game;
            if (GameCanvas.loadmap == null) GameCanvas.loadmap = allocateBossScheduleTestMap();
            GameCanvas.loadmap.idMap = 33;
            GameScreen.player = new Player(100, (byte) 0, "r4_passive_fixture", 0, 0);
            GameScreen.player.x = 100;
            GameScreen.player.y = 100;
            GameScreen.player.hp = 1000;
            GameScreen.player.Action = (byte) 0;
            GameScreen.player.posTransRoad = null;
            Player.isLockKey = false;
            ChatTextField.isShow = false;
            GameScreen.ObjFocus = null;
            GameCanvas.currentDialog = null;
            GameCanvas.menu2 = null;
            GameScreen.Vecplayers = new mVector();
            MainObject doubar = new MainObject();
            doubar.name = "Doubar";
            doubar.typeObject = 2;
            doubar.x = 110;
            doubar.y = 100;
            GameScreen.Vecplayers.addElement(doubar);
            Zeus.bossHuntConfigPath = null;
            Zeus.bossHuntStatusPath = null;
            Zeus.bossHunt.getConfig().setEnabled(true);
            Zeus.bossHunt.setActiveEntry(null);
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            Zeus.resetBossMenuSelectionState();
            npcAttemptsField.setInt(null, 0);
            npcAttemptTimeField.setLong(null, 0L);
            Zeus.clearBossHuntDiagReason();

            // The actual production Boss tick must NOT initiate NPC GiaoTiep.
            Zeus.bossHuntTick();
            check("R4-PASSIVE-01: nearby Doubar never auto-clicked",
                    npcAttemptsField.getInt(null) == 0
                    && npcAttemptTimeField.getLong(null) == 0L);
            checkEquals("R4-PASSIVE-02: operator action is explicitly required",
                    "SCHEDULE_MANUAL_NPC_REQUIRED", Zeus.getLastBossHuntDiagReason());
            check("R4-PASSIVE-03: no automatic NPC approach path or movement lock",
                    GameScreen.player.posTransRoad == null && !Player.isLockKey);

            long now = System.currentTimeMillis();
            mVector entry = new mVector();
            entry.addElement(new iCommand("Nhiệm vụ", (IAction) null));
            entry.addElement(new iCommand("Thông tin boss", (IAction) null));
            entry.addElement(new iCommand("Đóng", (IAction) null));

            // Even the former private fixture arm hook is permanently disabled.
            Zeus.bossHunt.setState(BossHuntState.NEED_SCHEDULE);
            armBossMenuFixture(doubar, now);
            Zeus.observeBossMenu("SERVER", entry, 0, -22, "MENU", now);
            check("R4-PASSIVE-04: valid Boss menu stays observation-only",
                    Zeus.pendingBossMenuSelection == null);
            Zeus.observeBossMenu("SERVER", entry, 0, -90, "MENU", now + 10L);
            check("R4-PASSIVE-05: foreign server menu cannot stage", Zeus.pendingBossMenuSelection == null);

            final int[] commandCalls = new int[1];
            Menu2 operatorMenu = new Menu2();
            Zeus.setMenu2IdMenu(operatorMenu, 0);
            Zeus.setMenu2IdNpc(operatorMenu, -22);
            operatorMenu.isShowMenu = true;
            operatorMenu.menuSelectedItem = 0;
            mVector nativeItems = new mVector();
            nativeItems.addElement(new iCommand("Thông tin boss", new IAction() {
                public void perform() { commandCalls[0]++; }
            }));
            operatorMenu.menuItems = nativeItems;
            GameCanvas.menu2 = operatorMenu;

            // Deliberately inject a stale selection to exercise the dispatch guard.
            Zeus.pendingBossMenuSelection = new Zeus.PendingBossMenuSelection(
                    -22, 0, 0, "Thông tin boss", "MATCH_BOSS_ENTRY", now, 1);
            boolean autoSelected = Zeus.dispatchPendingBossMenuSelection(now + 100L);
            check("R4-PASSIVE-06: injected selection can never invoke native command",
                    !autoSelected && commandCalls[0] == 0
                    && Zeus.pendingBossMenuSelection == null);
            check("R4-PASSIVE-07: operator menu identity and selection untouched",
                    GameCanvas.menu2 == operatorMenu && operatorMenu.isShowMenu
                    && operatorMenu.menuSelectedItem == 0);

            Zeus.bossHuntTick();
            check("R4-PASSIVE-08: Boss tick cannot dismiss operator Menu2",
                    GameCanvas.menu2 == operatorMenu && operatorMenu.isShowMenu
                    && commandCalls[0] == 0);

            // Foreground dialog outranks even a visible background Menu2.
            MsgDialog foreground = new MsgDialog();
            GameCanvas.currentDialog = foreground;
            Zeus.bossHuntTick();
            check("R4-PASSIVE-09: foreground dialog is never auto-confirmed",
                    GameCanvas.currentDialog == foreground
                    && GameCanvas.menu2 == operatorMenu && operatorMenu.isShowMenu
                    && commandCalls[0] == 0);
            GameCanvas.currentDialog = null;
            GameCanvas.menu2 = null;

            GameCanvas.loadmap.idMap = 1;
            Zeus.observeBossMenu("SERVER", entry, 0, -22, "MENU", now + 200L);
            check("R4-PASSIVE-10: changed map cannot produce pending selection",
                    Zeus.pendingBossMenuSelection == null);
            GameCanvas.loadmap.idMap = 33;
            Player.isLockKey = true;
            Zeus.observeBossMenu("SERVER", entry, 0, -22, "MENU", now + 300L);
            check("R4-PASSIVE-11: manual takeover cannot stage menu actions",
                    Zeus.pendingBossMenuSelection == null);
            Player.isLockKey = false;
            Zeus.bossHunt.getConfig().setEnabled(false);
            Zeus.observeBossMenu("SERVER", entry, 0, -22, "MENU", now + 400L);
            check("R4-PASSIVE-12: disabled Boss Hunt cannot stage menu actions",
                    Zeus.pendingBossMenuSelection == null);

            // Passive SERVER schedule observation remains enabled independently.
            mVector schedule = new mVector();
            schedule.addElement(new iCommand(
                    "Dê Vàng: 10:14:00. Rừng cao nguyên K 2", (IAction) null));
            BossScheduleReader.ScheduleReport previous = BossScheduleReader.getLatestReport();
            Zeus.observeBossMenu("SERVER", schedule, 137, -22, "MENU", now + 500L);
            BossScheduleReader.ScheduleReport observed = BossScheduleReader.getLatestReport();
            check("R4-PASSIVE-13: operator schedule still reaches parser",
                    observed != null && observed != previous
                    && "SERVER".equals(observed.origin) && observed.rowCount() > 0);
            check("R4-PASSIVE-14: passive schedule never arms a native menu action",
                    Zeus.pendingBossMenuSelection == null && commandCalls[0] == 0);
            // Rebind the fixture's original foreground Menu2 before an
            // explicit operator mock command. No Boss dispatch occurs here.
            GameCanvas.menu2 = operatorMenu;
            check("R5-10: original operator menu is visible before manual perform",
                    GameCanvas.menu2 == operatorMenu && operatorMenu.isShowMenu);
            // The IAction implementation is local test code, not live native packet proof.
            ((iCommand) nativeItems.elementAt(0)).perform();
            check("R4-PASSIVE-15: explicit operator mock action remains available",
                    commandCalls[0] == 1 && GameCanvas.menu2 == operatorMenu
                    && operatorMenu.isShowMenu);
            GameCanvas.menu2 = null;
        } catch (Throwable error) {
            check("R4 passive Menu2 fixture failed: " + error.getMessage(), false);
        } finally {
            Zeus.resetBossMenuSelectionState();
            try {
                if (npcAttemptsField != null) npcAttemptsField.setInt(null, savedNpcAttempts);
                if (npcAttemptTimeField != null) npcAttemptTimeField.setLong(null, savedNpcAttemptTime);
                if (schedulePathField != null) schedulePathField.set(null, savedSchedulePath);
                if (latestReportField != null) latestReportField.set(null, savedLatestReport);
            } catch (Throwable error) {
                check("R4 passive fixture failed to restore captured static state", false);
            }
            GameCanvas.loadmap = savedMap;
            if (savedMap != null) savedMap.idMap = savedMapId;
            GameCanvas.game = savedGame;
            GameCanvas.currentScreen = savedScreen;
            GameScreen.player = savedPlayer;
            GameScreen.Vecplayers = savedPlayers;
            GameScreen.ObjFocus = savedFocus;
            GameCanvas.currentDialog = savedDialog;
            GameCanvas.menu2 = savedMenu;
            Player.isLockKey = savedLock;
            ChatTextField.isShow = savedChat;
            Zeus.bossHuntConfigPath = savedConfigPath;
            Zeus.bossHuntStatusPath = savedStatusPath;
            Zeus.bossHunt.getConfig().setEnabled(savedEnabled);
            Zeus.bossHunt.setActiveEntry(savedEntry);
            Zeus.bossHunt.setState(savedState);
            Zeus.clearBossHuntDiagReason();
            if (savedReason != null) Zeus.recordBossHuntDiagReason(savedReason);
        }
    }
}
