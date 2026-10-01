import java.lang.reflect.Field;
import java.lang.reflect.Method;

public class DungeonStateMachineTest {

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static Field f(String name) throws Exception {
        Field field = Zeus.class.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    static void set(String name, Object value) throws Exception {
        f(name).set(null, value);
    }

    static Object get(String name) throws Exception {
        return f(name).get(null);
    }

    static void call(String name) throws Exception {
        Method m = Zeus.class.getDeclaredMethod(name);
        m.setAccessible(true);
        m.invoke(null);
    }

    static void callInt(String name, int arg) throws Exception {
        Method m = Zeus.class.getDeclaredMethod(name, int.class);
        m.setAccessible(true);
        m.invoke(null, arg);
    }

    static int getGoal() throws Exception {
        Method m = Zeus.class.getDeclaredMethod("goal");
        m.setAccessible(true);
        return ((Integer) m.invoke(null)).intValue();
    }

    static void initWorld() throws Exception {
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
            fu.q.d = 1;
        }
        cs.i = 10;
        cs.j = 20;
        if (cn.g == null) {
            cn.g = new bq(100, (byte) 0, "hero", 0, 0);
        }
        cn.g.cG = (byte) 0; // alive
        cn.g.bt = 1000;
        cn.g.bs = 1000;
        cn.g.bv = 1000;
        cn.g.bu = 1000;
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
        cn.j = new et("entities");
        try {
            Field fNet = ef.class.getDeclaredField("a");
            fNet.setAccessible(true);
            if (fNet.get(q.a()) == null) {
                fNet.set(q.a(), new l());
            }
        } catch (Throwable t) {
        }
    }

    static java.util.Vector getSentPackets() throws Exception {
        Field fNet = ef.class.getDeclaredField("a");
        fNet.setAccessible(true);
        l netL = (l) fNet.get(q.a());
        Field fAx = l.class.getDeclaredField("o");
        fAx.setAccessible(true);
        ax axObj = (ax) fAx.get(netL);
        return axObj.a;
    }

    public static void main(String[] args) throws Exception {
        System.out.println("=== DungeonStateMachineTest ===");
        initWorld();

        // ---------------------------------------------------------------------
        // Test 1: Route to Map 1
        // ---------------------------------------------------------------------
        System.out.println("--- Test 1: Route to Map 1 ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        fu.q.d = 44; // Standing on Map 44
        set("navTarget", -1);
        set("navDone", false);

        callInt("dungeonGotoNpc", 44);
        check("Dungeon requests routing when not on Map 1", ((Boolean) get("dungeonNavigating")).booleanValue());
        check("goal() returns Map 1 when dungeonNavigating", getGoal() == Zeus.DUNGEON_NPC_MAP);

        // Arrival on Map 1
        fu.q.d = 1;
        cn.g.aZ = Zeus.DUNGEON_NPC_X;
        cn.g.ba = Zeus.DUNGEON_NPC_Y;
        callInt("dungeonGotoNpc", 1);
        check("dungeonNavigating cleared upon arrival on Map 1", !((Boolean) get("dungeonNavigating")).booleanValue());

        // ---------------------------------------------------------------------
        // Test 2: NPC Semantic Menu Flow
        // ---------------------------------------------------------------------
        System.out.println("--- Test 2: NPC Semantic Menu Flow ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonTried", 0);
        set("dungeonWait", 0);

        // Step 0: NPC dialog menu with "Giao tiếp" vs "Giao dịch"
        set("dungeonMenu", new String[] { "Giao dịch", "Giao tiếp", "Đóng" });
        call("dungeonInteract");
        check("Picks 'Giao tiếp' and advances to step 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("Wait budget armed after step 0 pick", ((Integer) get("dungeonWait")).intValue() == 60);

        // Step 1: Dungeon selection menu with "Vào Ngã Tư Tử Thần"
        set("dungeonMenu", new String[] { "Nhiệm vụ", "Vào Ngã Tư Tử Thần", "Rời đi" });
        call("dungeonInteract");
        check("Picks 'Vào Ngã Tư Tử Thần' and advances to step 2", ((Integer) get("dungeonStep")).intValue() == 2);
        check("Wait budget armed after step 1 pick", ((Integer) get("dungeonWait")).intValue() == 60);

        // Step 2: Confirmation dialog
        ah confirmDialog = new ah();
        bt yesBtn = new bt("Có", 1);
        bt noBtn = new bt("Không", 2);
        confirmDialog.C.a(yesBtn);
        confirmDialog.C.a(noBtn);
        fu.s = confirmDialog;
        call("dungeonInteract");
        check("Confirms dialog and advances to step 3", ((Integer) get("dungeonStep")).intValue() == 3);
        check("Wait budget armed after confirmation", ((Integer) get("dungeonWait")).intValue() == 80);

        // ---------------------------------------------------------------------
        // Test 3: Map 48 Entry
        // ---------------------------------------------------------------------
        System.out.println("--- Test 3: Map 48 Entry ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon");
        check("dungeonWasIn set upon Map 48 entry", ((Boolean) get("dungeonWasIn")).booleanValue());
        check("dungeonState transitions to DN_COMBAT", ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMBAT);
        check("dungeonNavigating is false inside dungeon", !((Boolean) get("dungeonNavigating")).booleanValue());

        // ---------------------------------------------------------------------
        // Test 4: Combat Profile Backup and Restore
        // ---------------------------------------------------------------------
        System.out.println("--- Test 4: Combat Profile Backup and Restore ---");
        Zeus.dungeonReset();
        set("atkMode", 1);
        set("atkMap", 5);
        set("atkX", 100);
        set("atkY", 200);
        set("atkRadius", 120);
        set("atkFarmOnArrival", true);

        Zeus.dungeonBackupCombat();
        check("dungeonCombatBackedUp is true", ((Boolean) get("dungeonCombatBackedUp")).booleanValue());

        // Mutate combat fields inside dungeon
        cn.g.bi = Zeus.DUNGEON_SCAN_RADIUS;
        check("Scan radius expanded to 600", cn.g.bi == 600);

        // Restore combat
        Zeus.dungeonRestoreCombat();
        check("dungeonCombatBackedUp cleared", !((Boolean) get("dungeonCombatBackedUp")).booleanValue());
        check("atkMode restored to 1", ((Integer) get("atkMode")).intValue() == 1);
        check("atkMap restored to 5", ((Integer) get("atkMap")).intValue() == 5);
        check("atkX restored to 100", ((Integer) get("atkX")).intValue() == 100);
        check("atkY restored to 200", ((Integer) get("atkY")).intValue() == 200);
        check("atkRadius restored to 120", ((Integer) get("atkRadius")).intValue() == 120);
        check("Native scan radius restored to 140", cn.g.bi == 140);

        // ---------------------------------------------------------------------
        // Test 5: Meteor Exclusion
        // ---------------------------------------------------------------------
        System.out.println("--- Test 5: Meteor Exclusion ---");
        fa meteor = new fa();
        meteor.cv = 1;
        meteor.bs = 1000;
        meteor.cC = "Thiên thạch lửa";
        meteor.aZ = 672;
        meteor.ba = 600;

        fa normal = new fa();
        normal.cv = 1;
        normal.bs = 1000;
        normal.cC = "Bọ cạp độc";
        normal.aZ = 675;
        normal.ba = 605;

        check("isMeteorTarget detects 'thien thach'", Zeus.isMeteorTarget(meteor));
        check("isMeteorTarget ignores normal monster", !Zeus.isMeteorTarget(normal));
        check("isValidDungeonTarget rejects meteor", !Zeus.isValidDungeonTarget(meteor));
        check("isValidDungeonTarget accepts normal monster", Zeus.isValidDungeonTarget(normal));

        // Held meteor target dropped immediately
        cn.i = meteor;
        cn.j = new et("entities");
        cn.j.a(meteor);
        cn.j.a(normal);
        Zeus.dungeonCombat();
        check("Held meteor target released", cn.i != meteor);
        check("Target switched to valid normal monster", cn.i == normal);

        // ---------------------------------------------------------------------
        // Test 6: Center Leash
        // ---------------------------------------------------------------------
        System.out.println("--- Test 6: Center Leash ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        fu.q.d = Zeus.DUNGEON_MAP;
        cn.g.aZ = 500; // Drifted from (672, 600) > 48 px
        cn.g.ba = 500;
        bq.m = false;
        cn.g.cO = null;
        cn.j = new et("empty"); // No monsters
        cn.i = null;
        set("dungeonNoTargetTicks", 19);

        set("travelStallTicks", 0);
        Zeus.dungeonCombat();
        check("Idle leash tick counter incremented to 20", ((Integer) get("dungeonNoTargetTicks")).intValue() == 20);
        check("Leash movement initiated toward (672, 600)", bq.m || cn.g.cO != null || ((Integer) get("travelStallTicks")).intValue() > 0);

        // ---------------------------------------------------------------------
        // Test 7: Death Inside Dungeon + Map 1 Respawn != Success
        // ---------------------------------------------------------------------
        System.out.println("--- Test 7: Death Inside Dungeon + Map 1 Respawn != Success ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 0);
        set("dungeonFails", 0);
        set("dungeonConsecutiveFails", 0);

        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon"); // Enter Map 48
        set("dungeonCombatEngaged", true);

        // Character dies inside Map 48
        cn.g.cG = 4;
        call("dungeon");
        check("dungeonDiedInRun is true after death", ((Boolean) get("dungeonDiedInRun")).booleanValue());
        check("dungeonState is DN_DEATH", ((Integer) get("dungeonState")).intValue() == Zeus.DN_DEATH);

        // Character wakes up in town (Map 1)
        fu.q.d = 1;
        cn.g.cG = 0; // alive in town
        call("dungeon");

        check("dungeonRuns was NOT incremented on town respawn", ((Integer) get("dungeonRuns")).intValue() == 0);
        check("dungeonFails incremented on town respawn", ((Integer) get("dungeonFails")).intValue() == 1);
        check("dungeonConsecutiveFails incremented", ((Integer) get("dungeonConsecutiveFails")).intValue() == 1);
        check("dungeonWhy reflects death respawn (7)", ((Integer) get("dungeonWhy")).intValue() == 7);

        // ---------------------------------------------------------------------
        // Test 8: Clean Alive Completion Path
        // ---------------------------------------------------------------------
        System.out.println("--- Test 8: Clean Alive Completion Path ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 0);
        set("dungeonFails", 0);
        set("dungeonConsecutiveFails", 0);

        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon"); // Enter Map 48
        set("dungeonCombatEngaged", true);
        set("dungeonClearCandidate", true); // Observed victory dialog
        set("dungeonDiedInRun", false);
        set("dungeonManualEscaped", false);
        cn.g.cG = 0; // alive

        // Server teleports back to Map 1
        fu.q.d = 1;
        call("dungeon");

        check("Clean exit increments dungeonRuns exactly once", ((Integer) get("dungeonRuns")).intValue() == 1);
        check("dungeonConsecutiveFails reset to 0", ((Integer) get("dungeonConsecutiveFails")).intValue() == 0);
        check("dungeonState transitioned to DN_COMPLETION_WAIT / DN_DONE", ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMPLETION_WAIT || ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        // ---------------------------------------------------------------------
        // Test 9: Ambiguous Map 48 -> Map 1 Exit Fails Closed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 9: Ambiguous Map 48 -> Map 1 Exit Fails Closed ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 1);
        set("dungeonFails", 0);

        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon"); // Enter Map 48
        // Exited immediately to Map 1 with NO combat and NO clear signal
        set("dungeonCombatEngaged", false);
        set("dungeonClearCandidate", false);
        set("dungeonMonstersCleared", false);
        fu.q.d = 1;
        call("dungeon");

        check("Ambiguous exit did NOT increment dungeonRuns", ((Integer) get("dungeonRuns")).intValue() == 1);
        check("Ambiguous exit incremented dungeonFails", ((Integer) get("dungeonFails")).intValue() == 1);
        check("Ambiguous exit why is 10", ((Integer) get("dungeonWhy")).intValue() == 10);

        // ---------------------------------------------------------------------
        // Test 10: Disconnect Inside Map 48 Recovery
        // ---------------------------------------------------------------------
        System.out.println("--- Test 10: Disconnect Inside Map 48 Recovery ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 3);
        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon");

        // Disconnect occurs, sessionReset invoked
        Zeus.sessionReset();
        check("dungeonWasIn preserved during inside-map48 session reset", ((Boolean) get("dungeonWasIn")).booleanValue());

        // Next tick reconstructs combat
        call("dungeon");
        check("dungeonState remains DN_COMBAT", ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMBAT);
        check("dungeonRuns unchanged after reconnect (still 3)", ((Integer) get("dungeonRuns")).intValue() == 3);

        // ---------------------------------------------------------------------
        // Test 11: Reconnect to Town Aborts Run
        // ---------------------------------------------------------------------
        System.out.println("--- Test 11: Reconnect to Town Aborts Run ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 3);
        set("dungeonFails", 0);
        set("dungeonWasIn", true);

        // Reconnect lands on Map 1 instead of Map 48
        fu.q.d = 1;
        Zeus.sessionReset();

        check("Town reconnect aborted run (fails incremented)", ((Integer) get("dungeonFails")).intValue() == 1);
        check("Town reconnect did NOT increment dungeonRuns", ((Integer) get("dungeonRuns")).intValue() == 3);
        check("dungeonWhy is 8 (disconnect abort)", ((Integer) get("dungeonWhy")).intValue() == 8);

        // ---------------------------------------------------------------------
        // Test 12: Two Consecutive Deaths Trigger Safety Stop
        // ---------------------------------------------------------------------
        System.out.println("--- Test 12: Two Consecutive Deaths Trigger Safety Stop ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonConsecutiveFails", 0);

        // Death 1
        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon");
        set("dungeonCombatEngaged", true);
        cn.g.cG = 4;
        call("dungeon");
        fu.q.d = 1; // Respawn town
        cn.g.cG = 0;
        call("dungeon");
        check("Consecutive fails = 1 after first death", ((Integer) get("dungeonConsecutiveFails")).intValue() == 1);
        check("Module still enabled after 1 failure", ((Boolean) get("dungeonEnabled")).booleanValue());

        // Death 2
        set("dungeonState", Zeus.DN_IDLE);
        set("dungeonWait", 0);
        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon");
        set("dungeonCombatEngaged", true);
        cn.g.cG = 4;
        call("dungeon");
        fu.q.d = 1; // Respawn town
        cn.g.cG = 0;
        call("dungeon");

        check("Consecutive fails = 2 triggers safety stop", ((Integer) get("dungeonConsecutiveFails")).intValue() == 2);
        check("State transitioned to DN_MANUAL_REVIEW", ((Integer) get("dungeonState")).intValue() == Zeus.DN_MANUAL_REVIEW);
        check("Why code is 5 (failure cap)", ((Integer) get("dungeonWhy")).intValue() == 5);

        // ---------------------------------------------------------------------
        // Test 13: Unlimited Mode (dungeon.max = -1) Respects Failure Cap
        // ---------------------------------------------------------------------
        System.out.println("--- Test 13: Unlimited Mode Respects Failure Cap ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonMaxRuns", -1); // Unlimited successful runs
        set("dungeonConsecutiveFails", 1); // 1 previous failure

        // Another failure occurs
        Zeus.dungeonFailRun(7, "death respawn");
        check("Unlimited mode stops at consecutive failure cap", ((Integer) get("dungeonState")).intValue() == Zeus.DN_MANUAL_REVIEW);
        check("Why is 5 under unlimited mode", ((Integer) get("dungeonWhy")).intValue() == 5);

        // ---------------------------------------------------------------------
        // Test 14: Manual Travel Conflict and Safe Yield
        // ---------------------------------------------------------------------
        System.out.println("--- Test 14: Manual Travel Conflict and Safe Yield ---");
        Zeus.dungeonReset();
        set("atkMode", 0);
        set("dungeonEnabled", true);
        set("dungeonNavigating", true);
        set("navTarget", 8); // Manual travel target
        set("navDone", false);
        fu.q.d = 44;

        callInt("dungeonGotoNpc", 44);
        check("Dungeon yields when Manual Travel is active", !((Boolean) get("dungeonNavigating")).booleanValue());
        check("Dungeon why is 9 (manual travel conflict)", ((Integer) get("dungeonWhy")).intValue() == 9);
        check("goal() honors navTarget (8) over dungeon", getGoal() == 8);

        // ---------------------------------------------------------------------
        // Test 15: Auto Farm Configuration Restoration
        // ---------------------------------------------------------------------
        System.out.println("--- Test 15: Auto Farm Configuration Restoration ---");
        Zeus.dungeonReset();
        set("atkMode", 2);
        set("atkMap", 20);
        set("atkX", 300);
        set("atkY", 400);
        set("atkRadius", 150);
        set("atkFarmOnArrival", true);

        // Backup
        Zeus.dungeonBackupCombat();
        // Mutate inside dungeon
        set("atkMap", 48);
        cn.g.bi = 600;

        // Restore
        Zeus.dungeonRestoreCombat();
        check("Auto Farm atkMode restored", ((Integer) get("atkMode")).intValue() == 2);
        check("Auto Farm atkMap restored", ((Integer) get("atkMap")).intValue() == 20);
        check("Auto Farm atkX restored", ((Integer) get("atkX")).intValue() == 300);
        check("Auto Farm atkY restored", ((Integer) get("atkY")).intValue() == 400);
        check("Auto Farm atkRadius restored", ((Integer) get("atkRadius")).intValue() == 150);

        // ---------------------------------------------------------------------
        // Test 16: Run-Count Limit
        // ---------------------------------------------------------------------
        System.out.println("--- Test 16: Run-Count Limit ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonMaxRuns", 3);
        set("dungeonRuns", 2);

        Zeus.dungeonDone(); // Run 3 complete
        check("Runs incremented to 3", ((Integer) get("dungeonRuns")).intValue() == 3);
        check("Reaching maxRuns stops module with why=4", ((Integer) get("dungeonWhy")).intValue() == 4);

        // ---------------------------------------------------------------------
        // Test 17: Schedule Regression
        // ---------------------------------------------------------------------
        System.out.println("--- Test 17: Schedule Regression ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 0);
        set("dungeonMaxRuns", -1);
        set("dungeonState", Zeus.DN_IDLE);
        set("dungeonSchedule", 25); // 12:30
        set("dungeonTripActive", false);

        // If schedule not due, remains IDLE
        call("dungeonIdle");
        // Depending on current local time, it's either IDLE or ROUTING, but handles cleanly
        check("dungeonState is valid after schedule check", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE || ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);

        // ---------------------------------------------------------------------
        // Test 18: No Duplicate Success Accounting
        // ---------------------------------------------------------------------
        System.out.println("--- Test 18: No Duplicate Success Accounting ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 5);
        set("dungeonMaxRuns", 10);

        Zeus.dungeonDone();
        check("First clear increments to 6", ((Integer) get("dungeonRuns")).intValue() == 6);
        // Repeated clear call without entering dungeon again
        check("dungeonWasIn is false after completion", !((Boolean) get("dungeonWasIn")).booleanValue());

        // ---------------------------------------------------------------------
        // Test 19: Fresh Dungeon Arm Transient Interaction Reset
        // ---------------------------------------------------------------------
        System.out.println("--- Test 19: Fresh Dungeon Arm Transient Interaction Reset ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 2);
        set("dungeonWait", 50);
        set("dungeonTried", 3);
        set("dungeonMenu", new String[] { "Nhiệm vụ", "Vào Ngã Tư Tử Thần" });
        set("dungeonMenuNpc", 123);
        set("dungeonMenuId", 9999);
        ah staleDialog = new ah();
        staleDialog.C.a(new bt("Có", 1));
        staleDialog.C.a(new bt("Không", 2));
        staleDialog.q = new String[] { "Bạn có muốn vào Ngã tư tử thần không?" };
        fu.s = staleDialog;

        Zeus.dungeonReset();
        check("dungeonStep reset to 0", ((Integer) get("dungeonStep")).intValue() == 0);
        check("dungeonWait reset to 0", ((Integer) get("dungeonWait")).intValue() == 0);
        check("dungeonTried reset to 0", ((Integer) get("dungeonTried")).intValue() == 0);
        check("dungeonMenu reset to null", get("dungeonMenu") == null);
        check("dungeonMenuNpc reset to MIN_VALUE", ((Integer) get("dungeonMenuNpc")).intValue() == Integer.MIN_VALUE);
        check("dungeonMenuId reset to MIN_VALUE", ((Integer) get("dungeonMenuId")).intValue() == Integer.MIN_VALUE);
        check("stale dungeon dialog dismissed on reset", fu.s == null);

        // ---------------------------------------------------------------------
        // Test 20: Player Inside NPC Interaction Radius -> GOTO_NPC Dispatches Exactly One Opcode 23
        // ---------------------------------------------------------------------
        System.out.println("--- Test 20: Player Inside NPC Radius -> Exactly One Opcode 23 ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        set("navTarget", -1);
        set("navDone", true);
        fu.q.d = 1;
        fa phoChiHuy = new fa();
        phoChiHuy.cv = 2; // NPC
        phoChiHuy.cu = -37;
        phoChiHuy.cC = "Pho Chi Huy";
        phoChiHuy.aZ = 552;
        phoChiHuy.ba = 504;
        cn.j = new et("entities");
        cn.j.a(phoChiHuy);

        cn.g.aZ = 552;
        cn.g.ba = 504;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("dungeonState transitions to DN_PREPARATION", ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("dungeonStep set to 0", ((Integer) get("dungeonStep")).intValue() == 0);
        check("dungeonWait armed to bounded period (>= 20)", ((Integer) get("dungeonWait")).intValue() >= 20);
        check("Exactly one packet sent", getSentPackets().size() == 1);
        ep sentPkt = (ep) getSentPackets().get(0);
        check("Dispatched packet is opcode 23", sentPkt.a == 23);

        // ---------------------------------------------------------------------
        // Test 21: Player Outside Interaction Radius -> Movement, No Opcode 23
        // ---------------------------------------------------------------------
        System.out.println("--- Test 21: Player Outside Radius -> Movement, No Opcode 23 ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        fu.q.d = 1;
        cn.g.aZ = 100; // Far outside 80-radius
        cn.g.ba = 100;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("dungeonState remains DN_ROUTING while approaching", ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);
        check("No opcode 23 sent while outside interaction radius", getSentPackets().size() == 0);

        // ---------------------------------------------------------------------
        // Test 22: Stale Confirmation Dialog Cleared Safely During GotoNpc / Fresh Arm
        // ---------------------------------------------------------------------
        System.out.println("--- Test 22: Stale Confirmation Dialog Cleared Safely ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        fu.q.d = 1;
        cn.g.aZ = 552;
        cn.g.ba = 504;
        ah leftoverDialog = new ah();
        leftoverDialog.C.a(new bt("Có", 1));
        leftoverDialog.C.a(new bt("Không", 2));
        leftoverDialog.q = new String[] { "Bạn có muốn vào Ngã tư tử thần không?" };
        fu.s = leftoverDialog;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("Stale dialog dismissed during goto NPC", fu.s == null);
        boolean hasOpcode23 = false;
        for (int i = 0; i < getSentPackets().size(); i++) {
            ep pkt = (ep) getSentPackets().get(i);
            if (pkt.a == 23) {
                hasOpcode23 = true;
            }
        }
        check("No premature opcode 23 dispatched while clearing dialog", !hasOpcode23);

        // ---------------------------------------------------------------------
        // Test 23: Unrelated Dialog Does NOT Auto-Confirm and Fails Closed to DN_MANUAL_REVIEW
        // ---------------------------------------------------------------------
        System.out.println("--- Test 23: Unrelated Dialog Fails Closed Without Auto-Confirm ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        fu.q.d = 1;
        cn.g.aZ = 552;
        cn.g.ba = 504;
        ah unrelatedDialog = new ah();
        unrelatedDialog.q = new String[] { "Thong bao: Bao tri may chu!" };
        bt okBtn = new bt("Dong", 1);
        unrelatedDialog.C.a(okBtn);
        fu.s = unrelatedDialog;
        getSentPackets().clear();

        // 3 consecutive ticks with unrelated dialog
        for (int i = 0; i < 3; i++) {
            callInt("dungeonGotoNpc", 1);
        }
        check("Unrelated dialog NOT auto-confirmed", fu.s == unrelatedDialog);
        check("No opcode 23 dispatched while unrelated dialog present", getSentPackets().size() == 0);
        check("State transitioned to DN_MANUAL_REVIEW on persistent dialog", ((Integer) get("dungeonState")).intValue() == Zeus.DN_MANUAL_REVIEW);
        check("Why code set to 5", ((Integer) get("dungeonWhy")).intValue() == 5);

        // ---------------------------------------------------------------------
        // Test 24: No Duplicate Opcode 23 or NPC Reopen While Waiting
        // ---------------------------------------------------------------------
        System.out.println("--- Test 24: No Duplicate Opcode 23 or NPC Reopen While Waiting ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        fu.s = null;

        // Subtest A: Waiting for first menu (Step 0)
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 15);
        set("dungeonMenu", null);
        getSentPackets().clear();
        call("dungeonInteract");
        check("Wait budget decrements at step 0", ((Integer) get("dungeonWait")).intValue() == 14);
        check("No opcode 23 dispatched while waiting for first menu", getSentPackets().size() == 0);

        // Subtest B: Waiting for second menu (Step 1)
        set("dungeonStep", 1);
        set("dungeonWait", 15);
        set("dungeonMenu", null);
        getSentPackets().clear();
        call("dungeonInteract");
        check("Wait budget decrements at step 1", ((Integer) get("dungeonWait")).intValue() == 14);
        check("No reopen dispatched while waiting for second menu", getSentPackets().size() == 0);

        // Subtest C: Waiting for confirmation dialog (Step 2)
        set("dungeonStep", 2);
        set("dungeonWait", 15);
        fu.s = null;
        getSentPackets().clear();
        call("dungeonInteract");
        check("Wait budget decrements at step 2", ((Integer) get("dungeonWait")).intValue() == 14);
        check("No reopen dispatched while waiting for confirmation dialog", getSentPackets().size() == 0);

        // Subtest D: Waiting for teleport (Step 3)
        set("dungeonStep", 3);
        set("dungeonWait", 15);
        getSentPackets().clear();
        call("dungeonInteract");
        check("Wait budget decrements at step 3", ((Integer) get("dungeonWait")).intValue() == 14);
        check("No reopen dispatched while waiting for teleport", getSentPackets().size() == 0);

        // ---------------------------------------------------------------------
        // Test 25: norm() and normSemantic() Length Safety & Accent Permutations
        // ---------------------------------------------------------------------
        System.out.println("--- Test 25: norm() and normSemantic() Safety ---");
        Method normM = Zeus.class.getDeclaredMethod("norm", String.class);
        normM.setAccessible(true);
        Method normSemM = Zeus.class.getDeclaredMethod("normSemantic", String.class);
        normSemM.setAccessible(true);

        check("norm(null) returns empty string", "".equals(normM.invoke(null, (String) null)));
        check("norm(\"\") returns empty string", "".equals(normM.invoke(null, "")));

        String upperAccents = "ÀÁẠẢÃÂẦẤẬẨẪĂẰẮẶẲẴÈÉẸẺẼÊỀẾỆỂỄÌÍỊỈĨÒÓỌỎÕÔỒỐỘỔỖƠỜỚỢỞỠÙÚỤỦŨƯỪỨỰỬỮỲÝỴỶỸĐ";
        String lowerAccents = "àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡùúụủũưừứựửữỳýỵỷỹđ";
        String normUpper = (String) normM.invoke(null, upperAccents);
        String normLower = (String) normM.invoke(null, lowerAccents);
        check("norm(upper) does not throw and equals norm(lower)", normUpper != null && normUpper.equals(normLower));

        String phrase = "  Vào   Ngã  Tư  Tử  Thần  ";
        String semantic = (String) normSemM.invoke(null, phrase);
        check("normSemantic collapses whitespace and normalizes", "vao nga tu tu than".equals(semantic));

        // ---------------------------------------------------------------------
        // Test 26: Native NPC Interaction Parity and Interaction Range Enforcement
        // ---------------------------------------------------------------------
        System.out.println("--- Test 26: Native NPC Interaction Parity & Range Enforcement ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        set("navTarget", -1);
        set("navDone", true);
        fu.q.d = 1;

        fa npcTarget = new fa();
        npcTarget.cv = 2; // NPC
        npcTarget.cu = -37;
        npcTarget.cC = "Pho Chi Huy";
        npcTarget.aZ = 552;
        npcTarget.ba = 504;
        cn.j = new et("entities");
        cn.j.a(npcTarget);

        // Subtest A: Character outside native interaction range (e.g. distance 150px > 140px)
        // Must NOT trigger interaction or send opcode 23
        cn.g.aZ = 552 + 150;
        cn.g.ba = 504;
        cn.g.cH = 0;
        cn.i = null;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("At distance 150px (> 140px), dungeonState remains DN_ROUTING",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);
        check("At distance 150px, no opcode 23 dispatched", getSentPackets().size() == 0);

        // Subtest B: Character at (576, 504) - adjacent tile (distance 24px <= 36px)
        // Must arrive, set target focus cn.i, face NPC (cH=2), stop velocity, and dispatch opcode 23
        cn.g.aZ = 576;
        cn.g.ba = 504;
        cn.g.cH = 0;
        cn.i = null;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("At distance 24px (<= 36px), dungeonState transitions to DN_PREPARATION",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Target entity focus cn.i is set to NPC", cn.i == npcTarget);
        check("Character facing cH is turned towards NPC (cH=2)", cn.g.cH == 2);
        check("Character movement velocity is zeroed", cn.g.bc == 0 && cn.g.bd == 0);
        check("Exactly one opcode 23 dispatched on arrival",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);

        // Subtest C: Retry via dungeonAskNpc() maintains cn.i and facing
        cn.i = null;
        cn.g.cH = 0;
        getSentPackets().clear();
        call("dungeonAskNpc");
        check("dungeonAskNpc() re-establishes cn.i focus", cn.i == npcTarget);
        check("dungeonAskNpc() re-establishes facing cH=2", cn.g.cH == 2);
        check("dungeonAskNpc() dispatches opcode 23",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);

        // ---------------------------------------------------------------------
        // Test 27: travelArrive Semantics at Live Coordinates (596, 512)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 27: travelArrive Semantics at Live Coordinates ---");
        cn.g.aZ = 596;
        cn.g.ba = 512;
        boolean arriveResult36 = callTravelArrive(552, 504, 36);
        check("travelArrive(552, 504, 36) returns false at live coordinates (596, 512) because Manhattan 52 > 36",
                !arriveResult36);

        // ---------------------------------------------------------------------
        // Test 28: Native NPC Interaction Eligibility & Range Boundary Tests
        // ---------------------------------------------------------------------
        System.out.println("--- Test 28: Native NPC Interaction Eligibility & Range Boundary Tests ---");
        // At live coordinates (596, 512), Euclidean distance is sqrt(44^2 + 8^2) ≈ 44.72 <= 140
        boolean eligibleLive = callDungeonNpcEligible(npcTarget);
        check("dungeonNpcEligible returns true at live coordinates (596, 512) (Euclidean 44.72px <= 140px)",
                eligibleLive);

        // Boundary tests: exactly at 140px (inside) vs 141px (outside)
        cn.g.aZ = 552 + 140; // dx = 140, dy = 0, Euclidean = 140
        cn.g.ba = 504;
        check("dungeonNpcEligible returns true immediately inside native range (distance 140px <= 140px)",
                callDungeonNpcEligible(npcTarget));

        cn.g.aZ = 552 + 141; // dx = 141, dy = 0, Euclidean = 141
        cn.g.ba = 504;
        check("dungeonNpcEligible returns false immediately outside native range (distance 141px > 140px)",
                !callDungeonNpcEligible(npcTarget));

        // Outside native condition: approach continues, zero packets dispatched
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        set("navTarget", -1);
        set("navDone", true);
        cn.g.aZ = 552 + 141;
        cn.g.ba = 504;
        cn.i = null;
        getSentPackets().clear();
        callInt("dungeonGotoNpc", 1);
        check("Approach continues while native interaction condition is false (distance 141px)",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);
        check("Zero packets dispatched while outside native interaction condition",
                getSentPackets().size() == 0);

        // Inside native condition at live coordinates (596, 512): arrives, halts, faces, dispatches opcode 23
        cn.g.aZ = 596;
        cn.g.ba = 512;
        cn.g.cH = 0;
        cn.g.bc = 5;
        cn.g.bd = 3;
        cn.i = null;
        getSentPackets().clear();
        callInt("dungeonGotoNpc", 1);
        check("At live coordinates (596, 512), dungeonState transitions to DN_PREPARATION",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Target entity focus cn.i set to NPC at (596, 512)", cn.i == npcTarget);
        check("Character facing cH is turned towards NPC (cH=2)", cn.g.cH == 2);
        check("Movement velocity halted (bc=0, bd=0)", cn.g.bc == 0 && cn.g.bd == 0);
        check("Exactly one opcode 23 dispatched when native interaction condition becomes true",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);

        // ---------------------------------------------------------------------
        // Test 29: Packet Serialization Parity & Wire Format
        // ---------------------------------------------------------------------
        System.out.println("--- Test 29: Packet Serialization Parity ---");
        ep pkt = (ep) getSentPackets().get(0);
        byte[] payload = pkt.a();
        check("Opcode is 23", pkt.a == 23);
        check("Payload length is 1", payload != null && payload.length == 1);
        check("Payload byte is (byte) -37 (0xDB)", payload != null && payload[0] == (byte) -37 && (payload[0] & 0xff) == 0xdb);

        // ---------------------------------------------------------------------
        // ---------------------------------------------------------------------
        // Test 30: Retry Bounded Wait and Exactly One Bounded Retry
        // ---------------------------------------------------------------------
        System.out.println("--- Test 30: Retry Bounded Wait ---");
        int waitArmed = ((Integer) get("dungeonWait")).intValue();
        check("First menu wait is armed to source-appropriate bounded period (>= 40 ticks)",
                waitArmed >= 40);

        // Ticking while waiting for menu does not dispatch duplicate packets
        getSentPackets().clear();
        for (int t = 0; t < 20; t++) {
            call("dungeonInteract");
        }
        check("No duplicate opcode 23 dispatched during menu wait countdown",
                getSentPackets().size() == 0);

        // ---------------------------------------------------------------------
        // Test 31: Broadcast Popup Coexistence with NPC Approach and Interaction
        // ---------------------------------------------------------------------
        System.out.println("--- Test 31: Broadcast Popup Coexistence with NPC Approach ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        set("navTarget", -1);
        set("navDone", true);
        fu.q.d = 1;

        fa phoChiHuy31 = new fa();
        phoChiHuy31.cv = 2; // NPC
        phoChiHuy31.cu = -37;
        phoChiHuy31.cC = "Pho Chi Huy";
        phoChiHuy31.aZ = 552;
        phoChiHuy31.ba = 504;
        cn.j = new et("entities");
        cn.j.a(phoChiHuy31);

        // Position player at live coordinates (612, 392)
        cn.g.aZ = 612;
        cn.g.ba = 392;
        cn.g.cH = 0;
        cn.i = null;

        // Present broadcast popup
        ah broadcastPopup = makeBroadcastPopup("Chúc mừng người chơi test đã vượt qua đợt thứ 10");
        fu.s = broadcastPopup;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("Dungeon does not treat broadcast popup as blocking modal",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Native interaction opcode 23 dispatched while broadcast popup present",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);
        check("Broadcast popup remains present and unmodified after interaction dispatch",
                fu.s == broadcastPopup);

        // ---------------------------------------------------------------------
        // Test 32: Menu Processing with Broadcast Popup Present
        // ---------------------------------------------------------------------
        System.out.println("--- Test 32: Menu Processing with Broadcast Popup Present ---");
        // Step 0: First menu arrives while broadcast popup is still on screen
        et firstMenuItems = new et("firstMenu");
        firstMenuItems.a(new bt("Giao tiếp", 0));
        firstMenuItems.a(new bt("Đóng", 1));
        callServerMenu(-37, 1, "Pho Chi Huy", firstMenuItems);

        check("dungeonMenu captured first menu while broadcast popup present",
                get("dungeonMenu") != null);
        getSentPackets().clear();

        call("dungeonInteract");
        check("Step 0 selects 'Giao tiếp' and advances to step 1 under broadcast popup",
                ((Integer) get("dungeonStep")).intValue() == 1);
        check("Broadcast popup still preserved and untouched after first menu selection",
                fu.s == broadcastPopup);

        // Step 1: Second menu arrives while broadcast popup is still on screen
        et secondMenuItems = new et("secondMenu");
        secondMenuItems.a(new bt("Vào Ngã Tư Tử Thần", 0));
        secondMenuItems.a(new bt("Đóng", 1));
        callServerMenu(-37, 2, "Menu", secondMenuItems);

        check("dungeonMenu captured second menu while broadcast popup present",
                get("dungeonMenu") != null);

        call("dungeonInteract");
        check("Step 1 selects 'Vào Ngã Tư Tử Thần' and advances to step 2 under broadcast popup",
                ((Integer) get("dungeonStep")).intValue() == 2);
        check("Broadcast popup still preserved and untouched after second menu selection",
                fu.s == broadcastPopup);

        // ---------------------------------------------------------------------
        // Test 33: Expected Dungeon Confirmation Dialog Handling with Broadcast Popup
        // ---------------------------------------------------------------------
        System.out.println("--- Test 33: Expected Dungeon Confirmation Handling ---");
        // At step 2, while broadcast popup is still in fu.s, dungeonInteract must NOT auto-confirm it
        set("dungeonWait", 10);
        call("dungeonInteract");
        check("Dungeon does not confirm broadcast popup as dungeon confirmation",
                ((Integer) get("dungeonStep")).intValue() == 2);
        check("Broadcast popup remains unconfirmed in fu.s", fu.s == broadcastPopup);

        // Genuine confirmation dialog arrives (replaces fu.s on client UI)
        ah confirmDialog33 = new ah();
        confirmDialog33.q = new String[] { "Bạn có muốn vào Ngã tư tử thần không?" };
        confirmDialog33.C = new et("buttons");
        confirmDialog33.C.a(new bt("Có", 1));
        confirmDialog33.C.a(new bt("Không", 2));
        fu.s = confirmDialog33;

        call("dungeonInteract");
        check("Genuine confirmation dialog confirmed and advances to step 3",
                ((Integer) get("dungeonStep")).intValue() == 3);
        check("Wait budget armed for teleport after confirmation (>= 80)",
                ((Integer) get("dungeonWait")).intValue() >= 80);

        // ---------------------------------------------------------------------
        // Test 34: Known Blocking Modal Prevents Unsafe Interaction (Fails Closed)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 34: Known Blocking Modal Prevents Unsafe Interaction ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        fu.q.d = 1;
        cn.g.aZ = 552;
        cn.g.ba = 504;

        ah blockingModal = makeBlockingModal("Bạn có chắc chắn muốn rời khỏi bang hội không?");
        fu.s = blockingModal;
        getSentPackets().clear();

        for (int i = 0; i < 3; i++) {
            callInt("dungeonGotoNpc", 1);
        }
        check("Blocking modal NOT auto-confirmed", fu.s == blockingModal);
        check("No opcode 23 dispatched while blocking modal present", getSentPackets().size() == 0);
        check("State transitioned to DN_MANUAL_REVIEW on persistent blocking modal",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_MANUAL_REVIEW);
        check("Why code set to 5", ((Integer) get("dungeonWhy")).intValue() == 5);

        // ---------------------------------------------------------------------
        // Test 35: Live Regression Case Parity
        // ---------------------------------------------------------------------
        System.out.println("--- Test 35: Live Regression Case Parity ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        set("navTarget", -1);
        set("navDone", true);
        fu.q.d = 1;

        cn.g.aZ = 612;
        cn.g.ba = 392;
        cn.g.cH = 0;
        cn.i = null;

        ah liveBroadcast = makeBroadcastPopup("Chúc mừng ... đã vượt qua đợt thứ 10");
        fu.s = liveBroadcast;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("Player at (612, 392) interacts with NPC (552, 504) while broadcast visible",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Opcode 23 sent for CU -37",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);
        check("Live broadcast popup preserved and untouched", fu.s == liveBroadcast);

        // ---------------------------------------------------------------------
        // Test 36: V2 Native Parity - First Menu Native Command Contract
        // ---------------------------------------------------------------------
        System.out.println("--- Test 36: V2 Native Parity - First Menu Native Command Contract ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);

        if (fu.p == null) {
            fu.p = new fr();
        }
        fu.p.a = true;
        setFrH(fu.p, -1);
        et menuItems36 = new et("menu36");
        final int[] callbacks36 = new int[2];
        cg target36 = new cg() {
            public void a(int e, int f) {
                if (e == 4) {
                    callbacks36[0]++;
                } else if (e == 5) {
                    callbacks36[1]++;
                }
            }
        };
        bt btnGiaoTiep36 = new bt("Giao tiếp", 4, target36);
        bt btnDong36 = new bt("Đóng", 5, target36);
        menuItems36.a(btnGiaoTiep36);
        menuItems36.a(btnDong36);
        setFrG(fu.p, menuItems36);

        callServerMenu(-37, 2, "Pho Chi Huy", menuItems36);
        getSentPackets().clear();

        call("dungeonInteract");

        check("Step 0 sets fu.p.h to matching 'Giao tiếp' item index (0)", getFrH(fu.p) == 0);
        check("Step 0 invokes native bt.a() command callback exactly once", callbacks36[0] == 1);
        check("Step 0 does NOT synthesize raw server-menu q.b packet", getSentPackets().size() == 0);
        check("Step 0 advances dungeonStep to 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("Step 0 arms wait budget (>= 40)", ((Integer) get("dungeonWait")).intValue() >= 40);

        // ---------------------------------------------------------------------
        // Test 37: Stale UI Reference Rejection
        // ---------------------------------------------------------------------
        System.out.println("--- Test 37: Stale UI Reference Rejection ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);

        // Menu was closed (fu.p.a = false)
        fu.p.a = false;
        callbacks36[0] = 0;
        getSentPackets().clear();

        call("dungeonInteract");
        check("Stale/closed menu does not trigger command callback", callbacks36[0] == 0);
        check("Stale menu does not dispatch packets", getSentPackets().size() == 0);

        // ---------------------------------------------------------------------
        // Test 38: Second Menu Native Action Contract
        // ---------------------------------------------------------------------
        System.out.println("--- Test 38: Second Menu Native Action Contract ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 1);
        set("dungeonWait", 0);
        set("dungeonTried", 0);

        fu.p.a = true;
        setFrC(fu.p, -37);
        setFrB(fu.p, 3);
        setFrH(fu.p, -1);
        et menuItems38 = new et("menu38");
        menuItems38.a(new bt("Vào Ngã Tư Tử Thần", 0));
        menuItems38.a(new bt("Đóng", 1));
        setFrG(fu.p, menuItems38);

        callServerMenu(-37, 3, "Nga Tu", menuItems38);
        getSentPackets().clear();

        call("dungeonInteract");
        check("Step 1 sets fu.p.h to matching 'Ngã Tư' index (0)", getFrH(fu.p) == 0);
        check("Step 1 invokes native action which closes fu.p", !fu.p.a);
        check("Step 1 dispatches server-menu q.b packet via native handler",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == -30);
        check("Step 1 advances dungeonStep to 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // ---------------------------------------------------------------------
        // Test 39: Fast Immediate Server Response Race (First Menu -> Second Menu)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 39: Fast Immediate Response Race (Step 0 -> Step 1) ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);

        fu.p.a = true;
        setFrH(fu.p, -1);
        et menuItems39 = new et("menu39");
        cg fastReplyNpc = new cg() {
            public void a(int e, int f) {
                // Immediate synchronous reply from server: second menu arrives DURING callback!
                try {
                    et fastSecond = new et("fastSecond");
                    fastSecond.a(new bt("Vào Ngã Tư Tử Thần", 0));
                    fastSecond.a(new bt("Đóng", 1));
                    setFrG(fu.p, fastSecond);
                    setFrB(fu.p, 4);
                    setFrC(fu.p, -37);
                    fu.p.a = true;
                    callServerMenu(-37, 4, "Nga Tu Fast", fastSecond);
                } catch (Exception ex) {
                }
            }
        };
        menuItems39.a(new bt("Giao tiếp", 4, fastReplyNpc));
        setFrG(fu.p, menuItems39);
        callServerMenu(-37, 2, "Pho Chi Huy", menuItems39);

        call("dungeonInteract");
        check("State armed to accept fast second menu without dropping",
                ((Integer) get("dungeonStep")).intValue() == 1);
        check("Fast second menu captured in dungeonMenu", get("dungeonMenu") != null);

        // ---------------------------------------------------------------------
        // Test 40: Fast Immediate Response Race (Step 1 -> Confirmation)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 40: Fast Immediate Response Race (Step 1 -> Confirmation) ---");
        // Next tick processes the fast second menu
        call("dungeonInteract");
        check("Step 1 processed fast second menu and advanced to Step 2",
                ((Integer) get("dungeonStep")).intValue() == 2);

        // Immediate confirmation arrives
        ah fastConfirm = new ah();
        fastConfirm.q = new String[] { "Bạn có muốn vào Ngã tư tử thần không?" };
        fastConfirm.C = new et("buttons");
        final boolean[] confirmClicked = new boolean[1];
        cg confirmTarget = new cg() {
            public void a(int e, int f) {
                confirmClicked[0] = true;
            }
        };
        fastConfirm.C.a(new bt("Có", 1, confirmTarget));
        fastConfirm.C.a(new bt("Không", 2));
        fu.s = fastConfirm;

        call("dungeonInteract");
        check("Step 2 confirms fast confirmation dialog", confirmClicked[0]);
        check("Step 2 advances to Step 3", ((Integer) get("dungeonStep")).intValue() == 3);

        // ---------------------------------------------------------------------
        // Test 41: Broadcast Coexistence Throughout Entry
        // ---------------------------------------------------------------------
        System.out.println("--- Test 41: Broadcast Coexistence Throughout Entry ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);

        ah entryBroadcast = makeBroadcastPopup("Thông báo: Sự kiện đang diễn ra!");
        fu.s = entryBroadcast;

        fu.p.a = true;
        et bCastFirstMenu = new et("bCastFirstMenu");
        bCastFirstMenu.a(new bt("Giao tiếp", 4, target36));
        setFrG(fu.p, bCastFirstMenu);
        callServerMenu(-37, 2, "Pho Chi Huy", bCastFirstMenu);

        call("dungeonInteract");
        check("Broadcast popup preserved during Step 0 Giao tiếp", fu.s == entryBroadcast);
        check("Step 0 advanced under broadcast popup", ((Integer) get("dungeonStep")).intValue() == 1);

        // ---------------------------------------------------------------------
        // Test 42: Full Entry Chain Deterministic Parity
        // ---------------------------------------------------------------------
        System.out.println("--- Test 42: Full Entry Chain Deterministic Parity ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        fu.q.d = 1;
        cn.g.aZ = 552;
        cn.g.ba = 504;
        cn.g.cH = 0;
        cn.i = null;

        fa pcf42 = new fa();
        pcf42.cv = 2;
        pcf42.cu = -37;
        pcf42.cC = "Pho Chi Huy";
        pcf42.aZ = 552;
        pcf42.ba = 504;
        cn.j = new et("entities");
        cn.j.a(pcf42);

        // Step A: approach and arrival
        getSentPackets().clear();
        callInt("dungeonGotoNpc", 1);
        check("Full Chain: Arrival transitions to DN_PREPARATION",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Full Chain: Opcode 23 sent to NPC",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);

        // Step B: First menu arrives
        fu.p.a = true;
        setFrH(fu.p, -1);
        et chainFirst = new et("chainFirst");
        final boolean[] chainFirstClicked = new boolean[1];
        cg chainTarget = new cg() {
            public void a(int e, int f) {
                chainFirstClicked[0] = true;
            }
        };
        chainFirst.a(new bt("Giao tiếp", 4, chainTarget));
        chainFirst.a(new bt("Đóng", 5));
        setFrG(fu.p, chainFirst);
        callServerMenu(-37, 2, "Pho Chi Huy", chainFirst);

        call("dungeonInteract");
        check("Full Chain: Step 0 native callback invoked", chainFirstClicked[0]);
        check("Full Chain: State advanced to Step 1", ((Integer) get("dungeonStep")).intValue() == 1);

        // Step C: Second menu arrives
        fu.p.a = true;
        setFrC(fu.p, -37);
        setFrB(fu.p, 3);
        setFrH(fu.p, -1);
        et chainSecond = new et("chainSecond");
        chainSecond.a(new bt("Vào Ngã Tư Tử Thần", 0));
        chainSecond.a(new bt("Đóng", 1));
        setFrG(fu.p, chainSecond);
        callServerMenu(-37, 3, "Nga Tu", chainSecond);

        getSentPackets().clear();
        call("dungeonInteract");
        check("Full Chain: Step 1 native action sent q.b",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == -30);
        check("Full Chain: State advanced to Step 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // Step D: Confirmation dialog arrives
        ah chainConfirm = new ah();
        chainConfirm.q = new String[] { "Bạn có muốn vào Ngã tư tử thần không?" };
        chainConfirm.C = new et("buttons");
        final boolean[] chainConfirmClicked = new boolean[1];
        cg chainConfirmTarget = new cg() {
            public void a(int e, int f) {
                chainConfirmClicked[0] = true;
            }
        };
        chainConfirm.C.a(new bt("Có", 1, chainConfirmTarget));
        chainConfirm.C.a(new bt("Không", 2));
        fu.s = chainConfirm;

        call("dungeonInteract");
        check("Full Chain: Step 2 confirmation invoked", chainConfirmClicked[0]);
        check("Full Chain: State advanced to Step 3", ((Integer) get("dungeonStep")).intValue() == 3);

        // Step E: Server teleports player to Map 48
        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon");
        check("Full Chain: Arrival in Map 48 enters DN_COMBAT",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMBAT);

        // ---------------------------------------------------------------------
        // Test 43: fu.t Native NPC Dialog Discovery Contract (DUNGEON-04G2)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 43: fu.t Native NPC Dialog Discovery Contract ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        fu.q.d = 1;
        fu.s = null;
        fu.p.a = false;
        setFrG(fu.p, null);

        ah futDialog = new ah();
        futDialog.q = new String[] { "Ta có một nhiệm vụ rất quan trọng đang cần mi giúp đỡ!" };
        futDialog.C = new et("buttons");
        final boolean[] futGiaoTiepClicked = new boolean[1];
        cg futTarget = new cg() {
            public void a(int e, int f) {
                futGiaoTiepClicked[0] = true;
            }
        };
        futDialog.C.a(new bt("Giao tiếp", 4, futTarget));
        futDialog.C.a(new bt("Đóng", 5));
        fu.t = futDialog;

        getSentPackets().clear();
        call("dungeonInteract");
        check("fu.t Dialog: Step 0 executes native bt.a() from fu.t", futGiaoTiepClicked[0]);
        check("fu.t Dialog: Step advances to 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("fu.t Dialog: No synthetic q.b packet dispatched", getSentPackets().isEmpty());

        // ---------------------------------------------------------------------
        // Test 44: fu.s Broadcast Popup Coexistence with fu.t NPC Dialog
        // ---------------------------------------------------------------------
        System.out.println("--- Test 44: fu.s Broadcast Coexistence with fu.t NPC Dialog ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        fu.q.d = 1;
        fu.p.a = false;
        setFrG(fu.p, null);

        // fu.s has a server broadcast announcement
        fu.s = makeBroadcastPopup("Chúc mừng người chơi x đã vượt qua đợt 5");
        // fu.t has the active NPC dialog with Giao tiếp
        final boolean[] futCoexistClicked = new boolean[1];
        ah futCoexistDialog = new ah();
        futCoexistDialog.q = new String[] { "Ta có một nhiệm vụ rất quan trọng đang cần mi giúp đỡ!" };
        futCoexistDialog.C = new et("buttons");
        futCoexistDialog.C.a(new bt("Giao tiếp", 4, new cg() {
            public void a(int e, int f) {
                futCoexistClicked[0] = true;
            }
        }));
        futCoexistDialog.C.a(new bt("Đóng", 5));
        fu.t = futCoexistDialog;

        call("dungeonInteract");
        check("fu.t + fu.s: Giao tiếp executed from fu.t", futCoexistClicked[0]);
        check("fu.t + fu.s: Broadcast preserved in fu.s", fu.s != null);
        check("fu.t + fu.s: Step advances to 1", ((Integer) get("dungeonStep")).intValue() == 1);

        // ---------------------------------------------------------------------
        // Test 45: Stale fu.t Dialog Invalidation & Safety
        // ---------------------------------------------------------------------
        System.out.println("--- Test 45: Stale fu.t Dialog Invalidation & Safety ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        fu.q.d = 1;
        fu.p.a = false;
        setFrG(fu.p, null);

        // fu.t is replaced with a dialog that has NO Giao tiếp (e.g. general notice)
        ah futNoGiaoTiep = new ah();
        futNoGiaoTiep.q = new String[] { "Thông báo hệ thống" };
        futNoGiaoTiep.C = new et("buttons");
        futNoGiaoTiep.C.a(new bt("Đóng", 5));
        fu.t = futNoGiaoTiep;
        fu.s = null;

        call("dungeonInteract");
        check("Replaced fu.t without Giao tiếp does not advance", ((Integer) get("dungeonStep")).intValue() == 0);
        check("Wait budget decrements", ((Integer) get("dungeonWait")).intValue() < 60);

        // Null fu.t and null fu.p: wait budget decrements, no crash
        fu.t = null;
        call("dungeonInteract");
        check("Null fu.t and fu.p: Step remains 0", ((Integer) get("dungeonStep")).intValue() == 0);

        // ---------------------------------------------------------------------
        // Test 46: Full Entry Chain with fu.t Native Dialog
        // ---------------------------------------------------------------------
        System.out.println("--- Test 46: Full Entry Chain with fu.t Native Dialog ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        fu.q.d = 1;
        cn.g.aZ = 552;
        cn.g.ba = 504;

        fa pcf46 = new fa();
        pcf46.cv = 2;
        pcf46.cu = -37;
        pcf46.cC = "Pho Chi Huy";
        pcf46.aZ = 552;
        pcf46.ba = 504;
        cn.j = new et("entities");
        cn.j.a(pcf46);

        fu.s = makeBroadcastPopup("Server Broadcast Notice");
        fu.p.a = false;
        setFrG(fu.p, null);

        // Arrival at NPC
        getSentPackets().clear();
        callInt("dungeonGotoNpc", 1);
        check("Chain fu.t: Transitions to DN_PREPARATION",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Chain fu.t: Opcode 23 sent to NPC",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);

        // NPC dialog arrives in fu.t
        ah chainFutDialog = new ah();
        chainFutDialog.q = new String[] { "Ta có một nhiệm vụ rất quan trọng đang cần mi giúp đỡ!" };
        chainFutDialog.C = new et("buttons");
        final boolean[] chainFutClicked = new boolean[1];
        chainFutDialog.C.a(new bt("Giao tiếp", 4, new cg() {
            public void a(int e, int f) {
                chainFutClicked[0] = true;
            }
        }));
        chainFutDialog.C.a(new bt("Đóng", 5));
        fu.t = chainFutDialog;

        getSentPackets().clear();
        call("dungeonInteract");
        check("Chain fu.t: Native callback invoked from fu.t", chainFutClicked[0]);
        check("Chain fu.t: State advanced to Step 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("Chain fu.t: Broadcast popup in fu.s preserved", fu.s != null);

        // Second menu arrives in fu.p
        fu.p.a = true;
        setFrC(fu.p, -37);
        setFrB(fu.p, 3);
        setFrH(fu.p, -1);
        et chainFutSecond = new et("chainFutSecond");
        chainFutSecond.a(new bt("Vào Ngã Tư Tử Thần", 0));
        chainFutSecond.a(new bt("Đóng", 1));
        setFrG(fu.p, chainFutSecond);
        callServerMenu(-37, 3, "Nga Tu", chainFutSecond);

        getSentPackets().clear();
        call("dungeonInteract");
        check("Chain fu.t: Step 1 native action sent q.b (opcode -30)",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == -30);
        check("Chain fu.t: State advanced to Step 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // Confirmation arrives in fu.s
        ah chainFutConfirm = new ah();
        chainFutConfirm.q = new String[] { "Bạn có muốn vào Ngã tư tử thần không?" };
        chainFutConfirm.C = new et("buttons");
        final boolean[] chainFutConfirmClicked = new boolean[1];
        chainFutConfirm.C.a(new bt("Có", 1, new cg() {
            public void a(int e, int f) {
                chainFutConfirmClicked[0] = true;
            }
        }));
        chainFutConfirm.C.a(new bt("Không", 2));
        fu.s = chainFutConfirm;

        call("dungeonInteract");
        check("Chain fu.t: Step 2 confirmation invoked", chainFutConfirmClicked[0]);
        check("Chain fu.t: State advanced to Step 3", ((Integer) get("dungeonStep")).intValue() == 3);

        // Arrival on Map 48
        fu.q.d = Zeus.DUNGEON_MAP;
        call("dungeon");
        check("Chain fu.t: Arrival in Map 48 enters DN_COMBAT",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMBAT);

        // ---------------------------------------------------------------------
        // Test 47: Exact Native First Dialog Dispatch Parity & V2 Contract (DUNGEON-04H)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 47: Exact Native First Dialog Dispatch Parity & V2 Contract ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        fu.q.d = 1;
        fu.p.a = false;
        setFrG(fu.p, null);

        // 47.1: Baseline Failure Proof - Real native ah button in fu.t has bt.d == null.
        // Calling bt.a() with coexisting broadcast in fu.s diverts to fu.s.b() without sending Opcode 23.
        // Contract requirement: findGiaoTiepInDialog must reject bt.d == null as invalid command.
        Method mFindGiaoTiep = Zeus.class.getDeclaredMethod("findGiaoTiepInDialog", da.class);
        mFindGiaoTiep.setAccessible(true);

        ah nativeAhPlayerDialog = new ah();
        nativeAhPlayerDialog.C = new et("playerContext");
        bt nativeAhBtnNoTarget = new bt("Giao tiếp", 4); // bt.d is NULL in native ah.java:555
        nativeAhPlayerDialog.C.a(nativeAhBtnNoTarget);
        nativeAhPlayerDialog.C.a(new bt("Đóng", 8));
        fu.t = nativeAhPlayerDialog;
        fu.s = makeBroadcastPopup("Server Announcement");

        Object rejectedBtn = mFindGiaoTiep.invoke(null, nativeAhPlayerDialog);
        check("Exact Parity: Native ah button with bt.d == null is rejected", rejectedBtn == null);

        // 47.2: Exact Native fu.p First Dialog Contract with active broadcast
        // When Pho Chi Huy conversation opens natively in fu.p, bt.d is the NPC (ez/bm).
        // Step 0 must select fu.p, set cursor index fu.p.h = 0, pre-arm Step 1, invoke bt.a(),
        // dispatch Opcode 23 (payload 0xDB), and keep fu.s broadcast preserved.
        fu.p.a = true;
        fa liveNpc = new fa();
        liveNpc.cv = 2;
        liveNpc.cu = -37;
        liveNpc.cC = "Pho Chi Huy";
        cn.j = new et("entities");
        cn.j.a(liveNpc);

        et nativeFrItems = new et("nativeNpcMenu");
        final boolean[] liveNpcActionCalled = new boolean[1];
        bt liveNpcGiaoTiep = new bt("Giao tiếp", 4, new cg() {
            public void a(int e, int f) {
                liveNpcActionCalled[0] = true;
                try {
                    q.a().a((byte) liveNpc.cu);
                } catch (Throwable t) {}
            }
        });
        nativeFrItems.a(liveNpcGiaoTiep);
        setFrG(fu.p, nativeFrItems);
        setFrH(fu.p, -1);

        getSentPackets().clear();
        call("dungeonInteract");
        check("Exact Parity: fu.p native command callback invoked", liveNpcActionCalled[0]);
        check("Exact Parity: Step 0 sets fu.p.h cursor to 0", getFrH(fu.p) == 0);
        check("Exact Parity: Step advanced to 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("Exact Parity: Exactly one packet sent", getSentPackets().size() == 1);
        check("Exact Parity: Packet is Opcode 23 for NPC -37",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == 23);
        check("Exact Parity: Broadcast in fu.s preserved", fu.s != null);

        // 47.3: Fast Second-Menu Arrival Race Test (Step 0 -> Step 1 immediate arrival)
        // If Opcode -30 arrives immediately after Step 0 dispatch, Step 1 must process it without loss.
        fu.p.a = true;
        setFrC(fu.p, -37);
        setFrB(fu.p, 3);
        setFrH(fu.p, -1);
        et secondMenuItems47 = new et("secondMenu");
        secondMenuItems47.a(new bt("Vào Ngã Tư Tử Thần", 0));
        secondMenuItems47.a(new bt("Đóng", 1));
        setFrG(fu.p, secondMenuItems47);
        callServerMenu(-37, 3, "Nga Tu", secondMenuItems47);

        getSentPackets().clear();
        call("dungeonInteract");
        check("Fast Response: Step 1 native action sent q.b (opcode -30)",
                getSentPackets().size() == 1 && ((ep) getSentPackets().get(0)).a == -30);
        check("Fast Response: State advanced to Step 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // 47.4: V2 Native Speech Dialog Parity in fu.s
        // When speech dialog containing "Phó chỉ huy" appears in fu.s, it advances via softkey/Key 5.
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        fu.p.a = false;
        setFrG(fu.p, null);

        final boolean[] speechSoftkeyInvoked = new boolean[1];
        ah speechDialog = new ah();
        speechDialog.q = new String[] { "Phó chỉ huy: Ta có nhiệm vụ vào Ngã tư tử thần cho ngươi!" };
        speechDialog.ab = new bt("Giao tiếp", 4, new cg() {
            public void a(int e, int f) {
                speechSoftkeyInvoked[0] = true;
            }
        });
        fu.s = speechDialog;

        call("dungeonInteract");
        check("V2 Parity: Speech dialog advances via softkey/action", speechSoftkeyInvoked[0]);
        check("V2 Parity: Step advances to 1", ((Integer) get("dungeonStep")).intValue() == 1);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }

    static ah makeBroadcastPopup(String text) {
        ah dialog = new ah();
        dialog.q = new String[] { text };
        dialog.C = new et("buttons");
        dialog.C.a(new bt("Ok", -1));
        return dialog;
    }

    static ah makeBlockingModal(String text) {
        ah dialog = new ah();
        dialog.q = new String[] { text };
        dialog.C = new et("buttons");
        dialog.C.a(new bt("Đồng ý", 1));
        dialog.C.a(new bt("Không", 2));
        return dialog;
    }

    static void callServerMenu(int idNpc, int idMenu, String title, et items) throws Exception {
        Method m = Zeus.class.getDeclaredMethod("serverMenu", et.class, int.class, int.class, String.class);
        m.setAccessible(true);
        m.invoke(null, items, idMenu, idNpc, title);
    }

    static boolean callTravelArrive(int x, int y, int tol) throws Exception {
        Method m = Zeus.class.getDeclaredMethod("travelArrive", int.class, int.class, int.class);
        m.setAccessible(true);
        return ((Boolean) m.invoke(null, x, y, tol)).booleanValue();
    }

    static boolean callDungeonNpcEligible(fa npc) throws Exception {
        Method m = Zeus.class.getDeclaredMethod("dungeonNpcEligible", fa.class);
        m.setAccessible(true);
        return ((Boolean) m.invoke(null, npc)).booleanValue();
    }

    static int getFrH(fr menu) {
        try {
            Field f = fr.class.getDeclaredField("h");
            f.setAccessible(true);
            return f.getInt(menu);
        } catch (Throwable t) {
            return -999;
        }
    }

    static void setFrH(fr menu, int val) {
        try {
            Field f = fr.class.getDeclaredField("h");
            f.setAccessible(true);
            f.setInt(menu, val);
        } catch (Throwable t) {
        }
    }

    static et getFrG(fr menu) {
        try {
            Field f = fr.class.getDeclaredField("g");
            f.setAccessible(true);
            return (et) f.get(menu);
        } catch (Throwable t) {
            return null;
        }
    }

    static void setFrG(fr menu, et items) {
        try {
            Field f = fr.class.getDeclaredField("g");
            f.setAccessible(true);
            f.set(menu, items);
        } catch (Throwable t) {
        }
    }

    static void setFrC(fr menu, int val) {
        try {
            Field f = fr.class.getDeclaredField("C");
            f.setAccessible(true);
            f.setInt(menu, val);
        } catch (Throwable t) {
        }
    }

    static void setFrB(fr menu, int val) {
        try {
            Field f = fr.class.getDeclaredField("B");
            f.setAccessible(true);
            f.setInt(menu, val);
        } catch (Throwable t) {
        }
    }
}


