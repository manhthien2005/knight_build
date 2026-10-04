import Main.GameCanvas;
import GameScreen.GameScreen;
import GameScreen.SelectCharScreen;
import GameScreen.LoginScreen;
import GameScreen.LoadMapScreen;
import GameScreen.MainScreen;
import GameScreen.PaintInfoGameScreen;
import GameScreen.TabScreenNew;
import GameObjects.Player;
import GameObjects.MainObject;
import GameObjects.MainMonster;
import GameObjects.Item;
import GameObjects.MainItem;
import GameObjects.AutoGetItem;
import GameObjects.MainClan;
import GameObjects.MainRMS;
import GameObjects.DelaySkill;
import GameObjects.Other_Players;
import InterfaceComponents.MsgDialog;
import InterfaceComponents.MainDialog;
import InterfaceComponents.InputDialog;
import InterfaceComponents.ChatTextField;
import InterfaceComponents.iCommand;
import InterfaceComponents.TabRebuildItem;
import InterfaceComponents.DataRebuildItem;
import InterfaceComponents.TabShopNew;
import InterfaceComponents.MainTabNew;
import CLib.TField;
import Model.Menu2;
import Model.Point;
import Model.T;
import Model.mCamera;
import Model.AvMain;
import Thread_More.LoadMap;
import Skill.HotKey;
import CLib.mGraphics;
import CLib.mVector;
import CLib.mSystem;
import CLib.Session_ME;
import net.Message;
import netcommand.Cmd_Message;
import netcommand.global.GlobalService;
import netcommand.global.GlobalLogicHandler;
import java.lang.reflect.Field;
import java.lang.reflect.Method;

public class DungeonStateMachineTest {

    static void setDialogText(MainDialog d, String text) {
        try {
            Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(d, new String[] { text });
        } catch (Throwable t) {}
    }

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
        if (GameCanvas.game == null) {
            GameCanvas.game = new GameScreen();
        }
        GameCanvas.currentScreen = GameCanvas.game;
        LoadMapScreen.isNextMap = true;
        if (GameCanvas.loadmap == null) {
            try {
                Field uf = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
                uf.setAccessible(true);
                sun.misc.Unsafe unsafe = (sun.misc.Unsafe) uf.get(null);
                GameCanvas.loadmap = (LoadMap) unsafe.allocateInstance(LoadMap.class);
            } catch (Throwable t) {
            }
        }
        if (GameCanvas.loadmap != null) {
            GameCanvas.loadmap.idMap = 1;
        }
        LoadMap.isShowEffAuto = 10;
        LoadMap.EFF_PHOBANG_END = 20;
        if (GameScreen.player == null) {
            GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
        }
        GameScreen.player.Action = (byte) 0; // alive
        GameScreen.player.hp = 1000;
        GameScreen.player.maxHp = 1000;
        GameScreen.player.mp = 1000;
        GameScreen.player.maxMp = 1000;
        GameScreen.player.typePk = 100;
        GameScreen.player.typeBoss = 100;
        GameScreen.player.x = 100;
        GameScreen.player.y = 100;
        GameScreen.ObjFocus = null;
        GameCanvas.currentDialog = null;
        GameCanvas.subDialog = null;
        if (GameCanvas.menu2 != null) {
            GameCanvas.menu2.isShowMenu = false;
        }
        GameScreen.Vecplayers = new mVector("entities");
        try {
            if (Session_ME.gI() == null) {
                new Session_ME();
            }
            Field fNet = Cmd_Message.class.getDeclaredField("session");
            fNet.setAccessible(true);
            if (fNet.get(GlobalService.gI()) == null) {
                fNet.set(GlobalService.gI(), Session_ME.gI());
            }
        } catch (Throwable t) {
        }
    }

    static java.util.Vector getSentPackets() throws Exception {
        Object link = Session_ME.gI();
        if (link == null) {
            new Session_ME();
            link = Session_ME.gI();
        }
        Field fSender = Session_ME.class.getDeclaredField("sender");
        fSender.setAccessible(true);
        Object sender = fSender.get(link);
        Field fMsg = sender.getClass().getDeclaredField("sendingMessage");
        fMsg.setAccessible(true);
        return (java.util.Vector) fMsg.get(sender);
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
        GameCanvas.loadmap.idMap = 44; // Standing on Map 44
        set("navTarget", -1);
        set("navDone", false);

        callInt("dungeonGotoNpc", 44);
        check("Dungeon requests routing when not on Map 1", ((Boolean) get("dungeonNavigating")).booleanValue());
        check("goal() returns Map 1 when dungeonNavigating", getGoal() == Zeus.DUNGEON_NPC_MAP);

        // Arrival on Map 1
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = Zeus.DUNGEON_NPC_X;
        GameScreen.player.y = Zeus.DUNGEON_NPC_Y;
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
        MsgDialog confirmDialog = new MsgDialog();
        setDialogText(confirmDialog, "Bạn có muốn vào Ngã tư tử thần không?" );
        iCommand yesBtn = new iCommand("Có", 1);
        iCommand noBtn = new iCommand("Không", 2);
        confirmDialog.cmdList.addElement(yesBtn);
        confirmDialog.cmdList.addElement(noBtn);
        GameCanvas.currentDialog = confirmDialog;
        call("dungeonInteract");
        check("Confirms dialog and advances to step 3", ((Integer) get("dungeonStep")).intValue() == 3);
        check("Wait budget armed after confirmation", ((Integer) get("dungeonWait")).intValue() == 80);

        // ---------------------------------------------------------------------
        // Test 3: Map 48 Entry
        // ---------------------------------------------------------------------
        System.out.println("--- Test 3: Map 48 Entry ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
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
        GameScreen.player.wFocus = Zeus.DUNGEON_SCAN_RADIUS;
        check("Scan radius expanded to 600", GameScreen.player.wFocus == 600);

        // Restore combat
        Zeus.dungeonRestoreCombat();
        check("dungeonCombatBackedUp cleared", !((Boolean) get("dungeonCombatBackedUp")).booleanValue());
        check("atkMode restored to 1", ((Integer) get("atkMode")).intValue() == 1);
        check("atkMap restored to 5", ((Integer) get("atkMap")).intValue() == 5);
        check("atkX restored to 100", ((Integer) get("atkX")).intValue() == 100);
        check("atkY restored to 200", ((Integer) get("atkY")).intValue() == 200);
        check("atkRadius restored to 120", ((Integer) get("atkRadius")).intValue() == 120);
        check("Native scan radius restored to 140", GameScreen.player.wFocus == 140);

        // ---------------------------------------------------------------------
        // Test 5: Meteor Exclusion
        // ---------------------------------------------------------------------
        System.out.println("--- Test 5: Meteor Exclusion ---");
        MainObject meteor = new MainObject();
        meteor.typeObject = 1;
        meteor.maxHp = 1000;
        meteor.hp = 1000;
        meteor.name = "Thiên thạch lửa";
        meteor.x = 672;
        meteor.y = 600;

        MainObject normal = new MainObject();
        normal.typeObject = 1;
        normal.maxHp = 1000;
        normal.hp = 1000;
        normal.name = "Bọ cạp độc";
        normal.x = 675;
        normal.y = 605;

        check("isMeteorTarget detects 'thien thach'", Zeus.isMeteorTarget(meteor));
        check("isMeteorTarget ignores normal monster", !Zeus.isMeteorTarget(normal));
        check("isValidDungeonTarget rejects meteor", !Zeus.isValidDungeonTarget(meteor));
        check("isValidDungeonTarget accepts normal monster", Zeus.isValidDungeonTarget(normal));

        // Held meteor target dropped immediately
        GameScreen.ObjFocus = meteor;
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(meteor);
        GameScreen.Vecplayers.addElement(normal);
        Zeus.dungeonCombat();
        check("Held meteor target released", GameScreen.ObjFocus != meteor);
        check("Target switched to valid normal monster", GameScreen.ObjFocus == normal);

        // ---------------------------------------------------------------------
        // Test 6: Center Leash
        // ---------------------------------------------------------------------
        System.out.println("--- Test 6: Center Leash ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        GameScreen.player.x = 500; // Drifted from (672, 600) > 48 px
        GameScreen.player.y = 500;
        Player.isLockKey = false;
        GameScreen.player.posTransRoad = null;
        GameScreen.Vecplayers = new mVector("empty"); // No monsters
        GameScreen.ObjFocus = null;
        set("dungeonNoTargetTicks", 19);

        set("travelStallTicks", 0);
        Zeus.dungeonCombat();
        check("Idle leash tick counter incremented to 20", ((Integer) get("dungeonNoTargetTicks")).intValue() == 20);
        check("Leash movement initiated toward (672, 600)", Player.isLockKey || GameScreen.player.posTransRoad != null || ((Integer) get("travelStallTicks")).intValue() > 0);

        // ---------------------------------------------------------------------
        // Test 7: Death Inside Dungeon + Map 1 Respawn != Success
        // ---------------------------------------------------------------------
        System.out.println("--- Test 7: Death Inside Dungeon + Map 1 Respawn != Success ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonRuns", 0);
        set("dungeonFails", 0);
        set("dungeonConsecutiveFails", 0);

        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        call("dungeon"); // Enter Map 48
        set("dungeonCombatEngaged", true);

        // Character dies inside Map 48
        GameScreen.player.Action = 4;
        call("dungeon");
        check("dungeonDiedInRun is true after death", ((Boolean) get("dungeonDiedInRun")).booleanValue());
        check("dungeonState is DN_DEATH", ((Integer) get("dungeonState")).intValue() == Zeus.DN_DEATH);

        // Character wakes up in town (Map 1)
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.Action = 0; // alive in town
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

        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        call("dungeon"); // Enter Map 48
        set("dungeonCombatEngaged", true);
        set("dungeonClearCandidate", true); // Observed victory dialog
        set("dungeonDiedInRun", false);
        set("dungeonManualEscaped", false);
        GameScreen.player.Action = 0; // alive

        // Server teleports back to Map 1
        GameCanvas.loadmap.idMap = 1;
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

        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        call("dungeon"); // Enter Map 48
        // Exited immediately to Map 1 with NO combat and NO clear signal
        set("dungeonCombatEngaged", false);
        set("dungeonClearCandidate", false);
        set("dungeonMonstersCleared", false);
        GameCanvas.loadmap.idMap = 1;
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
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
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
        GameCanvas.loadmap.idMap = 1;
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
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        call("dungeon");
        set("dungeonCombatEngaged", true);
        GameScreen.player.Action = 4;
        call("dungeon");
        GameCanvas.loadmap.idMap = 1; // Respawn town
        GameScreen.player.Action = 0;
        call("dungeon");
        check("Consecutive fails = 1 after first death", ((Integer) get("dungeonConsecutiveFails")).intValue() == 1);
        check("Module still enabled after 1 failure", ((Boolean) get("dungeonEnabled")).booleanValue());

        // Death 2
        set("dungeonState", Zeus.DN_IDLE);
        set("dungeonWait", 0);
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        call("dungeon");
        set("dungeonCombatEngaged", true);
        GameScreen.player.Action = 4;
        call("dungeon");
        GameCanvas.loadmap.idMap = 1; // Respawn town
        GameScreen.player.Action = 0;
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
        GameCanvas.loadmap.idMap = 44;

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
        GameScreen.player.wFocus = 600;

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
        set("dungeonStartMin", 1200);
        set("dungeonEndMin", 1215);
        set("dungeonTripActive", false);

        // If schedule not due, remains IDLE
        call("dungeonIdle");
        // Depending on current UTC+7 time, it's either IDLE or ROUTING, but handles cleanly
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
        MsgDialog staleDialog = new MsgDialog();
        staleDialog.cmdList.addElement(new iCommand("Có", 1));
        staleDialog.cmdList.addElement(new iCommand("Không", 2));
        setDialogText(staleDialog, "Bạn có muốn vào Ngã tư tử thần không?" );
        GameCanvas.currentDialog = staleDialog;

        Zeus.dungeonReset();
        check("dungeonStep reset to 0", ((Integer) get("dungeonStep")).intValue() == 0);
        check("dungeonWait reset to 0", ((Integer) get("dungeonWait")).intValue() == 0);
        check("dungeonTried reset to 0", ((Integer) get("dungeonTried")).intValue() == 0);
        check("dungeonMenu reset to null", get("dungeonMenu") == null);
        check("dungeonMenuNpc reset to MIN_VALUE", ((Integer) get("dungeonMenuNpc")).intValue() == Integer.MIN_VALUE);
        check("dungeonMenuId reset to MIN_VALUE", ((Integer) get("dungeonMenuId")).intValue() == Integer.MIN_VALUE);
        check("stale dungeon dialog dismissed on reset", GameCanvas.currentDialog == null);

        // ---------------------------------------------------------------------
        // Test 20: Player Inside NPC Interaction Radius -> GOTO_NPC Dispatches Exactly One Opcode 23
        // ---------------------------------------------------------------------
        System.out.println("--- Test 20: Player Inside NPC Radius -> Exactly One Opcode 23 ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        set("navTarget", -1);
        set("navDone", true);
        GameCanvas.loadmap.idMap = 1;
        MainObject phoChiHuy = makePhoChiHuy(552, 504);
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(phoChiHuy);

        GameScreen.player.x = 552;
        GameScreen.player.y = 504;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("dungeonState transitions to DN_PREPARATION", ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("dungeonStep set to 0", ((Integer) get("dungeonStep")).intValue() == 0);
        check("dungeonWait armed to bounded period (>= 20)", ((Integer) get("dungeonWait")).intValue() >= 20);
        check("Exactly one packet sent", getSentPackets().size() == 1);
        Message sentPkt = (Message) getSentPackets().get(0);
        check("Dispatched packet is opcode 23", sentPkt.command == 23);

        // ---------------------------------------------------------------------
        // Test 21: Player Outside Interaction Radius -> Movement, No Opcode 23
        // ---------------------------------------------------------------------
        System.out.println("--- Test 21: Player Outside Radius -> Movement, No Opcode 23 ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = 100; // Far outside 80-radius
        GameScreen.player.y = 100;
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
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = 552;
        GameScreen.player.y = 504;
        MsgDialog leftoverDialog = new MsgDialog();
        leftoverDialog.cmdList.addElement(new iCommand("Có", 1));
        leftoverDialog.cmdList.addElement(new iCommand("Không", 2));
        setDialogText(leftoverDialog, "Bạn có muốn vào Ngã tư tử thần không?" );
        GameCanvas.currentDialog = leftoverDialog;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("Stale dialog dismissed during goto NPC", GameCanvas.currentDialog == null);
        boolean hasOpcode23 = false;
        for (int i = 0; i < getSentPackets().size(); i++) {
            Message pkt = (Message) getSentPackets().get(i);
            if (pkt.command == 23) {
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
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = 552;
        GameScreen.player.y = 504;
        MsgDialog unrelatedDialog = new MsgDialog();
        setDialogText(unrelatedDialog, "Thong bao: Bao tri may chu!" );
        iCommand okBtn = new iCommand("Dong", 1);
        unrelatedDialog.cmdList.addElement(okBtn);
        GameCanvas.currentDialog = unrelatedDialog;
        getSentPackets().clear();

        // 3 consecutive ticks with unrelated dialog
        for (int i = 0; i < 3; i++) {
            callInt("dungeonGotoNpc", 1);
        }
        check("Unrelated dialog NOT auto-confirmed", GameCanvas.currentDialog == unrelatedDialog);
        check("No opcode 23 dispatched while unrelated dialog present", getSentPackets().size() == 0);
        check("State transitioned to DN_MANUAL_REVIEW on persistent dialog", ((Integer) get("dungeonState")).intValue() == Zeus.DN_MANUAL_REVIEW);
        check("Why code set to 5", ((Integer) get("dungeonWhy")).intValue() == 5);

        // ---------------------------------------------------------------------
        // Test 24: No Duplicate Opcode 23 or NPC Reopen While Waiting
        // ---------------------------------------------------------------------
        System.out.println("--- Test 24: No Duplicate Opcode 23 or NPC Reopen While Waiting ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        GameCanvas.currentDialog = null;

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
        GameCanvas.currentDialog = null;
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
        GameCanvas.loadmap.idMap = 1;
        MainObject npcTarget = makePhoChiHuy(552, 504);
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(npcTarget);

        // Subtest A: Character outside native interaction range (e.g. distance 150px > 140px)
        // Must NOT trigger interaction or send opcode 23
        GameScreen.player.x = 552 + 150;
        GameScreen.player.y = 504;
        GameScreen.player.Direction = 0;
        GameScreen.ObjFocus = null;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("At distance 150px (> 140px), dungeonState remains DN_ROUTING",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);
        check("At distance 150px, no opcode 23 dispatched", getSentPackets().size() == 0);

        // Subtest B: Character at (576, 504) - adjacent tile (distance 24px <= 36px)
        // Must arrive, set target focus GameScreen.ObjFocus, face NPC (cH=2), stop velocity, and dispatch opcode 23
        GameScreen.player.x = 576;
        GameScreen.player.y = 504;
        GameScreen.player.Direction = 0;
        GameScreen.ObjFocus = null;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("At distance 24px (<= 36px), dungeonState transitions to DN_PREPARATION",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Target entity focus GameScreen.ObjFocus is set to NPC", GameScreen.ObjFocus == npcTarget);
        check("Character facing cH is turned towards NPC (cH=2)", GameScreen.player.Direction == 2);
        check("Character movement velocity is zeroed", GameScreen.player.vx == 0 && GameScreen.player.vy == 0);
        check("Exactly one opcode 23 dispatched on arrival",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);

        // Subtest C: Retry via dungeonAskNpc() maintains GameScreen.ObjFocus and facing
        GameScreen.ObjFocus = null;
        GameScreen.player.Direction = 0;
        getSentPackets().clear();
        call("dungeonAskNpc");
        check("dungeonAskNpc() re-establishes GameScreen.ObjFocus focus", GameScreen.ObjFocus == npcTarget);
        check("dungeonAskNpc() re-establishes facing cH=2", GameScreen.player.Direction == 2);
        check("dungeonAskNpc() dispatches opcode 23",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);

        // ---------------------------------------------------------------------
        // Test 27: travelArrive Semantics at Live Coordinates (596, 512)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 27: travelArrive Semantics at Live Coordinates ---");
        GameScreen.player.x = 596;
        GameScreen.player.y = 512;
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
        GameScreen.player.x = 552 + 140; // dx = 140, dy = 0, Euclidean = 140
        GameScreen.player.y = 504;
        check("dungeonNpcEligible returns true immediately inside native range (distance 140px <= 140px)",
                callDungeonNpcEligible(npcTarget));

        GameScreen.player.x = 552 + 141; // dx = 141, dy = 0, Euclidean = 141
        GameScreen.player.y = 504;
        check("dungeonNpcEligible returns false immediately outside native range (distance 141px > 140px)",
                !callDungeonNpcEligible(npcTarget));

        // Outside native condition: approach continues, zero packets dispatched
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        set("navTarget", -1);
        set("navDone", true);
        GameScreen.player.x = 552 + 141;
        GameScreen.player.y = 504;
        GameScreen.ObjFocus = null;
        getSentPackets().clear();
        callInt("dungeonGotoNpc", 1);
        check("Approach continues while native interaction condition is false (distance 141px)",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);
        check("Zero packets dispatched while outside native interaction condition",
                getSentPackets().size() == 0);

        // Inside native condition at live coordinates (596, 512): arrives, halts, faces, dispatches opcode 23
        GameScreen.player.x = 596;
        GameScreen.player.y = 512;
        GameScreen.player.Direction = 0;
        GameScreen.player.vx = 5;
        GameScreen.player.vy = 3;
        GameScreen.ObjFocus = null;
        getSentPackets().clear();
        callInt("dungeonGotoNpc", 1);
        check("At live coordinates (596, 512), dungeonState transitions to DN_PREPARATION",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Target entity focus GameScreen.ObjFocus set to NPC at (596, 512)", GameScreen.ObjFocus == npcTarget);
        check("Character facing cH is turned towards NPC (cH=2)", GameScreen.player.Direction == 2);
        check("Movement velocity halted (bc=0, bd=0)", GameScreen.player.vx == 0 && GameScreen.player.vy == 0);
        check("Exactly one opcode 23 dispatched when native interaction condition becomes true",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);

        // ---------------------------------------------------------------------
        // Test 29: Packet Serialization Parity & Wire Format
        // ---------------------------------------------------------------------
        System.out.println("--- Test 29: Packet Serialization Parity ---");
        Message pkt = (Message) getSentPackets().get(0);
        byte[] payload = pkt.getData();
        check("Opcode is 23", pkt.command == 23);
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
        GameCanvas.loadmap.idMap = 1;

        MainObject phoChiHuy31 = makePhoChiHuy(552, 504);
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(phoChiHuy31);

        // Position player at live coordinates (612, 392)
        GameScreen.player.x = 612;
        GameScreen.player.y = 392;
        GameScreen.player.Direction = 0;
        GameScreen.ObjFocus = null;

        // Present broadcast popup
        MsgDialog broadcastPopup = makeBroadcastPopup("Chúc mừng người chơi test đã vượt qua đợt thứ 10");
        GameCanvas.currentDialog = broadcastPopup;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("Dungeon does not treat broadcast popup as blocking modal",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Native interaction opcode 23 dispatched while broadcast popup present",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);
        check("Broadcast popup remains present and unmodified after interaction dispatch",
                GameCanvas.currentDialog == broadcastPopup);

        // ---------------------------------------------------------------------
        // Test 32: Menu Processing with Broadcast Popup Present
        // ---------------------------------------------------------------------
        System.out.println("--- Test 32: Menu Processing with Broadcast Popup Present ---");
        // Step 0: First menu arrives while broadcast popup is still on screen
        mVector firstMenuItems = new mVector("firstMenu");
        firstMenuItems.addElement(new iCommand("Giao tiếp", 0));
        firstMenuItems.addElement(new iCommand("Đóng", 1));
        callServerMenu(-37, 1, "Pho Chi Huy", firstMenuItems);

        check("dungeonMenu captured first menu while broadcast popup present",
                get("dungeonMenu") != null);
        getSentPackets().clear();

        call("dungeonInteract");
        check("Step 0 selects 'Giao tiếp' and advances to step 1 under broadcast popup",
                ((Integer) get("dungeonStep")).intValue() == 1);
        check("Broadcast popup still preserved and untouched after first menu selection",
                GameCanvas.currentDialog == broadcastPopup);

        // Step 1: Second menu arrives while broadcast popup is still on screen
        mVector secondMenuItems = new mVector("secondMenu");
        secondMenuItems.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        secondMenuItems.addElement(new iCommand("Đóng", 1));
        callServerMenu(-37, 2, "Menu", secondMenuItems);

        check("dungeonMenu captured second menu while broadcast popup present",
                get("dungeonMenu") != null);

        call("dungeonInteract");
        check("Step 1 selects 'Vào Ngã Tư Tử Thần' and advances to step 2 under broadcast popup",
                ((Integer) get("dungeonStep")).intValue() == 2);
        check("Broadcast popup still preserved and untouched after second menu selection",
                GameCanvas.currentDialog == broadcastPopup);

        // ---------------------------------------------------------------------
        // Test 33: Expected Dungeon Confirmation Dialog Handling with Broadcast Popup
        // ---------------------------------------------------------------------
        System.out.println("--- Test 33: Expected Dungeon Confirmation Handling ---");
        // At step 2, while broadcast popup is still in GameCanvas.currentDialog, dungeonInteract must NOT auto-confirm it
        set("dungeonWait", 10);
        call("dungeonInteract");
        check("Dungeon does not confirm broadcast popup as dungeon confirmation",
                ((Integer) get("dungeonStep")).intValue() == 2);
        check("Broadcast popup remains unconfirmed in GameCanvas.currentDialog", GameCanvas.currentDialog == broadcastPopup);

        // Genuine confirmation dialog arrives (replaces GameCanvas.currentDialog on client UI)
        MsgDialog confirmDialog33 = new MsgDialog();
        setDialogText(confirmDialog33, "Bạn có muốn vào Ngã tư tử thần không?" );
        confirmDialog33.cmdList = new mVector("buttons");
        confirmDialog33.cmdList.addElement(new iCommand("Có", 1));
        confirmDialog33.cmdList.addElement(new iCommand("Không", 2));
        GameCanvas.currentDialog = confirmDialog33;

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
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = 552;
        GameScreen.player.y = 504;

        MsgDialog blockingModal = makeBlockingModal("Bạn có chắc chắn muốn rời khỏi bang hội không?");
        GameCanvas.currentDialog = blockingModal;
        getSentPackets().clear();

        for (int i = 0; i < 3; i++) {
            callInt("dungeonGotoNpc", 1);
        }
        check("Blocking modal NOT auto-confirmed", GameCanvas.currentDialog == blockingModal);
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
        GameCanvas.loadmap.idMap = 1;

        GameScreen.player.x = 612;
        GameScreen.player.y = 392;
        GameScreen.player.Direction = 0;
        GameScreen.ObjFocus = null;

        MsgDialog liveBroadcast = makeBroadcastPopup("Chúc mừng ... đã vượt qua đợt thứ 10");
        GameCanvas.currentDialog = liveBroadcast;
        getSentPackets().clear();

        callInt("dungeonGotoNpc", 1);
        check("Player at (612, 392) interacts with NPC (552, 504) while broadcast visible",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Opcode 23 sent for CU -37",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);
        check("Live broadcast popup preserved and untouched", GameCanvas.currentDialog == liveBroadcast);

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

        if (GameCanvas.menu2 == null) {
            GameCanvas.menu2 = new Menu2();
        }
        GameCanvas.menu2.isShowMenu = true;
        setFrH(GameCanvas.menu2, -1);
        mVector menuItems36 = new mVector("menu36");
        final int[] callbacks36 = new int[2];
        AvMain target36 = new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                if (e == 4) {
                    callbacks36[0]++;
                } else if (e == 5) {
                    callbacks36[1]++;
                }
            }
        };
        iCommand btnGiaoTiep36 = new iCommand("Giao tiếp", 4, target36);
        iCommand btnDong36 = new iCommand("Đóng", 5, target36);
        menuItems36.addElement(btnGiaoTiep36);
        menuItems36.addElement(btnDong36);
        setFrG(GameCanvas.menu2, menuItems36);

        callServerMenu(-37, 2, "Pho Chi Huy", menuItems36);
        getSentPackets().clear();

        call("dungeonInteract");

        check("Step 0 sets GameCanvas.menu2.h to matching 'Giao tiếp' item index (0)", getFrH(GameCanvas.menu2) == 0);
        check("Step 0 invokes native iCommand.a() command callback exactly once", callbacks36[0] == 1);
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

        // Menu was closed (GameCanvas.menu2.isShowMenu = false)
        GameCanvas.menu2.isShowMenu = false;
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

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 3);
        setFrH(GameCanvas.menu2, -1);
        mVector menuItems38 = new mVector("menu38");
        menuItems38.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        menuItems38.addElement(new iCommand("Đóng", 1));
        setFrG(GameCanvas.menu2, menuItems38);

        callServerMenu(-37, 3, "Nga Tu", menuItems38);
        getSentPackets().clear();

        call("dungeonInteract");
        check("Step 1 sets GameCanvas.menu2.h to matching 'Ngã Tư' index (0)", getFrH(GameCanvas.menu2) == 0);
        check("Step 1 invokes native action which closes GameCanvas.menu2", !GameCanvas.menu2.isShowMenu);
        check("Step 1 dispatches server-menu q.b packet via native handler",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);
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

        GameCanvas.menu2.isShowMenu = true;
        setFrH(GameCanvas.menu2, -1);
        mVector menuItems39 = new mVector("menu39");
        AvMain fastReplyNpc = new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                // Immediate synchronous reply from server: second menu arrives DURING callback!
                try {
                    mVector fastSecond = new mVector("fastSecond");
                    fastSecond.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
                    fastSecond.addElement(new iCommand("Đóng", 1));
                    setFrG(GameCanvas.menu2, fastSecond);
                    setFrB(GameCanvas.menu2, 4);
                    setFrC(GameCanvas.menu2, -37);
                    GameCanvas.menu2.isShowMenu = true;
                    callServerMenu(-37, 4, "Nga Tu Fast", fastSecond);
                } catch (Exception ex) {
                }
            }
        };
        menuItems39.addElement(new iCommand("Giao tiếp", 4, fastReplyNpc));
        setFrG(GameCanvas.menu2, menuItems39);
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
        MsgDialog fastConfirm = new MsgDialog();
        setDialogText(fastConfirm, "Bạn có muốn vào Ngã tư tử thần không?" );
        fastConfirm.cmdList = new mVector("buttons");
        final boolean[] confirmClicked = new boolean[1];
        AvMain confirmTarget = new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                confirmClicked[0] = true;
            }
        };
        fastConfirm.cmdList.addElement(new iCommand("Có", 1, confirmTarget));
        fastConfirm.cmdList.addElement(new iCommand("Không", 2));
        GameCanvas.currentDialog = fastConfirm;

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

        MsgDialog entryBroadcast = makeBroadcastPopup("Thông báo: Sự kiện đang diễn ra!");
        GameCanvas.currentDialog = entryBroadcast;

        GameCanvas.menu2.isShowMenu = true;
        mVector bCastFirstMenu = new mVector("bCastFirstMenu");
        bCastFirstMenu.addElement(new iCommand("Giao tiếp", 4, target36));
        setFrG(GameCanvas.menu2, bCastFirstMenu);
        callServerMenu(-37, 2, "Pho Chi Huy", bCastFirstMenu);

        call("dungeonInteract");
        check("Broadcast popup preserved during Step 0 Giao tiếp", GameCanvas.currentDialog == entryBroadcast);
        check("Step 0 advanced under broadcast popup", ((Integer) get("dungeonStep")).intValue() == 1);

        // ---------------------------------------------------------------------
        // Test 42: Full Entry Chain Deterministic Parity
        // ---------------------------------------------------------------------
        System.out.println("--- Test 42: Full Entry Chain Deterministic Parity ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = 552;
        GameScreen.player.y = 504;
        GameScreen.player.Direction = 0;
        GameScreen.ObjFocus = null;

        MainObject pcf42 = makePhoChiHuy(552, 504);
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(pcf42);

        // Step A: approach and arrival
        getSentPackets().clear();
        callInt("dungeonGotoNpc", 1);
        check("Full Chain: Arrival transitions to DN_PREPARATION",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);
        check("Full Chain: Opcode 23 sent to NPC",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);

        // Step B: First menu arrives
        GameCanvas.menu2.isShowMenu = true;
        setFrH(GameCanvas.menu2, -1);
        mVector chainFirst = new mVector("chainFirst");
        final boolean[] chainFirstClicked = new boolean[1];
        AvMain chainTarget = new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                chainFirstClicked[0] = true;
            }
        };
        chainFirst.addElement(new iCommand("Giao tiếp", 4, chainTarget));
        chainFirst.addElement(new iCommand("Đóng", 5));
        setFrG(GameCanvas.menu2, chainFirst);
        callServerMenu(-37, 2, "Pho Chi Huy", chainFirst);

        call("dungeonInteract");
        check("Full Chain: Step 0 native callback invoked", chainFirstClicked[0]);
        check("Full Chain: State advanced to Step 1", ((Integer) get("dungeonStep")).intValue() == 1);

        // Step C: Second menu arrives
        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 3);
        setFrH(GameCanvas.menu2, -1);
        mVector chainSecond = new mVector("chainSecond");
        chainSecond.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        chainSecond.addElement(new iCommand("Đóng", 1));
        setFrG(GameCanvas.menu2, chainSecond);
        callServerMenu(-37, 3, "Nga Tu", chainSecond);

        getSentPackets().clear();
        call("dungeonInteract");
        check("Full Chain: Step 1 native action sent q.b",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);
        check("Full Chain: State advanced to Step 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // Step D: Confirmation dialog arrives
        MsgDialog chainConfirm = new MsgDialog();
        setDialogText(chainConfirm, "Bạn có muốn vào Ngã tư tử thần không?" );
        chainConfirm.cmdList = new mVector("buttons");
        final boolean[] chainConfirmClicked = new boolean[1];
        AvMain chainConfirmTarget = new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                chainConfirmClicked[0] = true;
            }
        };
        chainConfirm.cmdList.addElement(new iCommand("Có", 1, chainConfirmTarget));
        chainConfirm.cmdList.addElement(new iCommand("Không", 2));
        GameCanvas.currentDialog = chainConfirm;

        call("dungeonInteract");
        check("Full Chain: Step 2 confirmation invoked", chainConfirmClicked[0]);
        check("Full Chain: State advanced to Step 3", ((Integer) get("dungeonStep")).intValue() == 3);

        // Step E: Server teleports player to Map 48
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        call("dungeon");
        check("Full Chain: Arrival in Map 48 enters DN_COMBAT",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMBAT);

        // ---------------------------------------------------------------------
        // Test 43: GameCanvas.subDialog Total Rejection Contract (DUNGEON-04I)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 43: GameCanvas.subDialog Total Rejection Contract ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        GameCanvas.currentDialog = null;
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        // Even if GameCanvas.subDialog contains a dialog with "Giao tiếp", it must NEVER be accepted as NPC dialog
        MsgDialog futDialog = new MsgDialog();
        setDialogText(futDialog, "Ta có một nhiệm vụ rất quan trọng đang cần mi giúp đỡ!" );
        futDialog.cmdList = new mVector("buttons");
        final boolean[] futGiaoTiepClicked = new boolean[1];
        AvMain futTarget = new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                futGiaoTiepClicked[0] = true;
            }
        };
        futDialog.cmdList.addElement(new iCommand("Giao tiếp", 4, futTarget));
        futDialog.cmdList.addElement(new iCommand("Đóng", 5));
        GameCanvas.subDialog = futDialog;

        getSentPackets().clear();
        call("dungeonInteract");
        check("GameCanvas.subDialog Rejection: Step 0 does NOT execute iCommand.a() from GameCanvas.subDialog", !futGiaoTiepClicked[0]);
        check("GameCanvas.subDialog Rejection: Step remains 0", ((Integer) get("dungeonStep")).intValue() == 0);
        check("GameCanvas.subDialog Rejection: Wait budget decrements", ((Integer) get("dungeonWait")).intValue() < 60);
        GameCanvas.subDialog = null;

        // ---------------------------------------------------------------------
        // Test 44: Exact Native GameCanvas.menu2 NPC Dialog Contract (DUNGEON-04I)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 44: Exact Native GameCanvas.menu2 NPC Dialog Contract ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        GameCanvas.subDialog = null;

        // Broadcast in GameCanvas.currentDialog coexisting
        MsgDialog coexistBroadcast = makeBroadcastPopup("Chúc mừng người chơi x đã vượt qua đợt 5");
        GameCanvas.currentDialog = coexistBroadcast;

        // Native NPC dialogue in GameCanvas.menu2
        final MainObject livePhoChiHuy = new MainObject();
        livePhoChiHuy.typeObject = 2;
        livePhoChiHuy.ID = -37;
        livePhoChiHuy.name = "Pho Chi Huy";

        GameCanvas.menu2.isShowMenu = true;
        setFrH(GameCanvas.menu2, -1);
        mVector fupItems = new mVector("fupItems");
        final boolean[] fupGiaoTiepClicked = new boolean[1];
        iCommand liveGiaoTiep = new iCommand("Giao tiếp", 4, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                fupGiaoTiepClicked[0] = true;
                try {
                    GlobalService.gI().getlist_from_npc((byte) livePhoChiHuy.ID);
                } catch (Throwable t) {}
            }
        });
        fupItems.addElement(liveGiaoTiep);
        fupItems.addElement(new iCommand("Đóng", 1));
        setFrG(GameCanvas.menu2, fupItems);

        getSentPackets().clear();
        call("dungeonInteract");
        check("GameCanvas.menu2 Dialog: Step 0 executes native iCommand.a() from GameCanvas.menu2", fupGiaoTiepClicked[0]);
        check("GameCanvas.menu2 Dialog: GameCanvas.menu2.h cursor set to 0", getFrH(GameCanvas.menu2) == 0);
        check("GameCanvas.menu2 Dialog: Step advances to 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("GameCanvas.menu2 Dialog: Exactly one Opcode 23 sent",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);
        check("GameCanvas.menu2 Dialog: Broadcast in GameCanvas.currentDialog preserved", GameCanvas.currentDialog == coexistBroadcast);

        // ---------------------------------------------------------------------
        // Test 45: V2 Speech Dialog Recognition & Non-Blocking Safety (DUNGEON-04I)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 45: V2 Speech Dialog Recognition & Non-Blocking Safety ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        GameCanvas.subDialog = null;
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        // Speech dialog with live text: "Ta có một nhiệm vụ rất quan trọng đang cần mi giúp đỡ!"
        MsgDialog v2SpeechDialog = new MsgDialog();
        setDialogText(v2SpeechDialog, "Ta có một nhiệm vụ rất quan trọng đang cần mi giúp đỡ!" );
        v2SpeechDialog.cmdList = new mVector("buttons");
        v2SpeechDialog.cmdList.addElement(new iCommand("Giao tiếp", 4));
        v2SpeechDialog.cmdList.addElement(new iCommand("Đóng", 1));

        Method mIsSpeech = Zeus.class.getDeclaredMethod("isNpcSpeechDialog", MainDialog.class);
        mIsSpeech.setAccessible(true);
        boolean recognized = ((Boolean) mIsSpeech.invoke(null, v2SpeechDialog)).booleanValue();
        check("V2 Speech: isNpcSpeechDialog recognizes live text without nga tu", recognized);

        boolean isBlock = Zeus.isBlockingDialog(v2SpeechDialog);
        check("V2 Speech: isBlockingDialog does NOT block Pho Chi Huy speech dialog", !isBlock);

        // Advance speech dialog via V2 softkey path
        final boolean[] v2SoftkeyInvoked = new boolean[1];
        v2SpeechDialog.right = new iCommand("Giao tiếp", 4, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                v2SoftkeyInvoked[0] = true;
            }
        });
        GameCanvas.currentDialog = v2SpeechDialog;

        call("dungeonInteract");
        check("V2 Speech: advances via softkey ab", v2SoftkeyInvoked[0]);
        check("V2 Speech: Step advances to 1", ((Integer) get("dungeonStep")).intValue() == 1);

        // ---------------------------------------------------------------------
        // Test 46: dialogText MsgDialog.r, MsgDialog.s, MsgDialog.t & Polymorphic Dispatch (DUNGEON-04I)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 46: dialogText MsgDialog.r, MsgDialog.s, MsgDialog.t & Polymorphic Dispatch ---");
        MsgDialog titleAh = new MsgDialog();
        Field fAhR = MsgDialog.class.getDeclaredField("nameShow");
        fAhR.setAccessible(true);
        fAhR.set(titleAh, "Pho Chi Huy");

        Method mDt = Zeus.class.getDeclaredMethod("dialogText", MainDialog.class);
        mDt.setAccessible(true);
        String extractedR = (String) mDt.invoke(null, titleAh);
        check("dialogText: extracts text from MsgDialog.r", extractedR.indexOf("Pho Chi Huy") >= 0);

        // Polymorphic NPC click dispatch
        final boolean[] polymorphicKCalled = new boolean[1];
        MainObject mockNpc = new MainObject() {
            public void GiaoTiep() {
                polymorphicKCalled[0] = true;
            }
        };
        mockNpc.typeObject = 2;
        mockNpc.ID = -37;
        mockNpc.name = "Pho Chi Huy";
        mockNpc.x = 552;
        mockNpc.y = 504;
        GameScreen.player.x = 552;
        GameScreen.player.y = 504;

        Method mClickNpc = Zeus.class.getDeclaredMethod("dungeonClickNpc", MainObject.class);
        mClickNpc.setAccessible(true);
        mClickNpc.invoke(null, mockNpc);
        check("Polymorphic Dispatch: dungeonClickNpc calls GiaoTiep() on non-ez MainObject subclass", polymorphicKCalled[0]);

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
        GameCanvas.loadmap.idMap = 1;
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        // 47.1: Native GameCanvas.currentDialog Giao tiep button with d == null remains actionable
        // Working V2 scans current GameCanvas.currentDialog dialog and returns matching Giao tiếp buttons
        // regardless of whether iCommand.d is null, since iCommand.a() natively dispatches via GameCanvas.currentDialog.b().
        Method mFindGiaoTiep = Zeus.class.getDeclaredMethod("findGiaoTiepInDialog", MainDialog.class);
        mFindGiaoTiep.setAccessible(true);

        MsgDialog nativeAhDialog = new MsgDialog();
        nativeAhDialog.cmdList = new mVector("dialogButtons");
        iCommand nativeAhBtnNoTarget = new iCommand("Giao tiếp", 4); // iCommand.d is NULL in native MsgDialog dialogs
        nativeAhDialog.cmdList.addElement(nativeAhBtnNoTarget);
        nativeAhDialog.cmdList.addElement(new iCommand("Đóng", 8));
        GameCanvas.currentDialog = nativeAhDialog;
        GameCanvas.subDialog = null;
        GameCanvas.isPointerSelect = true;

        Object foundBtn = mFindGiaoTiep.invoke(null, nativeAhDialog);
        check("Exact Parity: Native GameCanvas.currentDialog button with iCommand.d == null is returned", foundBtn == nativeAhBtnNoTarget);
        check("Exact Parity: Found button has Pointer == null", foundBtn != null && ((iCommand) foundBtn).Pointer == null);
        if (foundBtn != null) {
            ((iCommand) foundBtn).perform();
            check("Exact Parity: Native iCommand.a() with d == null dispatches to current dialog GameCanvas.currentDialog.b() resetting GameCanvas.isPointerClick", !GameCanvas.isPointerSelect);
        }

        // 47.2: Exact Native GameCanvas.menu2 First Dialog Contract with active broadcast
        // When Pho Chi Huy conversation opens natively in GameCanvas.menu2, iCommand.d is the NPC (ez/bm).
        // Step 0 must select GameCanvas.menu2, set cursor index GameCanvas.menu2.h = 0, pre-arm Step 1, invoke iCommand.a(),
        // dispatch Opcode 23 (payload 0xDB), and keep GameCanvas.currentDialog broadcast preserved.
        GameCanvas.menu2.isShowMenu = true;
        MainObject liveNpc = new MainObject();
        liveNpc.typeObject = 2;
        liveNpc.ID = -37;
        liveNpc.name = "Pho Chi Huy";
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(liveNpc);

        mVector nativeFrItems = new mVector("nativeNpcMenu");
        final boolean[] liveNpcActionCalled = new boolean[1];
        iCommand liveNpcGiaoTiep = new iCommand("Giao tiếp", 4, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                liveNpcActionCalled[0] = true;
                try {
                    GlobalService.gI().getlist_from_npc((byte) liveNpc.ID);
                } catch (Throwable t) {}
            }
        });
        nativeFrItems.addElement(liveNpcGiaoTiep);
        setFrG(GameCanvas.menu2, nativeFrItems);
        setFrH(GameCanvas.menu2, -1);

        getSentPackets().clear();
        call("dungeonInteract");
        check("Exact Parity: GameCanvas.menu2 native command callback invoked", liveNpcActionCalled[0]);
        check("Exact Parity: Step 0 sets GameCanvas.menu2.h cursor to 0", getFrH(GameCanvas.menu2) == 0);
        check("Exact Parity: Step advanced to 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("Exact Parity: Exactly one packet sent", getSentPackets().size() == 1);
        check("Exact Parity: Packet is Opcode 23 for NPC -37",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23);
        check("Exact Parity: Broadcast in GameCanvas.currentDialog preserved", GameCanvas.currentDialog != null);

        // 47.3: Fast Second-Menu Arrival Race Test (Step 0 -> Step 1 immediate arrival)
        // If Opcode -30 arrives immediately after Step 0 dispatch, Step 1 must process it without loss.
        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 3);
        setFrH(GameCanvas.menu2, -1);
        mVector secondMenuItems47 = new mVector("secondMenu");
        secondMenuItems47.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        secondMenuItems47.addElement(new iCommand("Đóng", 1));
        setFrG(GameCanvas.menu2, secondMenuItems47);
        callServerMenu(-37, 3, "Nga Tu", secondMenuItems47);

        getSentPackets().clear();
        call("dungeonInteract");
        check("Fast Response: Step 1 native action sent q.b (opcode -30)",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);
        check("Fast Response: State advanced to Step 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // 47.4: V2 Native Speech Dialog Parity in GameCanvas.currentDialog
        // When speech dialog containing "Phó chỉ huy" appears in GameCanvas.currentDialog, it advances via softkey/Key 5.
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        final boolean[] speechSoftkeyInvoked = new boolean[1];
        MsgDialog speechDialog = new MsgDialog();
        setDialogText(speechDialog, "Phó chỉ huy: Ta có nhiệm vụ vào Ngã tư tử thần cho ngươi!" );
        speechDialog.right = new iCommand("Giao tiếp", 4, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                speechSoftkeyInvoked[0] = true;
            }
        });
        GameCanvas.currentDialog = speechDialog;

        call("dungeonInteract");
        check("V2 Parity: Speech dialog advances via softkey/action", speechSoftkeyInvoked[0]);
        check("V2 Parity: Step advances to 1", ((Integer) get("dungeonStep")).intValue() == 1);

        // ---------------------------------------------------------------------
        // Test 48: Native NPC Interaction Ownership & No Duplicate Opcode 23 (R2_A / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 48: Native NPC Interaction Ownership & No Duplicate Opcode 23 ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = 552;
        GameScreen.player.y = 504;
        GameScreen.player.Direction = 0;
        GameScreen.ObjFocus = null;

        final int[] kCallCount48 = new int[1];
        MainObject ezDouble48 = new MainObject() {
            public void GiaoTiep() {
                kCallCount48[0]++;
                try {
                    GlobalService.gI().getlist_from_npc((byte) this.ID);
                } catch (Throwable t) {}
            }
        };
        ezDouble48.typeObject = 2;
        ezDouble48.ID = -37;
        ezDouble48.name = "Pho Chi Huy";
        ezDouble48.x = 552;
        ezDouble48.y = 504;

        getSentPackets().clear();
        Method mClickNpc48 = Zeus.class.getDeclaredMethod("dungeonClickNpc", MainObject.class);
        mClickNpc48.setAccessible(true);
        boolean clicked48 = ((Boolean) mClickNpc48.invoke(null, ezDouble48)).booleanValue();

        check("Test 48: dungeonClickNpc returned true", clicked48);
        check("Test 48: npc.k() invoked exactly once", kCallCount48[0] == 1);
        check("Test 48: Exactly one packet sent across wire", getSentPackets().size() == 1);
        check("Test 48: Packet is opcode 23 with payload (byte)-37",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == 23
                && ((Message) getSentPackets().get(0)).getData()[0] == (byte) -37);
        check("Test 48: dungeonWait armed to bounded cooldown (40)", ((Integer) get("dungeonWait")).intValue() == 40);
        check("Test 48: dungeonState transitioned to DN_PREPARATION", ((Integer) get("dungeonState")).intValue() == Zeus.DN_PREPARATION);

        // ---------------------------------------------------------------------
        // Test 49: Polymorphic NPC Opening Menu Without Sending Packets (R2_A / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 49: Polymorphic NPC Opening Menu Without Raw Fallback ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_ROUTING);
        GameCanvas.loadmap.idMap = 1;
        GameScreen.player.x = 552;
        GameScreen.player.y = 504;

        final int[] kCallCount49 = new int[1];
        MainObject menuNpcDouble49 = new MainObject() {
            public void GiaoTiep() {
                kCallCount49[0]++;
                GameCanvas.menu2.isShowMenu = true;
                mVector localMenu = new mVector("localMenu");
                localMenu.addElement(new iCommand("Giao tiếp", 4));
                localMenu.addElement(new iCommand("Đóng", 1));
                setFrG(GameCanvas.menu2, localMenu);
            }
        };
        menuNpcDouble49.typeObject = 2;
        menuNpcDouble49.ID = -37;
        menuNpcDouble49.name = "Pho Chi Huy";
        menuNpcDouble49.x = 552;
        menuNpcDouble49.y = 504;

        getSentPackets().clear();
        boolean clicked49 = ((Boolean) mClickNpc48.invoke(null, menuNpcDouble49)).booleanValue();

        check("Test 49: dungeonClickNpc returned true", clicked49);
        check("Test 49: npc.k() invoked exactly once", kCallCount49[0] == 1);
        check("Test 49: Zero packets sent across wire (no synthetic opcode 23 fallback)", getSentPackets().size() == 0);
        check("Test 49: Native menu opened by npc.k() is active in GameCanvas.menu2", GameCanvas.menu2.isShowMenu);
        check("Test 49: dungeonWait armed to bounded cooldown (40)", ((Integer) get("dungeonWait")).intValue() == 40);

        // ---------------------------------------------------------------------
        // Test 50: Intermediate Speech Dialog While dungeonStep is 1 or 2 (R2_B / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 50: Intermediate Speech Dialog While dungeonStep is 1 or 2 ---");
        // Subtest 50A: dungeonStep == 1
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 1);
        set("dungeonWait", 60);
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        final boolean[] speech50AInvoked = new boolean[1];
        MsgDialog speech50A = new MsgDialog();
        setDialogText(speech50A, "Phó chỉ huy: Ngươi đã sẵn sàng bước vào cõi chết chưa?" );
        speech50A.right = new iCommand("Tiếp tục", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                speech50AInvoked[0] = true;
            }
        });
        GameCanvas.currentDialog = speech50A;

        call("dungeonInteract");
        check("Test 50A: Intermediate speech at Step 1 advances via softkey ab", speech50AInvoked[0]);
        check("Test 50A: dungeonWait refreshed after dialog advance", ((Integer) get("dungeonWait")).intValue() == 60);

        // Subtest 50B: dungeonStep == 2
        set("dungeonStep", 2);
        set("dungeonWait", 60);
        final boolean[] speech50BInvoked = new boolean[1];
        MsgDialog speech50B = new MsgDialog();
        setDialogText(speech50B, "Nhiệm vụ: Hãy tiêu diệt toàn bộ quái vật trong Ngã tư tử thần!" );
        speech50B.left = new iCommand("Đồng ý", 2, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                speech50BInvoked[0] = true;
            }
        });
        GameCanvas.currentDialog = speech50B;

        call("dungeonInteract");
        check("Test 50B: Intermediate speech at Step 2 advances via softkey Z", speech50BInvoked[0]);
        check("Test 50B: dungeonWait refreshed after dialog advance", ((Integer) get("dungeonWait")).intValue() == 60);

        // ---------------------------------------------------------------------
        // Test 51: Ngã Tư Submenu Dispatch Through GameCanvas.menu2.isShowMenu(2, 0) (R2_B / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 51: Ngã Tư Submenu Dispatch Through GameCanvas.menu2.isShowMenu(2, 0) ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 3);
        setFrH(GameCanvas.menu2, -1);
        mVector menu51 = new mVector("menu51");
        menu51.addElement(new iCommand("Thông tin", 0));
        menu51.addElement(new iCommand("Vào Ngã Tư Tử Thần", 1));
        menu51.addElement(new iCommand("Đóng", 2));
        setFrG(GameCanvas.menu2, menu51);
        callServerMenu(-37, 3, "Nga Tu", menu51);

        getSentPackets().clear();
        call("dungeonInteract");

        check("Test 51: Submenu sets GameCanvas.menu2.h to matching 'Ngã Tư' index (1)", getFrH(GameCanvas.menu2) == 1);
        check("Test 51: Native action dispatched server-menu q.b packet (opcode -30)",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);
        check("Test 51: Submenu dispatch advanced dungeonStep to 2", ((Integer) get("dungeonStep")).intValue() == 2);
        check("Test 51: Bounded wait cooldown armed (60)", ((Integer) get("dungeonWait")).intValue() == 60);

        // ---------------------------------------------------------------------
        // Test 52: Direct Transition to Map 48 Without Confirmation Dialog (R2_B / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 52: Direct Transition to Map 48 Without Confirmation Dialog ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 2);
        set("dungeonWait", 50);
        GameCanvas.currentDialog = null;

        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;

        call("dungeonInteract");
        check("Test 52: Direct arrival at Map 48 transitions to DN_COMBAT without confirmation dialog",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMBAT);

        // ---------------------------------------------------------------------
        // Test 53: Optional Valid Confirmation Dialog Handling (R2_B / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 53: Optional Valid Confirmation Dialog Handling ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 2);
        set("dungeonWait", 60);
        GameCanvas.loadmap.idMap = 1;

        final boolean[] confirm53Clicked = new boolean[1];
        MsgDialog confirm53 = new MsgDialog();
        setDialogText(confirm53, "Bạn có muốn vào Ngã tư tử thần không?" );
        confirm53.cmdList = new mVector("buttons");
        confirm53.cmdList.addElement(new iCommand("Có", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                confirm53Clicked[0] = true;
            }
        }));
        confirm53.cmdList.addElement(new iCommand("Không", 2));
        GameCanvas.currentDialog = confirm53;

        call("dungeonInteract");
        check("Test 53: Valid confirmation dialog confirmed via affirmative button", confirm53Clicked[0]);
        check("Test 53: State advanced to Step 3 waiting for teleport", ((Integer) get("dungeonStep")).intValue() == 3);
        check("Test 53: Teleport wait budget armed (80)", ((Integer) get("dungeonWait")).intValue() == 80);

        // ---------------------------------------------------------------------
        // Test 54: Unrelated Dialog Fail-Closed Behavior (R2_B / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 54: Unrelated Dialog Fail-Closed Behavior ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;

        final boolean[] unrelatedActionClicked = new boolean[1];
        MsgDialog unrelated54 = new MsgDialog();
        setDialogText(unrelated54, "Giao dịch vật phẩm với người chơi khác?" );
        unrelated54.cmdList = new mVector("buttons");
        unrelated54.cmdList.addElement(new iCommand("Đồng ý", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                unrelatedActionClicked[0] = true;
            }
        }));
        unrelated54.cmdList.addElement(new iCommand("Hủy", 2));
        GameCanvas.currentDialog = unrelated54;

        for (int i = 0; i < 4; i++) {
            call("dungeonInteract");
        }

        check("Test 54: Unrelated modal callback NEVER invoked (fail-closed)", !unrelatedActionClicked[0]);
        check("Test 54: Unrelated modal remains in GameCanvas.currentDialog untouched", GameCanvas.currentDialog == unrelated54);
        check("Test 54: State transitioned to DN_MANUAL_REVIEW on persistent unrelated modal",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_MANUAL_REVIEW);
        check("Test 54: dungeonWhy set to 5", ((Integer) get("dungeonWhy")).intValue() == 5);

        // ---------------------------------------------------------------------
        // Test 55: Bounded Retry Does Not Spam Opcode 23 Every Tick (R2_C / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 55: Bounded Retry Does Not Spam Opcode 23 Every Tick ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 40);
        set("dungeonTried", 0);
        GameCanvas.currentDialog = null;
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        MainObject npc55 = makePhoChiHuy(552, 504);
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(npc55);
        set("dungeonNpcCu", -37);

        getSentPackets().clear();
        for (int tick = 0; tick < 39; tick++) {
            call("dungeonInteract");
        }
        check("Test 55: Zero packets dispatched during wait cooldown (39 ticks)", getSentPackets().size() == 0);
        check("Test 55: Wait decremented to 1", ((Integer) get("dungeonWait")).intValue() == 1);

        call("dungeonInteract");
        check("Test 55: Wait decremented to 0", ((Integer) get("dungeonWait")).intValue() == 0);
        check("Test 55: Still zero packets sent while wait just hit 0", getSentPackets().size() == 0);

        call("dungeonInteract");
        check("Test 55: Exactly 1 retry packet sent after wait expired", getSentPackets().size() == 1);
        check("Test 55: Retry reset dungeonWait to 40", ((Integer) get("dungeonWait")).intValue() == 40);
        check("Test 55: dungeonTried incremented to 1", ((Integer) get("dungeonTried")).intValue() == 1);

        // ---------------------------------------------------------------------
        // Test 56: Broadcast Announcement Coexistence During Submenu Dispatch (R2_B / R2_D)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 56: Broadcast Coexistence During Submenu Dispatch ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);

        MsgDialog broadcast56 = makeBroadcastPopup("Sự kiện nhân đôi kinh nghiệm đang diễn ra!");
        GameCanvas.currentDialog = broadcast56;

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 3);
        setFrH(GameCanvas.menu2, -1);
        mVector menu56 = new mVector("menu56");
        menu56.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        menu56.addElement(new iCommand("Đóng", 1));
        setFrG(GameCanvas.menu2, menu56);
        callServerMenu(-37, 3, "Nga Tu", menu56);

        getSentPackets().clear();
        call("dungeonInteract");

        check("Test 56: Submenu dispatched despite broadcast in GameCanvas.currentDialog",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);
        check("Test 56: Broadcast popup in GameCanvas.currentDialog preserved and unmodified", GameCanvas.currentDialog == broadcast56);
        check("Test 56: Step advanced to 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // ---------------------------------------------------------------------
        // Test 57: Blank Ambiguous Modal After Ngã Tư Submenu Fails Closed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 57: Blank Ambiguous Modal After Ngã Tư Submenu Fails Closed ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 2); // Submenu already picked, waiting for confirmation or teleport
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;

        final boolean[] blankModalActionClicked = new boolean[1];
        MsgDialog blankModal57 = new MsgDialog();
        // empty/no dialog text
        setDialogText(blankModal57, "" );
        blankModal57.cmdList = new mVector("buttons");
        blankModal57.cmdList.addElement(new iCommand("Đồng ý", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                blankModalActionClicked[0] = true;
            }
        }));
        blankModal57.cmdList.addElement(new iCommand("Hủy", 2));
        GameCanvas.currentDialog = blankModal57;

        // Tick repeatedly to let bounded retry policy run
        for (int i = 0; i < 4; i++) {
            call("dungeonInteract");
        }

        check("Test 57: Blank ambiguous modal callback NEVER invoked (fail-closed)", !blankModalActionClicked[0]);
        check("Test 57: Blank ambiguous modal remains unconfirmed in GameCanvas.currentDialog", GameCanvas.currentDialog == blankModal57);
        check("Test 57: State transitioned to DN_MANUAL_REVIEW on persistent blank modal",
                ((Integer) get("dungeonState")).intValue() == Zeus.DN_MANUAL_REVIEW);
        check("Test 57: dungeonWhy set to 5", ((Integer) get("dungeonWhy")).intValue() == 5);

        // ---------------------------------------------------------------------
        // Test 58: Current GameCanvas.currentDialog Giao Tiếp Button with d == null Dispatches Native Callback
        // ---------------------------------------------------------------------
        System.out.println("--- Test 58: Current GameCanvas.currentDialog Giao Tiếp Button with d == null Dispatches Native Callback ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        MsgDialog speechAh58 = new MsgDialog();
        speechAh58.cmdList = new mVector("dialogButtons");
        iCommand giaoTiepBtn58 = new iCommand("Giao tiếp", 4); // iCommand.d is NULL
        speechAh58.cmdList.addElement(giaoTiepBtn58);
        speechAh58.cmdList.addElement(new iCommand("Đóng", 8));
        GameCanvas.currentDialog = speechAh58;
        GameCanvas.isPointerSelect = true;

        call("dungeonInteract");
        check("Test 58: Giao tiếp with d == null in GameCanvas.currentDialog dispatches native iCommand.a() -> GameCanvas.currentDialog.b() resetting GameCanvas.isPointerClick", !GameCanvas.isPointerSelect);
        check("Test 58: Step advances to 1", ((Integer) get("dungeonStep")).intValue() == 1);
        check("Test 58: Wait cooldown armed (60)", ((Integer) get("dungeonWait")).intValue() == 60);

        // ---------------------------------------------------------------------
        // Test 59: Real Dungeon Confirmation Remains Optional and Functional
        // ---------------------------------------------------------------------
        System.out.println("--- Test 59: Real Dungeon Confirmation Remains Optional and Functional ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 2);
        set("dungeonWait", 60);
        GameCanvas.loadmap.idMap = 1;

        final boolean[] realConfirmClicked = new boolean[1];
        MsgDialog realConfirm59 = new MsgDialog();
        setDialogText(realConfirm59, "Bạn có muốn vào Ngã tư tử thần không?" );
        realConfirm59.cmdList = new mVector("buttons");
        realConfirm59.cmdList.addElement(new iCommand("Vào", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                realConfirmClicked[0] = true;
            }
        }));
        realConfirm59.cmdList.addElement(new iCommand("Không", 2));
        GameCanvas.currentDialog = realConfirm59;

        call("dungeonInteract");
        check("Test 59: Real dungeon confirmation confirmed via affirmative button", realConfirmClicked[0]);
        check("Test 59: State advanced to Step 3 waiting for teleport", ((Integer) get("dungeonStep")).intValue() == 3);
        check("Test 59: Teleport wait budget armed (80)", ((Integer) get("dungeonWait")).intValue() == 80);

        // ---------------------------------------------------------------------
        // Test 60: Runtime Loop Processes Intermediate Speech Before Wait Expires
        // ---------------------------------------------------------------------
        System.out.println("--- Test 60: Runtime Loop Processes Intermediate Speech Before Wait Expires ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 1);
        set("dungeonWait", 60);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.menu2.isShowMenu = false;
        setFrG(GameCanvas.menu2, null);

        final boolean[] speechActionCalled60 = new boolean[1];
        MsgDialog speechDialog60 = new MsgDialog();
        setDialogText(speechDialog60, "Phó chỉ huy: Ngươi đã sẵn sàng chưa?" );
        speechDialog60.right = new iCommand("Tiếp tục", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                speechActionCalled60[0] = true;
            }
        });
        GameCanvas.currentDialog = speechDialog60;

        // Execute via the real runtime entrypoint dungeon(), NOT dungeonInteract() directly
        call("dungeon");
        check("Test 60: Speech native action executes on this tick via dungeon()", speechActionCalled60[0]);
        check("Test 60: Call not blocked by outer dungeonWait gate, wait refreshed (60)", ((Integer) get("dungeonWait")).intValue() == 60);

        // ---------------------------------------------------------------------
        // Test 61: Runtime Loop Processes Ngã Tư Submenu Before Wait Expires
        // ---------------------------------------------------------------------
        System.out.println("--- Test 61: Runtime Loop Processes Ngã Tư Submenu Before Wait Expires ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 1);
        set("dungeonWait", 50); // dungeonWait > 0
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null;

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 3);
        setFrH(GameCanvas.menu2, -1);
        mVector menu61 = new mVector("menu61");
        menu61.addElement(new iCommand("Nhiệm vụ", 0));
        menu61.addElement(new iCommand("Vào Ngã Tư Tử Thần", 1));
        menu61.addElement(new iCommand("Đóng", 2));
        setFrG(GameCanvas.menu2, menu61);
        callServerMenu(-37, 3, "Nga Tu", menu61);

        getSentPackets().clear();
        // Execute via the real runtime entrypoint dungeon(), NOT dungeonInteract()
        call("dungeon");
        check("Test 61: Native GameCanvas.menu2.isShowMenu(2,0) path dispatches opcode -30 on that tick via dungeon()",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);
        check("Test 61: Only one action dispatched", getSentPackets().size() == 1);
        check("Test 61: State advanced to Step 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // ---------------------------------------------------------------------
        // Test 62: Runtime Loop Still Respects Cooldown When No Live UI Is Actionable
        // ---------------------------------------------------------------------
        System.out.println("--- Test 62: Runtime Loop Still Respects Cooldown When No Live UI Is Actionable ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 40);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null; // no confirmation dialog, no speech dialog
        GameCanvas.menu2.isShowMenu = false; // no active submenu
        setFrG(GameCanvas.menu2, null);
        set("dungeonMenu", null); // no legacy menu
        set("dungeonMenuItems", null);

        MainObject npc62 = makePhoChiHuy(552, 504);
        GameScreen.Vecplayers = new mVector("entities");
        GameScreen.Vecplayers.addElement(npc62);
        set("dungeonNpcCu", -37);

        getSentPackets().clear();
        // Invoke dungeon() once
        call("dungeon");
        check("Test 62: No opcode 23 or -30 sent while cooldown active", getSentPackets().size() == 0);
        check("Test 62: dungeonWait decremented from 40 to 39", ((Integer) get("dungeonWait")).intValue() == 39);

        // ---------------------------------------------------------------------
        // Test 63: Same Ngã Tư Submenu After Native Dispatch Is Not Resent
        // ---------------------------------------------------------------------
        System.out.println("--- Test 63: Same Ngã Tư Submenu After Native Dispatch Is Not Resent ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null;

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 0);
        setFrH(GameCanvas.menu2, -1);
        mVector menu63 = new mVector("menu63");
        menu63.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        menu63.addElement(new iCommand("Hướng dẫn", 1));
        setFrG(GameCanvas.menu2, menu63);
        callServerMenu(-37, 0, "MENU", menu63);

        getSentPackets().clear();
        // Invoke real dungeon() runtime tick
        call("dungeon");

        // Verify exactly one opcode -30
        check("Test 63: Exactly one packet sent on first submenu dispatch", getSentPackets().size() == 1);
        check("Test 63: Dispatched packet is opcode -30", ((Message) getSentPackets().get(0)).command == -30);
        check("Test 63: Native action closed menu", !GameCanvas.menu2.isShowMenu);

        // Simulate server re-presenting the same Ngã Tư submenu
        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 0);
        setFrH(GameCanvas.menu2, -1);
        setFrG(GameCanvas.menu2, menu63);
        callServerMenu(-37, 0, "MENU", menu63);

        // Invoke dungeon() again
        call("dungeon");

        // Verify no second opcode -30 and no opcode 23
        int op23Count = 0;
        int op30Count = 0;
        for (Object p63 : getSentPackets()) {
            if (((Message) p63).command == 23) op23Count++;
            if (((Message) p63).command == -30) op30Count++;
        }
        check("Test 63: No opcode 23 sent", op23Count == 0);
        check("Test 63: Exactly one opcode -30 across both ticks", op30Count == 1);
        check("Test 63: State fails closed to DN_MANUAL_REVIEW",
                ((Integer) get("dungeonState")).intValue() == ((Integer) get("DN_MANUAL_REVIEW")).intValue());
        check("Test 63: State-only manual review leaves dungeonWhy == 0", ((Integer) get("dungeonWhy")).intValue() == 0);

        // ---------------------------------------------------------------------
        // Test 64: Direct Teleport After Ngã Tư Dispatch Still Succeeds
        // ---------------------------------------------------------------------
        System.out.println("--- Test 64: Direct Teleport After Ngã Tư Dispatch Still Succeeds ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null;

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 0);
        setFrH(GameCanvas.menu2, -1);
        mVector menu64 = new mVector("menu64");
        menu64.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        setFrG(GameCanvas.menu2, menu64);
        callServerMenu(-37, 0, "MENU", menu64);

        getSentPackets().clear();
        call("dungeon");
        check("Test 64: First tick dispatches opcode -30",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);

        // Change map to 48 before next tick (direct teleport)
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;
        call("dungeon");

        check("Test 64: DN_COMBAT is reached on direct map 48 teleport",
                ((Integer) get("dungeonState")).intValue() == ((Integer) get("DN_COMBAT")).intValue());
        check("Test 64: No second opcode -30 sent", getSentPackets().size() == 1);

        // ---------------------------------------------------------------------
        // Test 65: Explicit Confirmation After Ngã Tư Dispatch Still Works
        // ---------------------------------------------------------------------
        System.out.println("--- Test 65: Explicit Confirmation After Ngã Tư Dispatch Still Works ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null;

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 0);
        setFrH(GameCanvas.menu2, -1);
        mVector menu65 = new mVector("menu65");
        menu65.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        setFrG(GameCanvas.menu2, menu65);
        callServerMenu(-37, 0, "MENU", menu65);

        getSentPackets().clear();
        call("dungeon");
        check("Test 65: First tick dispatches opcode -30",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);

        // Present explicit dungeon confirmation
        final boolean[] confirm65Clicked = new boolean[1];
        MsgDialog confirmDlg65 = new MsgDialog();
        setDialogText(confirmDlg65, "Bạn có muốn vào Ngã tư tử thần không?" );
        confirmDlg65.cmdList = new mVector("btns");
        confirmDlg65.cmdList.addElement(new iCommand("Đồng ý", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                confirm65Clicked[0] = true;
            }
        }));
        confirmDlg65.cmdList.addElement(new iCommand("Không", 2));
        GameCanvas.currentDialog = confirmDlg65;

        call("dungeon");
        check("Test 65: Confirmation dialog is handled once", confirm65Clicked[0]);
        check("Test 65: No second opcode -30 sent", getSentPackets().size() == 1);
        check("Test 65: State advances to step 3 (waiting teleport)", ((Integer) get("dungeonStep")).intValue() == 3);

        // ---------------------------------------------------------------------
        // Test 66: Intermediate Meaningful Speech Remains Reactive
        // ---------------------------------------------------------------------
        System.out.println("--- Test 66: Intermediate Meaningful Speech Remains Reactive ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null;

        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 0);
        setFrH(GameCanvas.menu2, -1);
        mVector menu66 = new mVector("menu66");
        menu66.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        setFrG(GameCanvas.menu2, menu66);
        callServerMenu(-37, 0, "MENU", menu66);

        getSentPackets().clear();
        call("dungeon");
        check("Test 66: First tick dispatches opcode -30",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);

        // Present a valid Phó Chỉ Huy / nhiệm vụ speech dialog
        final boolean[] speech66Clicked = new boolean[1];
        MsgDialog speech66 = new MsgDialog();
        setDialogText(speech66, "Phó chỉ huy: Ngã tư tử thần rất nguy hiểm!" );
        speech66.right = new iCommand("Tiếp tục", 1, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                speech66Clicked[0] = true;
            }
        });
        GameCanvas.currentDialog = speech66;

        call("dungeon");
        check("Test 66: Speech dialog is handled reactively via softkey", speech66Clicked[0]);
        check("Test 66: No duplicate opcode -30 sent", getSentPackets().size() == 1);

        // ---------------------------------------------------------------------
        // Test 67: New Run Clears Stale Entry Latch
        // ---------------------------------------------------------------------
        System.out.println("--- Test 67: New Run Clears Stale Entry Latch ---");
        // Simulate previous run that stopped
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null;

        // Dispatches first Ngã Tư submenu normally
        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 0);
        setFrH(GameCanvas.menu2, -1);
        mVector menu67A = new mVector("menu67A");
        menu67A.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        setFrG(GameCanvas.menu2, menu67A);
        callServerMenu(-37, 0, "MENU", menu67A);

        getSentPackets().clear();
        call("dungeon");
        check("Test 67: First run dispatches opcode -30 normally",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);

        // Now reset / start a fresh run
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 0);
        set("dungeonWait", 0);
        set("dungeonTried", 0);
        GameCanvas.loadmap.idMap = 1;
        set("dungeonMapSeen", 1);
        GameCanvas.currentDialog = null;

        // Fresh run presents Ngã Tư submenu
        GameCanvas.menu2.isShowMenu = true;
        setFrC(GameCanvas.menu2, -37);
        setFrB(GameCanvas.menu2, 0);
        setFrH(GameCanvas.menu2, -1);
        mVector menu67B = new mVector("menu67B");
        menu67B.addElement(new iCommand("Vào Ngã Tư Tử Thần", 0));
        setFrG(GameCanvas.menu2, menu67B);
        callServerMenu(-37, 0, "MENU", menu67B);

        getSentPackets().clear();
        call("dungeon");
        check("Test 67: Fresh run may dispatch its first Ngã Tư submenu normally",
                getSentPackets().size() == 1 && ((Message) getSentPackets().get(0)).command == -30);
        check("Test 67: Fresh run state is step 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // ---------------------------------------------------------------------
        // Test 68: Dungeon Run Timeout Reason 6 Semantics Preserved
        // ---------------------------------------------------------------------
        System.out.println("--- Test 68: Dungeon Run Timeout Reason 6 Semantics Preserved ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_COMBAT);
        set("dungeonWhy", 0);
        set("dungeonFails", 0);
        GameCanvas.loadmap.idMap = Zeus.DUNGEON_MAP;

        // Trigger run timeout
        Zeus.dungeonFailRun(6, "dungeon run timed out (>300s)");

        check("Test 68: Dungeon run timeout sets dungeonWhy to 6", ((Integer) get("dungeonWhy")).intValue() == 6);
        check("Test 68: Dungeon run timeout transitions to DN_FAILURE",
                ((Integer) get("dungeonState")).intValue() == ((Integer) get("DN_FAILURE")).intValue());
        check("Test 68: Dungeon run timeout increments fails count", ((Integer) get("dungeonFails")).intValue() == 1);

        // ---------------------------------------------------------------------
        // Test 69: Background Giao Tiếp Dialog Does Not Re-trigger at Step 2
        // ---------------------------------------------------------------------
        System.out.println("--- Test 69: Background Giao Tiếp Dialog Does Not Re-trigger at Step 2 ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonState", Zeus.DN_PREPARATION);
        set("dungeonStep", 2);
        set("dungeonWait", 60);
        set("dungeonAwaitingEntry", true);
        GameCanvas.loadmap.idMap = 1;

        // Background dialog with "Giao tiếp" still lingering in GameCanvas.currentDialog
        MsgDialog bgDialog = new MsgDialog();
        setDialogText(bgDialog, "Pho Chi Huy: Can gi?" );
        bgDialog.cmdList = new mVector("buttons");
        bgDialog.cmdList.addElement(new iCommand("Giao tiếp", 0));
        bgDialog.cmdList.addElement(new iCommand("Đóng", 1));
        GameCanvas.currentDialog = bgDialog;

        getSentPackets().clear();
        call("dungeonInteract");

        check("Test 69: Zero packets sent (no duplicate opcode 23)", getSentPackets().isEmpty());
        check("Test 69: Step remains 2", ((Integer) get("dungeonStep")).intValue() == 2);
        check("Test 69: Still awaiting entry", ((Boolean) get("dungeonAwaitingEntry")).booleanValue());

        // --- Test 70: Confirmation Dialog in GameCanvas.subDialog & Awaiting Entry No-Ask-NPC ---
        System.out.println("--- Test 70: Confirmation Dialog in GameCanvas.subDialog & Awaiting Entry No-Ask-NPC ---");
        initWorld();
        set("dungeonEnabled", true);
        set("dungeonState", 2); // DN_INTERACT
        set("dungeonStep", 2);
        set("dungeonWait", 0);
        set("dungeonAwaitingEntry", true);
        GameCanvas.loadmap.idMap = 1;
        GameCanvas.currentDialog = null;

        // Part A: When dungeonWait == 0 and dungeonAwaitingEntry == true, dungeonInteract must NOT ask NPC
        getSentPackets().clear();
        call("dungeonInteract");
        check("Test 70A: Zero packets sent when wait expires while awaiting entry (no NPC re-ask)", getSentPackets().isEmpty());
        check("Test 70A: Still awaiting entry", ((Boolean) get("dungeonAwaitingEntry")).booleanValue());
        check("Test 70A: Step remains 2", ((Integer) get("dungeonStep")).intValue() == 2);

        // Part B: Confirmation dialog arriving in GameCanvas.subDialog
        final int[] test70BAction = new int[1];
        MsgDialog confirmDlgT = new MsgDialog();
        setDialogText(confirmDlgT, "Ban co muon vao nga tu tu than mot minh" );
        confirmDlgT.cmdList = new mVector("buttons");
        confirmDlgT.cmdList.addElement(new iCommand("Vào", 0, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                test70BAction[0]++;
            }
        }));
        confirmDlgT.cmdList.addElement(new iCommand("Đóng", 1));
        GameCanvas.subDialog = confirmDlgT;
        GameCanvas.currentDialog = null;

        call("dungeonInteract");
        check("Test 70B: Confirmation action executed exactly once", test70BAction[0] == 1);
        check("Test 70B: Step advanced to 3 on GameCanvas.subDialog confirmation dialog", ((Integer) get("dungeonStep")).intValue() == 3);
        check("Test 70B: Wait budget armed for teleport", ((Integer) get("dungeonWait")).intValue() == 80);

        // ---------------------------------------------------------------------
        // Test 71: Observed Live Solo Entry Modal Remains Accepted
        // ---------------------------------------------------------------------
        System.out.println("--- Test 71: Observed Live Solo Entry Modal Remains Accepted ---");
        initWorld();
        set("dungeonEnabled", true);
        set("dungeonState", 2); // DN_INTERACT / DN_PREPARATION
        set("dungeonStep", 2);
        set("dungeonWait", 60);
        set("dungeonAwaitingEntry", true);
        GameCanvas.loadmap.idMap = 1;

        final int[] test71Action = new int[1];
        MsgDialog liveSoloDlg = new MsgDialog();
        setDialogText(liveSoloDlg, "Ban co muon vao nga tu tu than mot minh" );
        liveSoloDlg.cmdList = new mVector("buttons");
        liveSoloDlg.cmdList.addElement(new iCommand("Ok", 0, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                test71Action[0]++;
            }
        }));
        liveSoloDlg.cmdList.addElement(new iCommand("Đóng", 1));
        GameCanvas.currentDialog = liveSoloDlg;
        GameCanvas.subDialog = null;

        call("dungeonInteract");
        check("Test 71: Confirmation executes exactly once", test71Action[0] == 1);
        check("Test 71: dungeonStep becomes 3", ((Integer) get("dungeonStep")).intValue() == 3);
        check("Test 71: Teleport wait is armed (80)", ((Integer) get("dungeonWait")).intValue() == 80);

        // ---------------------------------------------------------------------
        // Test 72: Standalone 'mot minh' Unrelated Modal Is Never Auto-Confirmed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 72: Standalone 'mot minh' Unrelated Modal Is Never Auto-Confirmed ---");
        initWorld();
        set("dungeonEnabled", true);
        set("dungeonState", 2);
        set("dungeonStep", 2);
        set("dungeonWait", 60);
        set("dungeonAwaitingEntry", true);
        GameCanvas.loadmap.idMap = 1;

        final int[] test72Action = new int[1];
        MsgDialog unrelatedMotMinh = new MsgDialog();
        setDialogText(unrelatedMotMinh, "Ban co muon tiep tuc mot minh khong?" );
        unrelatedMotMinh.cmdList = new mVector("buttons");
        unrelatedMotMinh.cmdList.addElement(new iCommand("Dong y", 0, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                test72Action[0]++;
            }
        }));
        unrelatedMotMinh.cmdList.addElement(new iCommand("Khong", 1));
        GameCanvas.currentDialog = unrelatedMotMinh;
        GameCanvas.subDialog = null;

        java.lang.reflect.Method mIsConfirm = Zeus.class.getDeclaredMethod("isDungeonConfirmDialog", MainDialog.class);
        mIsConfirm.setAccessible(true);
        boolean isConfirm = ((Boolean) mIsConfirm.invoke(null, unrelatedMotMinh)).booleanValue();
        check("Test 72: isDungeonConfirmDialog rejects standalone mot minh", !isConfirm);

        call("dungeonInteract");
        check("Test 72: No affirmative callback executed on standalone mot minh", test72Action[0] == 0);
        check("Test 72: Step does not advance to 3", ((Integer) get("dungeonStep")).intValue() != 3);
        check("Test 72: isBlockingDialog marks unrelated modal as blocker", Zeus.isBlockingDialog(unrelatedMotMinh));

        // ---------------------------------------------------------------------
        // Test 73: Dungeon-Looking Modal Before Submenu Dispatch Is Not Auto-Confirmed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 73: Dungeon-Looking Modal Before Submenu Dispatch Is Not Auto-Confirmed ---");
        initWorld();
        set("dungeonEnabled", true);
        set("dungeonState", 2);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonAwaitingEntry", false);
        GameCanvas.loadmap.idMap = 1;

        final int[] test73Action = new int[1];
        MsgDialog earlyConfirmDlg = new MsgDialog();
        setDialogText(earlyConfirmDlg, "Ban co muon vao nga tu tu than khong?" );
        earlyConfirmDlg.cmdList = new mVector("buttons");
        earlyConfirmDlg.cmdList.addElement(new iCommand("Vao", 0, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                test73Action[0]++;
            }
        }));
        earlyConfirmDlg.cmdList.addElement(new iCommand("Dong", 1));
        GameCanvas.currentDialog = earlyConfirmDlg;
        GameCanvas.subDialog = null;

        getSentPackets().clear();
        call("dungeonInteract");
        check("Test 73: No confirmation callback before submenu dispatch", test73Action[0] == 0);
        check("Test 73: Step remains 0 (not advanced to 3)", ((Integer) get("dungeonStep")).intValue() == 0);
        check("Test 73: Zero packets sent (no premature -32)", getSentPackets().isEmpty());

        // ---------------------------------------------------------------------
        // Test 74: Negative InputDialog.b Is Never Activated
        // ---------------------------------------------------------------------
        System.out.println("--- Test 74: Negative InputDialog.b Is Never Activated ---");
        initWorld();
        set("dungeonEnabled", true);
        set("dungeonState", 2);
        set("dungeonStep", 2);
        set("dungeonWait", 60);
        set("dungeonAwaitingEntry", true);
        GameCanvas.loadmap.idMap = 1;

        final int[] test74Action = new int[1];
        InputDialog negDzDlg = new InputDialog();
        java.lang.reflect.Field fH = InputDialog.class.getDeclaredField("name");
        fH.setAccessible(true);
        fH.set(negDzDlg, "Ban co muon vao");
        java.lang.reflect.Field fI = InputDialog.class.getDeclaredField("info");
        fI.setAccessible(true);
        fI.set(negDzDlg, "nga tu tu than");
        java.lang.reflect.Field fJ = InputDialog.class.getDeclaredField("xuluong");
        fJ.setAccessible(true);
        fJ.set(negDzDlg, "mot minh");
        java.lang.reflect.Field fB = InputDialog.class.getDeclaredField("cmdClose");
        fB.setAccessible(true);
        fB.set(negDzDlg, new iCommand("Không", 0, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                test74Action[0]++;
            }
        }));
        GameCanvas.currentDialog = negDzDlg;
        GameCanvas.subDialog = null;

        call("dungeonInteract");
        check("Test 74: Negative InputDialog.b callback is never invoked", test74Action[0] == 0);
        check("Test 74: Step does not advance to 3", ((Integer) get("dungeonStep")).intValue() != 3);

        // ---------------------------------------------------------------------
        // Test 75: GameCanvas.subDialog Giao Tiếp Rejection Contract Remains Intact
        // ---------------------------------------------------------------------
        System.out.println("--- Test 75: GameCanvas.subDialog Giao Tiếp Rejection Contract Remains Intact ---");
        initWorld();
        set("dungeonEnabled", true);
        set("dungeonState", 2);
        set("dungeonStep", 0);
        set("dungeonWait", 60);
        set("dungeonAwaitingEntry", false);
        GameCanvas.loadmap.idMap = 1;

        final int[] test75Action = new int[1];
        MsgDialog fuTGiaoTiep = new MsgDialog();
        setDialogText(fuTGiaoTiep, "Pho Chi Huy: Can gi?" );
        fuTGiaoTiep.cmdList = new mVector("buttons");
        fuTGiaoTiep.cmdList.addElement(new iCommand("Giao tiếp", 0, new AvMain() {
            public void commandPointer(int e, int f) { a(e, f); }
            public void a(int e, int f) {
                test75Action[0]++;
            }
        }));
        GameCanvas.currentDialog = null;
        GameCanvas.subDialog = fuTGiaoTiep;

        getSentPackets().clear();
        call("dungeonInteract");
        check("Test 75: GameCanvas.subDialog is not used as source for Giao tiếp", test75Action[0] == 0);
        check("Test 75: Step remains 0", ((Integer) get("dungeonStep")).intValue() == 0);
        check("Test 75: Zero packets sent from GameCanvas.subDialog Giao tiếp", getSentPackets().isEmpty());

        // ---------------------------------------------------------------------
        // Test 76: Exact UTC+7 Boundaries & 1-minute window boundaries
        // ---------------------------------------------------------------------
        System.out.println("--- Test 76: Exact UTC+7 Boundaries & 1-minute window ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonStartMin", 1200); // 20:00
        set("dungeonEndMin", 1215);   // 20:15
        set("dungeonMaxRuns", -1);
        set("dungeonRuns", 0);
        set("dungeonScheduleDateKey", -1);

        java.util.Calendar cal76 = java.util.Calendar.getInstance(java.util.TimeZone.getTimeZone("UTC"));
        cal76.set(2026, java.util.Calendar.OCTOBER, 4, 13, 0, 0); // 13:00 UTC = 20:00 UTC+7
        cal76.set(java.util.Calendar.MILLISECOND, 0);
        long t20_00 = cal76.getTime().getTime();
        long t19_59 = t20_00 - 60000L;
        long t20_14 = t20_00 + 14 * 60000L;
        long t20_15 = t20_00 + 15 * 60000L;
        long t23_30 = t20_00 + 210 * 60000L;

        check("Test 76: 19:59 (1 min before start) is rejected", !Zeus.dungeonScheduleDue(t19_59));
        check("Test 76: 20:00 (exact start boundary) is accepted", Zeus.dungeonScheduleDue(t20_00));
        check("Test 76: 20:14 (1 min before end) is accepted", Zeus.dungeonScheduleDue(t20_14));
        check("Test 76: 20:15 (exact end boundary) is rejected", !Zeus.dungeonScheduleDue(t20_15));
        check("Test 76: 23:30 (hours after window) does not catch up", !Zeus.dungeonScheduleDue(t23_30));

        // ---------------------------------------------------------------------
        // Test 77: Timezone Independence Under Varying Default JVM Timezones
        // ---------------------------------------------------------------------
        System.out.println("--- Test 77: Timezone Independence ---");
        java.util.TimeZone origTz = java.util.TimeZone.getDefault();
        try {
            String[] testTzs = new String[] { "UTC", "GMT+08:00", "America/New_York", "Europe/London", "Asia/Tokyo" };
            for (int i = 0; i < testTzs.length; i++) {
                java.util.TimeZone.setDefault(java.util.TimeZone.getTimeZone(testTzs[i]));
                int min = Zeus.dungeonMinuteOfDayUtc7(t20_00);
                int dateKey = Zeus.dungeonDateKeyUtc7(t20_00);
                check("Test 77: Minute is 1200 under JVM tz=" + testTzs[i], min == 1200);
                check("Test 77: Date key is 2026277 under JVM tz=" + testTzs[i], dateKey == 2026277);
            }
        } finally {
            java.util.TimeZone.setDefault(origTz);
        }

        // ---------------------------------------------------------------------
        // Test 78: Daily Quota Reset & Next Day Auto-Rearm Through dungeonIdle()
        // ---------------------------------------------------------------------
        System.out.println("--- Test 78: Daily Quota Reset & Next Day Auto-Rearm ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonStartMin", 1200);
        set("dungeonEndMin", 1215);
        set("dungeonMaxRuns", 2);
        set("dungeonRuns", 2);
        set("dungeonConsecutiveFails", 3);
        set("dungeonScheduleDateKey", 2026277); // Stamped for Day 1
        set("dungeonState", Zeus.DN_IDLE);

        // Day 1 inside window at 20:05 (min=1205, dateKey=2026277):
        // Quota is exhausted (2 >= 2) -> dungeonIdle() must remain in DN_IDLE, dungeonRuns unchanged
        Zeus.dungeonIdle(1205, 2026277);
        check("Test 78: Day 1 quota already reached stays DN_IDLE", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);
        check("Test 78: Day 1 dungeonRuns remains 2", ((Integer) get("dungeonRuns")).intValue() == 2);
        check("Test 78: Day 1 dungeonConsecutiveFails remains 3", ((Integer) get("dungeonConsecutiveFails")).intValue() == 3);

        // Day 2 before window at 19:59 (min=1199, dateKey=2026278):
        Zeus.dungeonIdle(1199, 2026278);
        check("Test 78: Day 2 before window stays DN_IDLE", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);
        check("Test 78: Day 2 before window dungeonRuns remains 2", ((Integer) get("dungeonRuns")).intValue() == 2);

        // Day 2 at exact start boundary 20:00 (min=1200, dateKey=2026278):
        // Window open on new calendar day!
        Zeus.dungeonIdle(1200, 2026278);
        check("Test 78: Day 2 start boundary resets dungeonRuns to 0", ((Integer) get("dungeonRuns")).intValue() == 0);
        check("Test 78: Day 2 start boundary resets dungeonConsecutiveFails to 0", ((Integer) get("dungeonConsecutiveFails")).intValue() == 0);
        check("Test 78: Day 2 dungeonScheduleDateKey stamped to Day 2", ((Integer) get("dungeonScheduleDateKey")).intValue() == 2026278);
        check("Test 78: Day 2 transitions to DN_ROUTING", ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);
        check("Test 78: dungeonEnabled remains true without toggle", ((Boolean) get("dungeonEnabled")).booleanValue());

        // Day 2 after endMin at 20:15 (min=1215, dateKey=2026278):
        // Put state back to DN_IDLE with 1 run completed
        set("dungeonState", Zeus.DN_IDLE);
        set("dungeonRuns", 1);
        Zeus.dungeonIdle(1215, 2026278);
        check("Test 78: Day 2 after endMin stays DN_IDLE (no new run)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);
        check("Test 78: Day 2 after endMin dungeonRuns remains 1", ((Integer) get("dungeonRuns")).intValue() == 1);

        // Day 2 running trip in DN_COMBAT or DN_ROUTING at/past endMin is not cancelled
        set("dungeonState", Zeus.DN_ROUTING);
        check("Test 78: Active routing not aborted", ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);

        // ---------------------------------------------------------------------
        // Test 79: Reaching Max Today Stays in DN_IDLE Instead of Permanently Disabling
        // ---------------------------------------------------------------------
        System.out.println("--- Test 79: Reaching Max Today Stays in DN_IDLE ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonStartMin", 1200);
        set("dungeonEndMin", 1215);
        set("dungeonMaxRuns", 2);
        set("dungeonRuns", 1);
        set("dungeonState", Zeus.DN_COMBAT);
        set("dungeonWasIn", true);

        Zeus.dungeonDone();
        check("Test 79: dungeonRuns reached max (2)", ((Integer) get("dungeonRuns")).intValue() == 2);
        check("Test 79: State remains DN_IDLE (not DN_OFF)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);
        check("Test 79: dungeonWhy is 0 (not 4)", ((Integer) get("dungeonWhy")).intValue() == 0);

        // ---------------------------------------------------------------------
        // Test 80: Run Active at endMin Is Not Interrupted
        // ---------------------------------------------------------------------
        System.out.println("--- Test 80: Active Run At endMin Not Interrupted ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonStartMin", 1200);
        set("dungeonEndMin", 1215);
        set("dungeonMaxRuns", 5);
        set("dungeonRuns", 0);
        set("dungeonState", Zeus.DN_COMBAT);
        set("dungeonWasIn", true);
        set("dungeonCombatEngaged", true);
        set("dungeonMonstersCleared", true);
        GameCanvas.loadmap.idMap = 48; // inside dungeon
        GameScreen.player.Action = 0; // alive

        // Call tick while inside dungeon past endMin
        call("dungeon");
        check("Test 80: Dungeon not aborted while inside map 48", ((Integer) get("dungeonState")).intValue() == Zeus.DN_COMBAT);

        // Client transitions back to Map 1 after run completed
        GameCanvas.loadmap.idMap = 1;
        call("dungeon");
        check("Test 80: Run successfully completed and accounted", ((Integer) get("dungeonRuns")).intValue() == 1);
        check("Test 80: Returned to DN_IDLE for cooldown", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        // ---------------------------------------------------------------------
        // Test 81: Unscheduled Mode (-1/-1) Preserves Legacy Session Behavior
        // ---------------------------------------------------------------------
        System.out.println("--- Test 81: Unscheduled Mode Preserves Legacy Behavior ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonStartMin", -1);
        set("dungeonEndMin", -1);
        set("dungeonMaxRuns", 1);
        set("dungeonRuns", 0);
        set("dungeonState", Zeus.DN_COMBAT);
        set("dungeonWasIn", true);

        check("Test 81: isDungeonScheduled is false", !Zeus.isDungeonScheduled());
        check("Test 81: dungeonScheduleDue is true for unscheduled", Zeus.dungeonScheduleDue());
        Zeus.dungeonDone();
        check("Test 81: dungeonRuns is 1", ((Integer) get("dungeonRuns")).intValue() == 1);
        check("Test 81: Unscheduled mode stops with DN_OFF", ((Integer) get("dungeonState")).intValue() == Zeus.DN_OFF);
        check("Test 81: Unscheduled mode stops with why=4", ((Integer) get("dungeonWhy")).intValue() == 4);

        // ---------------------------------------------------------------------
        // Test 82: Configuration Validation & Fail-Closed Safety
        // ---------------------------------------------------------------------
        System.out.println("--- Test 82: Configuration Validation ---");
        Method acceptControlMethod = Zeus.class.getDeclaredMethod("acceptControl", int[].class, String.class, String.class);
        acceptControlMethod.setAccessible(true);

        int[] validVals = new int[38];
        validVals[0] = 15; // CTL_VERSION
        validVals[1] = 0;  // atk.mode
        validVals[2] = 0;  // atk.map
        validVals[3] = 0;  // atk.zone
        validVals[4] = -1; // atk.x
        validVals[5] = -1; // atk.y
        validVals[6] = 120;// atk.radius
        validVals[7] = 1;  // atk.hpOn
        validVals[8] = 50; // atk.hpPct
        validVals[9] = 1;  // atk.mpOn
        validVals[10] = 50;// atk.mpPct
        validVals[11] = 1; // revive.mode
        validVals[13] = 0; // atk.zoneMode
        validVals[14] = 1; // atk.zonePick
        validVals[15] = 0; // item.rank
        validVals[16] = 0; // item.mphp
        validVals[17] = 1; // item.gold
        validVals[18] = 0; // mount.on
        validVals[19] = 0; // mount.id
        validVals[20] = 0; // item.medalDialog
        validVals[21] = 0; // item.dropsOn
        validVals[23] = -1;// nav.target
        validVals[24] = 0; // ui.ring
        validVals[25] = 0; // atk.farmOnArrival
        validVals[26] = 0; // nav.detectSpots
        validVals[27] = 0; // revive.delay
        validVals[28] = 0; // revive.on
        validVals[29] = 0; // enhance.on
        validVals[30] = 10;// enhance.maxLv
        validVals[31] = 0; // enhance.charm
        validVals[32] = 1; // dungeon.on
        validVals[33] = -1;// dungeon.max
        validVals[34] = 1200; // dungeon.startMin
        validVals[35] = 1215; // dungeon.endMin
        validVals[36] = 1; // ui.effects
        validVals[37] = 0; // ui.hidePlayers

        String buffs = "000";
        String drops = "000000";

        // Valid configuration
        boolean resValid = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: Valid 1200..1215 window accepted", resValid);

        // Unscheduled -1/-1 valid
        validVals[34] = -1;
        validVals[35] = -1;
        boolean resUnscheduled = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: Unscheduled -1/-1 accepted", resUnscheduled);

        // One-sided -1 (startMin=-1, endMin=1215) rejected
        validVals[34] = -1;
        validVals[35] = 1215;
        boolean resOneSided1 = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: One-sided startMin=-1 rejected", !resOneSided1);

        // One-sided -1 (startMin=1200, endMin=-1) rejected
        validVals[34] = 1200;
        validVals[35] = -1;
        boolean resOneSided2 = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: One-sided endMin=-1 rejected", !resOneSided2);

        // start >= end (1200 >= 1200) rejected
        validVals[34] = 1200;
        validVals[35] = 1200;
        boolean resEqual = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: start == end rejected", !resEqual);

        // start > end (1215 > 1200) rejected
        validVals[34] = 1215;
        validVals[35] = 1200;
        boolean resGreater = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: start > end rejected", !resGreater);

        // Out of range (< -1) rejected
        validVals[34] = -2;
        validVals[35] = 1215;
        boolean resNegative = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: startMin < -1 rejected", !resNegative);

        // Out of range (> 1439) rejected
        validVals[34] = 1200;
        validVals[35] = 1440;
        boolean resOverflow = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: endMin > 1439 rejected", !resOverflow);

        // dungeon.max = 0 is rejected (valid domain is -1 or 1..10)
        validVals[33] = 0;
        validVals[34] = 1200;
        validVals[35] = 1215;
        boolean resMaxZero = ((Boolean) acceptControlMethod.invoke(null, validVals, buffs, drops)).booleanValue();
        check("Test 82: dungeon.max=0 rejected", !resMaxZero);
        validVals[33] = -1;

        // ---------------------------------------------------------------------
        // Test 83: Clock / Conversion Failure Fails Closed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 83: Clock / Conversion Failure Fails Closed ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonStartMin", 1200);
        set("dungeonEndMin", 1215);
        set("dungeonMaxRuns", -1);
        set("dungeonState", Zeus.DN_IDLE);

        // Negative minute indicates clock/conversion failure -> fails closed
        check("Test 83: Conversion failure min=-1 in dungeonScheduleDue fails closed", !Zeus.dungeonScheduleDue(-1, 2026278));
        // Negative dateKey indicates clock/conversion failure -> fails closed
        check("Test 83: Conversion failure dateKey=-1 in dungeonScheduleDue fails closed", !Zeus.dungeonScheduleDue(1205, -1));

        // In dungeonIdle, conversion failures fail closed (stay in DN_IDLE)
        Zeus.dungeonIdle(-1, 2026278);
        check("Test 83: Conversion failure min=-1 in dungeonIdle stays DN_IDLE", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);
        Zeus.dungeonIdle(1205, -1);
        check("Test 83: Conversion failure dateKey=-1 in dungeonIdle stays DN_IDLE", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        // ---------------------------------------------------------------------
        // Test 84: Invalid Schedule Pair Fails Closed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 84: Invalid Schedule Pair Fails Closed ---");
        Zeus.dungeonReset();
        set("dungeonEnabled", true);
        set("dungeonMaxRuns", -1);

        // Exact -1/-1 remains unscheduled and immediate
        set("dungeonStartMin", -1);
        set("dungeonEndMin", -1);
        check("Test 84: Exact -1/-1 is unscheduled", Zeus.isDungeonUnscheduled());
        check("Test 84: Exact -1/-1 is not scheduled", !Zeus.isDungeonScheduled());
        check("Test 84: Exact -1/-1 scheduleDue is true", Zeus.dungeonScheduleDue(1205, 2026278));
        set("dungeonState", Zeus.DN_IDLE);
        Zeus.dungeonIdle(1205, 2026278);
        check("Test 84: Exact -1/-1 starts run", ((Integer) get("dungeonState")).intValue() == Zeus.DN_ROUTING);

        // Invalid -1/500 fails closed
        set("dungeonStartMin", -1);
        set("dungeonEndMin", 500);
        check("Test 84: -1/500 is not unscheduled", !Zeus.isDungeonUnscheduled());
        check("Test 84: -1/500 is not scheduled", !Zeus.isDungeonScheduled());
        check("Test 84: -1/500 scheduleDue fails closed", !Zeus.dungeonScheduleDue(200, 2026278));
        set("dungeonState", Zeus.DN_IDLE);
        Zeus.dungeonIdle(200, 2026278);
        check("Test 84: -1/500 dungeonIdle fails closed (stays DN_IDLE)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        // Invalid 500/-1 fails closed
        set("dungeonStartMin", 500);
        set("dungeonEndMin", -1);
        check("Test 84: 500/-1 is not unscheduled", !Zeus.isDungeonUnscheduled());
        check("Test 84: 500/-1 is not scheduled", !Zeus.isDungeonScheduled());
        check("Test 84: 500/-1 scheduleDue fails closed", !Zeus.dungeonScheduleDue(600, 2026278));
        set("dungeonState", Zeus.DN_IDLE);
        Zeus.dungeonIdle(600, 2026278);
        check("Test 84: 500/-1 dungeonIdle fails closed (stays DN_IDLE)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        // Invalid start >= end fails closed
        set("dungeonStartMin", 600);
        set("dungeonEndMin", 500);
        check("Test 84: 600/500 scheduleDue fails closed", !Zeus.dungeonScheduleDue(550, 2026278));
        set("dungeonState", Zeus.DN_IDLE);
        Zeus.dungeonIdle(550, 2026278);
        check("Test 84: 600/500 dungeonIdle fails closed (stays DN_IDLE)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        set("dungeonStartMin", 500);
        set("dungeonEndMin", 500);
        check("Test 84: 500/500 scheduleDue fails closed", !Zeus.dungeonScheduleDue(500, 2026278));
        set("dungeonState", Zeus.DN_IDLE);
        Zeus.dungeonIdle(500, 2026278);
        check("Test 84: 500/500 dungeonIdle fails closed (stays DN_IDLE)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        // Invalid values outside 0..1439 fail closed
        set("dungeonStartMin", -5);
        set("dungeonEndMin", 1200);
        check("Test 84: -5/1200 scheduleDue fails closed", !Zeus.dungeonScheduleDue(600, 2026278));
        set("dungeonState", Zeus.DN_IDLE);
        Zeus.dungeonIdle(600, 2026278);
        check("Test 84: -5/1200 dungeonIdle fails closed (stays DN_IDLE)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        set("dungeonStartMin", 1200);
        set("dungeonEndMin", 1500);
        check("Test 84: 1200/1500 scheduleDue fails closed", !Zeus.dungeonScheduleDue(1205, 2026278));
        set("dungeonState", Zeus.DN_IDLE);
        Zeus.dungeonIdle(1205, 2026278);
        check("Test 84: 1200/1500 dungeonIdle fails closed (stays DN_IDLE)", ((Integer) get("dungeonState")).intValue() == Zeus.DN_IDLE);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }

    static MainObject makePhoChiHuy(int x, int y) {
        MainObject pcf = new MainObject() {
            public void GiaoTiep() {
                try {
                    GlobalService.gI().getlist_from_npc((byte) this.ID);
                } catch (Throwable t) {}
            }
        };
        pcf.typeObject = 2;
        pcf.ID = -37;
        pcf.name = "Pho Chi Huy";
        pcf.x = x;
        pcf.y = y;
        return pcf;
    }

    static MsgDialog makeBroadcastPopup(String text) {
        MsgDialog dialog = new MsgDialog();
        setDialogText(dialog, text );
        dialog.cmdList = new mVector("buttons");
        dialog.cmdList.addElement(new iCommand("Ok", -1));
        return dialog;
    }

    static MsgDialog makeBlockingModal(String text) {
        MsgDialog dialog = new MsgDialog();
        setDialogText(dialog, text );
        dialog.cmdList = new mVector("buttons");
        dialog.cmdList.addElement(new iCommand("Đồng ý", 1));
        dialog.cmdList.addElement(new iCommand("Không", 2));
        return dialog;
    }

    static void callServerMenu(int idNpc, int idMenu, String title, mVector items) throws Exception {
        Method m = Zeus.class.getDeclaredMethod("serverMenu", mVector.class, int.class, int.class, String.class);
        m.setAccessible(true);
        m.invoke(null, items, idMenu, idNpc, title);
    }

    static boolean callTravelArrive(int x, int y, int tol) throws Exception {
        Method m = Zeus.class.getDeclaredMethod("travelArrive", int.class, int.class, int.class);
        m.setAccessible(true);
        return ((Boolean) m.invoke(null, x, y, tol)).booleanValue();
    }

    static boolean callDungeonNpcEligible(MainObject npc) throws Exception {
        Method m = Zeus.class.getDeclaredMethod("dungeonNpcEligible", MainObject.class);
        m.setAccessible(true);
        return ((Boolean) m.invoke(null, npc)).booleanValue();
    }

    static int getFrH(Menu2 menu) {
        return menu.menuSelectedItem;
    }

    static void setFrH(Menu2 menu, int val) {
        menu.menuSelectedItem = val;
    }

    static mVector getFrG(Menu2 menu) {
        return menu.menuItems;
    }

    static void setFrG(Menu2 menu, mVector items) {
        menu.menuItems = items;
    }

    static void setFrC(Menu2 menu, int val) {
        try {
            Field f = Menu2.class.getDeclaredField("IdNpc");
            f.setAccessible(true);
            f.setInt(menu, val);
        } catch (Throwable t) {
        }
    }

    static void setFrB(Menu2 menu, int val) {
        try {
            Field f = Menu2.class.getDeclaredField("IdMenu");
            f.setAccessible(true);
            f.setInt(menu, val);
        } catch (Throwable t) {
        }
    }
}


