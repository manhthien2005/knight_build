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

public class CharacterSlotTest {

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

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static mVector makeCharList(int count) {
        mVector list = new mVector("TestChars");
        for (int i = 0; i < count; i++) {
            list.addElement(new Other_Players(i + 1, (byte) 0, "char" + i, 0, 0));
        }
        return list;
    }

    public static void main(String[] args) throws Exception {
        if (GameCanvas.selectChar == null) {
            GameCanvas.selectChar = new SelectCharScreen();
        }
        GameCanvas.currentScreen = GameCanvas.selectChar;

        // ---------------------------------------------------------------------
        // Test 1: Internal slot 0 selects first character when >= 1 exists
        // ---------------------------------------------------------------------
        System.out.println("--- Test 1: slot 0 selects first character ---");
        System.setProperty("zeus.auth.slot", "0");
        SelectCharScreen.VecSelectChar = makeCharList(1);
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("slot 0 sets MsgDialog.isAutologin=true", MsgDialog.isAutologin);
        check("slot 0 sets GameCanvas.selectChar.selectChar=0", GameCanvas.selectChar.selectChar == 0);

        // ---------------------------------------------------------------------
        // Test 2: Internal slot 1 selects second character when >= 2 exist
        // ---------------------------------------------------------------------
        System.out.println("--- Test 2: slot 1 selects second character ---");
        System.setProperty("zeus.auth.slot", "1");
        SelectCharScreen.VecSelectChar = makeCharList(2);
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("slot 1 sets MsgDialog.isAutologin=true", MsgDialog.isAutologin);
        check("slot 1 sets GameCanvas.selectChar.selectChar=1", GameCanvas.selectChar.selectChar == 1);

        // ---------------------------------------------------------------------
        // Test 3: Internal slot 2 selects third character when 3 exist
        // ---------------------------------------------------------------------
        System.out.println("--- Test 3: slot 2 selects third character ---");
        System.setProperty("zeus.auth.slot", "2");
        SelectCharScreen.VecSelectChar = makeCharList(3);
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("slot 2 sets MsgDialog.isAutologin=true", MsgDialog.isAutologin);
        check("slot 2 sets GameCanvas.selectChar.selectChar=2", GameCanvas.selectChar.selectChar == 2);

        // ---------------------------------------------------------------------
        // Test 4: Requested slot >= character count refuses world entry (no fallback to Slot 1)
        // ---------------------------------------------------------------------
        System.out.println("--- Test 4: requested slot >= count refuses world entry ---");
        System.setProperty("zeus.auth.slot", "1");
        SelectCharScreen.VecSelectChar = makeCharList(1); // count=1, slot 1 is empty/unavailable
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("unavailable slot 1 refuses MsgDialog.isAutologin", !MsgDialog.isAutologin);
        check("no fallback to slot 0 in GameCanvas.selectChar.selectChar", GameCanvas.selectChar.selectChar != 0);

        System.setProperty("zeus.auth.slot", "2");
        SelectCharScreen.VecSelectChar = makeCharList(2); // count=2, slot 2 is empty/unavailable
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("unavailable slot 2 refuses MsgDialog.isAutologin", !MsgDialog.isAutologin);
        check("no fallback to slot 0 in GameCanvas.selectChar.selectChar", GameCanvas.selectChar.selectChar != 0);

        // ---------------------------------------------------------------------
        // Test 5: Null character object at requested position refuses world entry
        // ---------------------------------------------------------------------
        System.out.println("--- Test 5: null character object at target index refuses ---");
        System.setProperty("zeus.auth.slot", "0");
        mVector listWithNull = new mVector("TestNull");
        listWithNull.addElement(null);
        SelectCharScreen.VecSelectChar = listWithNull;
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("null character object refuses MsgDialog.isAutologin", !MsgDialog.isAutologin);

        // ---------------------------------------------------------------------
        // Test 6: Malformed / out-of-range JVM property fails closed
        // ---------------------------------------------------------------------
        System.out.println("--- Test 6: malformed / out-of-range property fails closed ---");
        SelectCharScreen.VecSelectChar = makeCharList(3);

        System.setProperty("zeus.auth.slot", "3"); // outside 0..2
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("slot 3 fails closed (MsgDialog.isAutologin is false)", !MsgDialog.isAutologin);

        System.setProperty("zeus.auth.slot", "-1"); // negative
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("slot -1 fails closed (MsgDialog.isAutologin is false)", !MsgDialog.isAutologin);

        System.setProperty("zeus.auth.slot", "bad"); // non-numeric
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("slot bad fails closed (MsgDialog.isAutologin is false)", !MsgDialog.isAutologin);

        // ---------------------------------------------------------------------
        // Test 7: Absent property preserves legacy internal default 0
        // ---------------------------------------------------------------------
        System.out.println("--- Test 7: absent property preserves legacy internal default 0 ---");
        System.clearProperty("zeus.auth.slot");
        SelectCharScreen.VecSelectChar = makeCharList(1);
        GameCanvas.selectChar.selectChar = -1;
        MsgDialog.isAutologin = false;
        call("authReset");
        call("auth");
        check("absent property selects slot 0", MsgDialog.isAutologin && GameCanvas.selectChar.selectChar == 0);

        // ---------------------------------------------------------------------
        // Test 8: Valid-slot bounded auth retry repeats identical slot
        // ---------------------------------------------------------------------
        System.out.println("--- Test 8: bounded auth retry repeats identical slot ---");
        System.setProperty("zeus.auth.slot", "1");
        SelectCharScreen.VecSelectChar = makeCharList(2);
        call("authReset");
        MsgDialog.isAutologin = false;
        call("auth");
        check("initial attempt selects slot 1", MsgDialog.isAutologin && GameCanvas.selectChar.selectChar == 1);
        MsgDialog.isAutologin = false;

        // Tick past retry interval (75 ticks)
        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("retry attempt 2 re-submits slot 1", MsgDialog.isAutologin && GameCanvas.selectChar.selectChar == 1);
        MsgDialog.isAutologin = false;

        for (int i = 0; i < 76; i++) {
            call("auth");
        }
        check("retry attempt 3 re-submits slot 1", MsgDialog.isAutologin && GameCanvas.selectChar.selectChar == 1);
        MsgDialog.isAutologin = false;

        for (int i = 0; i < 150; i++) {
            call("auth");
        }
        check("exhausted retries stops submitting", !MsgDialog.isAutologin);

        // ---------------------------------------------------------------------
        // Test 9: Unavailable slot does not generate repeated auth submissions
        // ---------------------------------------------------------------------
        System.out.println("--- Test 9: unavailable slot does not consume retry submissions ---");
        System.setProperty("zeus.auth.slot", "2");
        SelectCharScreen.VecSelectChar = makeCharList(1); // slot 2 unavailable
        call("authReset");
        MsgDialog.isAutologin = false;
        call("auth");
        check("initial tick does not submit unavailable slot", !MsgDialog.isAutologin);

        for (int i = 0; i < 200; i++) {
            call("auth");
        }
        check("unavailable slot never submits after 200 ticks", !MsgDialog.isAutologin);
        check("unavailable slot does not exhaust authAttempts", ((Integer) get("authAttempts")).intValue() == 0);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
