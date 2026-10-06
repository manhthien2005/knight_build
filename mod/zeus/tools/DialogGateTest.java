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

public class DialogGateTest {

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

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static void setDialogText(MainDialog d, String text) {
        try {
            Field f = MainDialog.class.getDeclaredField("strinfo");
            f.setAccessible(true);
            f.set(d, new String[] { text });
        } catch (Throwable t) {}
    }

    static class TestTarget extends AvMain {
        boolean pressed = false;
        int pressCount = 0;
        boolean dismissOnPress = true;

        public void commandPointer(int index, int subIndex) {
            pressed = true;
            pressCount++;
            if (dismissOnPress) {
                GameCanvas.currentDialog = null;
            }
        }
        public void a(int id, int h) {
            commandPointer(id, h);
        }
    }

    static MsgDialog makeDialog(String text, TestTarget target, String caption) {
        MsgDialog dialog = new MsgDialog();
        setDialogText(dialog, text );
        mVector list = new mVector("buttons");
        iCommand button = new iCommand(caption, 1, target);
        list.addElement(button);
        dialog.cmdList = list;
        return dialog;
    }

    static MsgDialog makeTwoButtonDialog(String text, TestTarget target1, String cap1, TestTarget target2, String cap2) {
        MsgDialog dialog = new MsgDialog();
        setDialogText(dialog, text );
        mVector list = new mVector("buttons");
        iCommand b1 = new iCommand(cap1, 1, target1);
        iCommand b2 = new iCommand(cap2, 2, target2);
        list.addElement(b1);
        list.addElement(b2);
        dialog.cmdList = list;
        return dialog;
    }

    static void setupWorldState() {
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
                // fallback
            }
        }
        if (GameCanvas.loadmap != null) {
            GameCanvas.loadmap.idMap = 1; // map id 1
        }
        LoadMap.isShowEffAuto = 10;
        LoadMap.EFF_PHOBANG_END = 20; // LoadMap.isShowEffAuto != LoadMap.EFF_PHOBANG_END
        if (GameScreen.player == null) {
            GameScreen.player = new Player(100, (byte) 0, "hero", 0, 0);
        }
        GameScreen.player.Action = (byte) 0; // alive
        GameScreen.player.typePk = -1; // realistic peaceful v4.0.3 player
        GameScreen.player.typeBoss = 0; // standard non-boss player
        GameScreen.player.x = 100;
        GameScreen.player.y = 100;
        GameScreen.ObjFocus = null; // no captcha
        GameCanvas.currentDialog = null;
        GameCanvas.subDialog = null;
        if (GameCanvas.menu2 != null) {
            GameCanvas.menu2.isShowMenu = false;
        }
    }

    public static void main(String[] args) throws Exception {
        setupWorldState();

        // =====================================================================
        // Test 1: Harmless informational dialog dismissed without ready() true
        // =====================================================================
        System.out.println("--- Test 1: Safe Informational Dialog Dismissal ---");
        TestTarget targetSafe = new TestTarget();
        MsgDialog safeDialog = makeDialog("Thong bao tu server: Bao tri hoan tat.", targetSafe, "Đóng");
        GameCanvas.currentDialog = safeDialog;

        // Verify ready() is false initially because GameCanvas.currentDialog != null
        check("ready() is false while dialog open", !boolCall("ready"));
        check("gameReady() is false while dialog open", !boolCall("gameReady"));

        // Tick through debounce
        for (int i = 0; i < 5; i++) {
            call("tick");
        }
        check("safe dialog button pressed", targetSafe.pressed);
        check("safe dialog dismissed (GameCanvas.currentDialog == null)", GameCanvas.currentDialog == null);

        // =====================================================================
        // Test 2: Unknown dialog fails closed (untouched, automation blocked)
        // =====================================================================
        System.out.println("--- Test 2: Unknown Dialog Fails Closed ---");
        TestTarget targetUnknown = new TestTarget();
        MsgDialog unknownDialog = makeDialog("Nap the nhan khuyen mai 500% cuc hot", targetUnknown, "Đóng");
        GameCanvas.currentDialog = unknownDialog;

        for (int i = 0; i < 10; i++) {
            call("tick");
        }
        check("unknown dialog NOT pressed", !targetUnknown.pressed);
        check("unknown dialog remains open", GameCanvas.currentDialog == unknownDialog);
        check("ready() remains false", !boolCall("ready"));
        check("gameReady() remains false", !boolCall("gameReady"));
        GameCanvas.currentDialog = null; // clear for next test

        // =====================================================================
        // Test 3: Dangerous/confirmation dialog never auto-confirmed
        // =====================================================================
        System.out.println("--- Test 3: Dangerous Dialog Safety ---");
        TestTarget targetYes = new TestTarget();
        TestTarget targetNo = new TestTarget();
        MsgDialog deleteDialog = makeTwoButtonDialog("Ban co muon xoa nhan vat nay khong?", targetYes, "Đồng ý", targetNo, "Không");
        GameCanvas.currentDialog = deleteDialog;

        for (int i = 0; i < 10; i++) {
            call("tick");
        }
        check("dangerous 2-button dialog NOT pressed", !targetYes.pressed && !targetNo.pressed);
        check("dangerous dialog remains open", GameCanvas.currentDialog == deleteDialog);
        check("gameReady() blocked by dangerous dialog", !boolCall("gameReady"));
        GameCanvas.currentDialog = null;

        // Single button with "Đồng ý" caption should also NEVER be confirmed
        TestTarget targetDongY = new TestTarget();
        MsgDialog confirmSingle = makeDialog("Xac nhan mua vat pham voi gia 1000 ngoc?", targetDongY, "Đồng ý");
        GameCanvas.currentDialog = confirmSingle;
        for (int i = 0; i < 10; i++) {
            call("tick");
        }
        check("single-button Dong Y dialog NOT confirmed", !targetDongY.pressed);
        check("single-button Dong Y dialog remains open", GameCanvas.currentDialog == confirmSingle);
        GameCanvas.currentDialog = null;

        // Pháp sư blacksmith intro dialog must NOT be dismissed by generic dialog dismisser when enhancement is idle
        TestTarget targetCuongHoa = new TestTarget();
        TestTarget targetDong = new TestTarget();
        MsgDialog phapSuDlg = makeTwoButtonDialog("Ta có thể gia tăng sức mạnh của một món đồ bằng thuật cường hóa chúng", targetCuongHoa, "Cường hóa", targetDong, "Đóng");
        try {
            Field fn = MsgDialog.class.getDeclaredField("nameShow");
            fn.setAccessible(true);
            fn.set(phapSuDlg, "Pháp sư");
        } catch (Throwable t) {}
        GameCanvas.currentDialog = phapSuDlg;
        for (int i = 0; i < 10; i++) {
            call("tick");
        }
        check("Pháp sư blacksmith intro dialog NOT dismissed by generic tick", !targetCuongHoa.pressed && !targetDong.pressed);
        check("Pháp sư dialog remains open when enhancement idle", GameCanvas.currentDialog == phapSuDlg);
        GameCanvas.currentDialog = null;

        // =====================================================================
        // Test 4: Bounded retries on stubborn dialog
        // =====================================================================
        System.out.println("--- Test 4: Bounded Retries ---");
        TestTarget targetStubborn = new TestTarget();
        targetStubborn.dismissOnPress = false; // Dialog refuses to close
        MsgDialog stubbornDialog = makeDialog("Thong bao: Su kien dua top.", targetStubborn, "OK");
        GameCanvas.currentDialog = stubbornDialog;

        for (int i = 0; i < 20; i++) {
            call("tick");
        }
        check("stubborn dialog retry count is bounded (<= 3)", targetStubborn.pressCount <= 3);
        check("stubborn dialog still blocks gameReady()", !boolCall("gameReady"));
        GameCanvas.currentDialog = null;

        // =====================================================================
        // Test 5: Internal readiness during login/char-select/world loading
        // =====================================================================
        System.out.println("--- Test 5: Internal Readiness Login/Loading ---");
        if (GameCanvas.selectChar == null) {
            GameCanvas.selectChar = new SelectCharScreen();
        }
        GameCanvas.currentScreen = GameCanvas.selectChar; // char select
        call("tick");
        check("gameReady() false on char-select", !boolCall("gameReady"));

        GameCanvas.currentScreen = null; // uninitialized
        check("gameReady() false when GameCanvas.currentScreen is null", !boolCall("gameReady"));

        // =====================================================================
        // Test 6: Internal readiness settles in world and becomes true
        // =====================================================================
        System.out.println("--- Test 6: Settling to Game Ready ---");
        setupWorldState();
        // Reset session state
        call("sessionReset");
        check("gameReady() false immediately after session reset", !boolCall("gameReady"));

        // Tick through settle period (10 ticks)
        for (int i = 0; i < 12; i++) {
            call("tick");
        }
        check("gameReady() becomes true after settling without dialog", boolCall("gameReady"));

        // =====================================================================
        // Test 7: Auto Farm and Travel regression checks
        // =====================================================================
        System.out.println("--- Test 7: Auto Farm and Travel Semantics ---");
        set("atkMode", Integer.valueOf(1)); // Stand
        set("atkMap", Integer.valueOf(5));
        set("atkX", Integer.valueOf(120));
        set("atkY", Integer.valueOf(240));
        set("navTarget", Integer.valueOf(8));
        Method goalMethod = Class.forName("Zeus").getDeclaredMethod("goal");
        goalMethod.setAccessible(true);
        check("goal() returns Auto Farm map (5) when atkMode=1",
                ((Integer) goalMethod.invoke(null)).intValue() == 5);

        set("atkMode", Integer.valueOf(0)); // Off
        check("goal() returns navTarget (8) when atkMode=0",
                ((Integer) goalMethod.invoke(null)).intValue() == 8);

        // =====================================================================
        // Test 8: Realistic peaceful player v4.0.3 gameReady() contract
        // =====================================================================
        System.out.println("--- Test 8: Realistic Peaceful Player gameReady() Contract ---");
        setupWorldState(); // player has typePk = -1, typeBoss = 0, x = 100, y = 100
        call("sessionReset");
        for (int i = 0; i < 12; i++) {
            call("tick");
        }
        check("8a: peaceful player + valid coordinates + settled world => gameReady true", boolCall("gameReady"));

        // 8b: Negative x coordinate => gameReady false
        GameScreen.player.x = -1;
        check("8b: negative x coordinate => gameReady false", !boolCall("gameReady"));
        GameScreen.player.x = 100;

        // 8c: Negative y coordinate => gameReady false
        GameScreen.player.y = -1;
        check("8c: negative y coordinate => gameReady false", !boolCall("gameReady"));
        GameScreen.player.y = 100;

        // 8d: Dialog open => gameReady false
        TestTarget dummyTarget = new TestTarget();
        GameCanvas.currentDialog = makeDialog("Some dialog", dummyTarget, "Dong");
        check("8d: dialog open => gameReady false", !boolCall("gameReady"));
        GameCanvas.currentDialog = null;

        // 8e: Dead player (Action = 4) => gameReady false
        GameScreen.player.Action = (byte) 4;
        check("8e: dead player (Action=4) => gameReady false", !boolCall("gameReady"));
        GameScreen.player.Action = (byte) 0;

        // 8f: Captcha active (ObjFocus != null && ObjFocus.typeBoss == 2) => gameReady false
        MainObject captchaMob = new MainObject(999, (byte) 1, "Con Ma", 100, 100);
        captchaMob.typeBoss = (byte) 2;
        GameScreen.ObjFocus = captchaMob;
        check("8f: captcha monster => gameReady false", !boolCall("gameReady"));
        GameScreen.ObjFocus = null;

        // 8g: Loading / not sceneReady => gameReady false
        LoadMapScreen.isNextMap = false;
        check("8g: loading / not sceneReady => gameReady false", !boolCall("gameReady"));
        LoadMapScreen.isNextMap = true;

        // 8h: Settle threshold not reached => gameReady false
        call("sessionReset");
        check("8h: settle threshold not reached (0 ticks) => gameReady false", !boolCall("gameReady"));
        for (int i = 0; i < 5; i++) {
            call("tick");
        }
        check("8h: settle threshold not reached (5 ticks < 10) => gameReady false", !boolCall("gameReady"));
        for (int i = 0; i < 7; i++) {
            call("tick");
        }
        check("8h: settle threshold reached (12 ticks >= 10) => gameReady true", boolCall("gameReady"));

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
