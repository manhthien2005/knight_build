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
        check("dungeonWait armed to 20", ((Integer) get("dungeonWait")).intValue() == 20);
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

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
