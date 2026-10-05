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

public class ReconnectTravelTest {

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

    static int intCall(String name) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod(name);
        m.setAccessible(true);
        return ((Integer) m.invoke(null)).intValue();
    }

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static void setupWorldState(int mapId) {
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
            GameCanvas.loadmap.idMap = mapId;
        }
        LoadMap.isShowEffAuto = 10;
        LoadMap.EFF_PHOBANG_END = 20;
        if (GameScreen.player == null) {
            GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
        }
        GameScreen.player.Action = (byte) 0; // alive
        GameScreen.player.typePk = -1; // realistic peaceful v4.0.3 player
        GameScreen.player.typeBoss = 0; // standard non-boss player
        GameScreen.player.x = 100;
        GameScreen.player.y = 100;
        GameScreen.ObjFocus = null;
        GameCanvas.currentDialog = null;
        GameCanvas.subDialog = null;
        if (GameCanvas.menu2 != null) {
            GameCanvas.menu2.isShowMenu = false;
        }
    }

    public static void main(String[] args) throws Exception {
        setupWorldState(1);

        // =====================================================================
        // Test 1: same_map_reconnect_reset
        // =====================================================================
        System.out.println("--- Test 1: same_map_reconnect_reset ---");
        setupWorldState(1);
        Player.isLockKey = true;
        GameScreen.player.posTransRoad = new short[] { 10, 20 };
        set("travelMapSeen", Integer.valueOf(1));
        set("travelHops", Integer.valueOf(5));

        // Transition to char-select screen
        if (GameCanvas.selectChar == null) {
            GameCanvas.selectChar = new SelectCharScreen();
        }
        GameCanvas.currentScreen = GameCanvas.selectChar;
        call("sessionTick");

        // Transition back to world screen on same map (1)
        GameCanvas.currentScreen = GameCanvas.game;
        call("sessionTick");

        check("movement lock Player.isLockKey cleared", !Player.isLockKey);
        check("path buffer GameScreen.player.posTransRoad cleared", GameScreen.player.posTransRoad == null);
        check("travelMapSeen reset to sentinel", ((Integer) get("travelMapSeen")).intValue() == Integer.MIN_VALUE);
        check("travelHops reset on new session", ((Integer) get("travelHops")).intValue() == 0);

        // Tick to settle on same map (1)
        for (int i = 0; i < 15; i++) {
            call("sessionTick");
        }
        check("mapStable() becomes true on same map", boolCall("mapStable"));

        // Travel should now reinitialize travelMapSeen to 1 without skipping
        set("atkMode", Integer.valueOf(1));
        set("atkMap", Integer.valueOf(1)); // already at destination
        set("atkX", Integer.valueOf(100));
        set("atkY", Integer.valueOf(100));
        call("travel");
        check("travelMapSeen updated to current map", ((Integer) get("travelMapSeen")).intValue() == 1);

        // =====================================================================
        // Test 2: cross_map_reconnect_reset
        // =====================================================================
        System.out.println("--- Test 2: cross_map_reconnect_reset ---");
        setupWorldState(5);
        set("travelHops", Integer.valueOf(18));
        set("travelStoneTried", Integer.valueOf(2));
        set("travelStallTicks", Integer.valueOf(40));
        set("atkMode", Integer.valueOf(1));
        set("atkMap", Integer.valueOf(20));
        set("atkX", Integer.valueOf(500));
        set("atkY", Integer.valueOf(600));
        set("navTarget", Integer.valueOf(8));
        set("navDone", Boolean.TRUE);

        // Reconnect into map 10
        GameCanvas.currentScreen = GameCanvas.selectChar;
        call("sessionTick");
        setupWorldState(10);
        GameCanvas.currentScreen = GameCanvas.game;
        call("sessionTick");

        check("cross-map travelHops reset to 0", ((Integer) get("travelHops")).intValue() == 0);
        check("cross-map travelStoneTried reset to 0", ((Integer) get("travelStoneTried")).intValue() == 0);
        check("cross-map travelStallTicks reset to 0", ((Integer) get("travelStallTicks")).intValue() == 0);
        check("durable atkMode preserved", ((Integer) get("atkMode")).intValue() == 1);
        check("durable atkMap preserved", ((Integer) get("atkMap")).intValue() == 20);
        check("durable atkX preserved", ((Integer) get("atkX")).intValue() == 500);
        check("durable atkY preserved", ((Integer) get("atkY")).intValue() == 600);
        check("durable navDone preserved", ((Boolean) get("navDone")).booleanValue());
        check("goal() returns Auto Farm destination (20)", intCall("goal") == 20);

        // =====================================================================
        // Test 3: map_stability
        // =====================================================================
        System.out.println("--- Test 3: map_stability ---");
        setupWorldState(7);
        call("sessionReset");

        // Tick 5 times on map 7
        for (int i = 0; i < 5; i++) {
            call("sessionTick");
        }
        check("mapStable() false after only 5 ticks", !boolCall("mapStable"));

        // Switch map to 8 on next tick
        setupWorldState(8);
        call("sessionTick");
        check("map change resets mapStable()", !boolCall("mapStable"));

        // Tick 8 more times on map 8 (total 9 ticks on map 8)
        for (int i = 0; i < 8; i++) {
            call("sessionTick");
        }
        check("mapStable() still false after 9 ticks", !boolCall("mapStable"));

        // 10th tick on map 8
        call("sessionTick");
        check("mapStable() true after 10 consecutive ticks on same map", boolCall("mapStable"));

        // =====================================================================
        // Test 4: portal_transition
        // =====================================================================
        System.out.println("--- Test 4: portal_transition ---");
        // Start stable on map 8
        check("initially stable on map 8", boolCall("mapStable"));

        // Portal transition: scene becomes not ready or map changes
        LoadMap.isShowEffAuto = LoadMap.EFF_PHOBANG_END; // scene not ready
        call("sessionTick");
        check("portal transition (scene loading) resets mapStable()", !boolCall("mapStable"));

        // New map loaded: map 9, scene ready
        setupWorldState(9);
        call("sessionTick");
        check("new map tick 1 not stable", !boolCall("mapStable"));

        for (int i = 0; i < 9; i++) {
            call("sessionTick");
        }
        check("new map settled (10 ticks) becomes mapStable()", boolCall("mapStable"));

        // =====================================================================
        // Test 5: character_select_retry_success
        // =====================================================================
        System.out.println("--- Test 5: character_select_retry_success ---");
        call("sessionReset");
        if (GameCanvas.selectChar == null) {
            GameCanvas.selectChar = new SelectCharScreen();
        }
        if (SelectCharScreen.VecSelectChar == null) {
            SelectCharScreen.VecSelectChar = new mVector("chars");
        }
        if (SelectCharScreen.VecSelectChar.size() == 0) {
            SelectCharScreen.VecSelectChar.addElement(new Other_Players(1, (byte) 0, "hero", 0, 0));
        }
        GameCanvas.currentScreen = GameCanvas.selectChar;
        MsgDialog.isAutologin = false;
        set("armed", Boolean.FALSE);

        // Initial entry to char-select
        call("auth");
        check("initial auth submit sets MsgDialog.isAutologin = true", MsgDialog.isAutologin);
        check("initial auth arms armed = true", ((Boolean) get("armed")).booleanValue());

        // Simulate client processing the submit flag but remaining on char-select
        MsgDialog.isAutologin = false;

        // Ticking fewer than retry interval should NOT re-submit
        for (int i = 0; i < 30; i++) {
            call("auth");
        }
        check("no rapid duplicate submit during interval", !MsgDialog.isAutologin);

        // Tick past interval (threshold = 75 ticks)
        for (int i = 0; i < 50; i++) {
            call("auth");
        }
        check("retry submit triggered after interval", MsgDialog.isAutologin);

        // Leaving char select resets retry state
        MsgDialog.isAutologin = false;
        GameCanvas.currentScreen = GameCanvas.game;
        call("auth");
        check("leaving char-select disarms armed", !((Boolean) get("armed")).booleanValue());

        // =====================================================================
        // Test 6: character_select_retry_exhaustion
        // =====================================================================
        System.out.println("--- Test 6: character_select_retry_exhaustion ---");
        GameCanvas.currentScreen = GameCanvas.selectChar;
        MsgDialog.isAutologin = false;

        // Attempt 1
        call("auth");
        check("attempt 1 submitted", MsgDialog.isAutologin);
        MsgDialog.isAutologin = false;

        // Wait interval -> Attempt 2
        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("attempt 2 submitted", MsgDialog.isAutologin);
        MsgDialog.isAutologin = false;

        // Wait interval -> Attempt 3
        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("attempt 3 submitted", MsgDialog.isAutologin);
        MsgDialog.isAutologin = false;

        // Wait further interval -> Exhausted! No attempt 4
        for (int i = 0; i < 150; i++) {
            call("auth");
        }
        check("after 3 attempts, retries exhausted and MsgDialog.isAutologin NOT set", !MsgDialog.isAutologin);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
