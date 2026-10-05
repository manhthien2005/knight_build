import GameScreen.LogoScreen;
import GameScreen.LoginScreen;
import Main.GameCanvas;
import CLib.mSystem;
import CLib.Session_ME;
import netcommand.global.GlobalService;
import javax.microedition.rms.RecordStore;
import org.microemu.MIDletBridge;
import org.microemu.MicroEmulator;
import org.microemu.RecordStoreManager;
import org.microemu.util.MemoryRecordStoreManager;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationHandler;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;

/**
 * ServerFailClosedTest — Verifies Canonical Host Guard & Choke-Point Security:
 * 1. Nine-server cached/live list + saved hs8 + IndexServer 0 -> serverTargetSafe true.
 * 2. Nine-server list + saved hs1 + IndexServer 1 -> true.
 * 3. Nine-server list + saved hs4 + IndexServer 8 -> true.
 * 4. Static official fallback + saved hs8 + bootstrap IndexServer 0 (runtime host hs1) -> false.
 * 5. Nine-server list + unknown saved host + IndexServer 0 -> false.
 * 6. Malformed selectedServerHost RMS -> false.
 * 7. Out-of-range IndexServer -> false without throwing.
 * 8. Null listServer -> false without throwing.
 * 9. Missing selectedServerHost store -> true, preserving unmanaged/manual official behavior.
 * 10. GameCanvas.connect guard prevents native socket-connect path when false.
 * 11. LoginScreen.login guard prevents reaching GlobalService.login/opcode 1 path when false.
 * 12. Both methods proceed normally when host validation is true.
 */
public class ServerFailClosedTest {

    private static final String CAPTURED_9_SERVER_PAYLOAD =
        "Bạch Hổ New:hs8.teamobi.com:19129:0," +
        "Chiến Thần:hs1.teamobi.com:19129:0," +
        "Rồng Lửa:hs2.teamobi.com:19129:0," +
        "Global Server:hsglobal.teamobi.com:19129:1," +
        "Phượng Hoàng:hs3.teamobi.com:19129:0," +
        "Nhân Mã:hs5.teamobi.com:19129:0," +
        "Kì Lân:hs6.teamobi.com:19129:0," +
        "Thiên Hà:hs7.teamobi.com:19129:0," +
        "Thách Đấu:hs4.teamobi.com:19129:0,";

    private static final String STATIC_8_SERVER_PAYLOAD =
        "Chiến Thần:hs1.teamobi.com:19129:0," +
        "Rồng Lửa:hs2.teamobi.com:19129:0," +
        "Global Server:hsglobal.teamobi.com:19129:1," +
        "Phượng Hoàng:hs3.teamobi.com:19129:0," +
        "Nhân Mã:hs5.teamobi.com:19129:0," +
        "Kì Lân:hs6.teamobi.com:19129:0," +
        "Thiên Hà:hs7.teamobi.com:19129:0," +
        "Thách Đấu:hs4.teamobi.com:19129:0,";

    private static void assertTrue(boolean condition, String message) {
        if (!condition) {
            throw new AssertionError("FAILED: " + message);
        }
    }

    private static void assertEquals(int expected, int actual, String message) {
        if (expected != actual) {
            throw new AssertionError("FAILED: " + message + " (expected " + expected + ", got " + actual + ")");
        }
    }

    private static void setSavedServerHost(String host) {
        try {
            try {
                RecordStore.deleteRecordStore("selectedServerHost");
            } catch (Throwable ignored) {}
            if (host != null) {
                Model.CRes.saveRMS("selectedServerHost", host.getBytes("UTF-8"));
            }
        } catch (Exception e) {
            throw new RuntimeException("failed to set savedServerHost: " + e.getMessage(), e);
        }
    }

    private static void setSavedServerHostRaw(byte[] raw) {
        try {
            try {
                RecordStore.deleteRecordStore("selectedServerHost");
            } catch (Throwable ignored) {}
            if (raw != null) {
                RecordStore rs = RecordStore.openRecordStore("selectedServerHost", true);
                rs.addRecord(raw, 0, raw.length);
                rs.closeRecordStore();
            }
        } catch (Exception e) {
            throw new RuntimeException("failed to set savedServerHostRaw: " + e.getMessage(), e);
        }
    }

    static class HeadlessFont extends CLib.mFont {
        public HeadlessFont() {
            super(null, null, null, 0);
        }

        public String[] splitFontArray(String text, int width) {
            return new String[] { text };
        }
    }

    private static void initHeadlessEnvironment() {
        try {
            GameCanvas.w = 240;
            GameCanvas.h = 320;
            Field uf = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
            uf.setAccessible(true);
            Object unsafe = uf.get(null);
            Method alloc = unsafe.getClass().getMethod("allocateInstance", Class.class);
            CLib.mFont mockFont = (CLib.mFont) alloc.invoke(unsafe, HeadlessFont.class);
            CLib.mFont.tahoma_7b_white = mockFont;
            CLib.mFont.tahoma_7_white = mockFont;
            CLib.mFont.tahoma_7b_yellow = mockFont;
            CLib.mFont.tahoma_7_yellow = mockFont;

            final RecordStoreManager rsm = new MemoryRecordStoreManager();
            MicroEmulator emulator = (MicroEmulator) Proxy.newProxyInstance(
                MicroEmulator.class.getClassLoader(),
                new Class<?>[] { MicroEmulator.class },
                new InvocationHandler() {
                    public Object invoke(Object proxy, Method method, Object[] args) throws Throwable {
                        if ("getRecordStoreManager".equals(method.getName())) {
                            return rsm;
                        }
                        return null;
                    }
                }
            );
            MIDletBridge.setMicroEmulator(emulator);
        } catch (Exception e) {
            throw new RuntimeException("failed to init headless environment: " + e.getMessage(), e);
        }
    }

    private static void invokeApplyServerList(String payload, boolean bl) {
        try {
            LogoScreen.applyServerList(payload, bl);
        } catch (NullPointerException npe) {
            // In headless unit test mode, setChangeLang() -> loadCaptionCmd() throws
            // NPE because GUI screens (GameCanvas.login, etc.) are only instantiated by TemMidlet.
            // GameCanvas.IndexServer and listServer have already been committed before setChangeLang().
        }
    }

    public static void main(String[] args) throws Exception {
        System.out.println("Running ServerFailClosedTest...");
        initHeadlessEnvironment();

        // ── Case 1: Nine-server cached/live list + saved hs8 + IndexServer 0 -> serverTargetSafe true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0;
        assertTrue(Zeus.serverTargetSafe(), "Nine-server list + saved hs8 + IndexServer 0 must be safe");
        System.out.println("  [PASS] Case 1: 9-server list + saved hs8 + IndexServer 0 -> true");

        // ── Case 2: Nine-server list + saved hs1 + IndexServer 1 -> true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs1.teamobi.com");
        GameCanvas.IndexServer = 1;
        assertTrue(Zeus.serverTargetSafe(), "Nine-server list + saved hs1 + IndexServer 1 must be safe");
        System.out.println("  [PASS] Case 2: 9-server list + saved hs1 + IndexServer 1 -> true");

        // ── Case 3: Nine-server list + saved hs4 + IndexServer 8 -> true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs4.teamobi.com");
        GameCanvas.IndexServer = 8;
        assertTrue(Zeus.serverTargetSafe(), "Nine-server list + saved hs4 + IndexServer 8 must be safe");
        System.out.println("  [PASS] Case 3: 9-server list + saved hs4 + IndexServer 8 -> true");

        // ── Case 4: Static official fallback + saved hs8 + bootstrap IndexServer 0 (runtime host hs1) -> false ──
        invokeApplyServerList(STATIC_8_SERVER_PAYLOAD, false);
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0; // bootstrap index 0 maps to hs1 on static 8-server list
        assertTrue(!Zeus.serverTargetSafe(), "Static fallback + saved hs8 + IndexServer 0 (hs1) must be false");
        System.out.println("  [PASS] Case 4: static fallback + saved hs8 + bootstrap IndexServer 0 (hs1) -> false");

        // ── Case 5: Nine-server list + unknown saved host + IndexServer 0 -> false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs99.unknown.teamobi.com");
        GameCanvas.IndexServer = 0;
        assertTrue(!Zeus.serverTargetSafe(), "Nine-server list + unknown saved host must be false");
        System.out.println("  [PASS] Case 5: 9-server list + unknown saved host -> false");

        // ── Case 6: Malformed selectedServerHost RMS -> false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        GameCanvas.IndexServer = 0;
        setSavedServerHostRaw(new byte[0]);
        assertTrue(!Zeus.serverTargetSafe(), "Empty selectedServerHost RMS must be false");
        setSavedServerHost("   ");
        assertTrue(!Zeus.serverTargetSafe(), "Whitespace selectedServerHost RMS must be false");
        System.out.println("  [PASS] Case 6: malformed selectedServerHost RMS -> false");

        // ── Case 7: Out-of-range IndexServer -> false without throwing ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = -1;
        assertTrue(!Zeus.serverTargetSafe(), "IndexServer = -1 must return false without throwing");
        GameCanvas.IndexServer = 9;
        assertTrue(!Zeus.serverTargetSafe(), "IndexServer = 9 must return false without throwing");
        GameCanvas.IndexServer = 127;
        assertTrue(!Zeus.serverTargetSafe(), "IndexServer = 127 must return false without throwing");
        System.out.println("  [PASS] Case 7: out-of-range IndexServer -> false without throwing");

        // ── Case 8: Null listServer -> false without throwing ──
        String[][] savedList = mSystem.listServer;
        mSystem.listServer = null;
        GameCanvas.IndexServer = 0;
        setSavedServerHost("hs8.teamobi.com");
        assertTrue(!Zeus.serverTargetSafe(), "null listServer must return false without throwing");
        mSystem.listServer = savedList;
        System.out.println("  [PASS] Case 8: null listServer -> false without throwing");

        // ── Case 9: Missing selectedServerHost store -> true, preserving unmanaged/manual official behavior ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        GameCanvas.IndexServer = 0;
        setSavedServerHost(null); // deletes store so loadRMS returns null
        assertTrue(Zeus.serverTargetSafe(), "Missing selectedServerHost store must return true (unmanaged mode)");
        System.out.println("  [PASS] Case 9: missing selectedServerHost store -> true (unmanaged/manual behavior preserved)");

        // ── Case 10: GameCanvas.connect guard prevents native socket-connect path when false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 1; // runtime host is hs1 -> mismatch -> serverTargetSafe is false
        assertTrue(!Zeus.serverTargetSafe(), "Precondition: serverTargetSafe must be false");

        Field fConnecting = Session_ME.class.getDeclaredField("connecting");
        Field fConnected = Session_ME.class.getDeclaredField("connected");
        fConnecting.setAccessible(true);
        fConnected.setAccessible(true);
        fConnecting.setBoolean(Session_ME.gI(), false);
        fConnected.setBoolean(Session_ME.gI(), false);

        java.io.ByteArrayOutputStream baos10 = new java.io.ByteArrayOutputStream();
        java.io.PrintStream origOut = System.out;
        System.setOut(new java.io.PrintStream(baos10));
        try {
            GameCanvas.connect();
        } finally {
            System.setOut(origOut);
        }
        assertTrue(!baos10.toString().contains("------------IP host:"),
            "connect() when serverTargetSafe is false must not invoke native connect");

        boolean connectingAfter = fConnecting.getBoolean(Session_ME.gI());
        boolean connectedAfter = Session_ME.gI().isConnected();
        assertTrue(!connectingAfter && !connectedAfter, "connect() when serverTargetSafe is false must NOT open socket or begin connecting");
        System.out.println("  [PASS] Case 10: GameCanvas.connect guard prevents socket-connect path when false");

        // ── Case 11: LoginScreen.login guard prevents reaching GlobalService.login/opcode 1 path when false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 1; // serverTargetSafe is false
        assertTrue(!Zeus.serverTargetSafe(), "Precondition: serverTargetSafe must be false");

        Field fMsg = netcommand.Cmd_Message.class.getDeclaredField("m");
        fMsg.setAccessible(true);
        fMsg.set(GlobalService.gI(), null);

        Method mLogin = LoginScreen.class.getDeclaredMethod("login", String.class, String.class);
        mLogin.setAccessible(true);

        Field uf = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        uf.setAccessible(true);
        sun.misc.Unsafe unsafe = (sun.misc.Unsafe) uf.get(null);
        LoginScreen loginScreen = (LoginScreen) unsafe.allocateInstance(LoginScreen.class);

        mLogin.invoke(loginScreen, "testUser", "testPass");
        Object msgAfter = fMsg.get(GlobalService.gI());
        assertTrue(msgAfter == null, "LoginScreen.login when serverTargetSafe is false must NOT reach GlobalService.login / opcode 1");
        System.out.println("  [PASS] Case 11: LoginScreen.login guard prevents reaching GlobalService.login/opcode 1 path when false");

        // ── Case 12: Both methods proceed normally when host validation is true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0; // points to hs8.teamobi.com -> serverTargetSafe is true
        assertTrue(Zeus.serverTargetSafe(), "Precondition: serverTargetSafe must be true");

        // 12a: LoginScreen.login proceeds past guard to create opcode 1
        Field fu = LoginScreen.class.getDeclaredField("tfusername");
        Field fp = LoginScreen.class.getDeclaredField("tfpassword");
        fu.setAccessible(true);
        CLib.TField dummyTf1 = (CLib.TField) unsafe.allocateInstance(CLib.TField.class);
        CLib.TField dummyTf2 = (CLib.TField) unsafe.allocateInstance(CLib.TField.class);
        Field fText = CLib.TField.class.getDeclaredField("text");
        fText.setAccessible(true);
        fText.set(dummyTf1, "");
        fText.set(dummyTf2, "");
        fu.set(null, dummyTf1);
        fp.set(null, dummyTf2);
        GameScreen.WorldMapScreen.namePos = new String[0];
        InterfaceComponents.TabQuest.nameItemQuest = new String[0];
        fMsg.set(GlobalService.gI(), null);

        try {
            mLogin.invoke(loginScreen, "testUser", "testPass");
        } catch (Exception ignored) {
        }
        Object msgSuccess = fMsg.get(GlobalService.gI());
        assertTrue(msgSuccess != null, "LoginScreen.login when serverTargetSafe is true MUST proceed to create opcode 1 message");
        net.Message sentMsg = (net.Message) msgSuccess;
        assertEquals(1, (int) sentMsg.command, "Produced message must have opcode 1");

        // 12b: GameCanvas.connect proceeds to native connect
        fConnecting.setBoolean(Session_ME.gI(), false);
        fConnected.setBoolean(Session_ME.gI(), false);
        java.io.ByteArrayOutputStream baos12 = new java.io.ByteArrayOutputStream();
        System.setOut(new java.io.PrintStream(baos12));
        try {
            GameCanvas.connect();
        } finally {
            System.setOut(origOut);
        }
        assertTrue(baos12.toString().contains("------------IP host:hs8.teamobi.com--port:19129"),
            "GameCanvas.connect when serverTargetSafe is true MUST proceed to native connect path (host: hs8.teamobi.com)");
        fConnecting.setBoolean(Session_ME.gI(), false);
        fConnected.setBoolean(Session_ME.gI(), false);
        System.out.println("  [PASS] Case 12: both connect and login proceed normally when host validation is true");

        System.out.println("ALL 12 ServerFailClosedTest CHECKS PASSED!");
    }
}
