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

    private static void setManagedHost(String host) {
        if (host == null) {
            System.clearProperty("zeus.server.host");
        } else {
            System.setProperty("zeus.server.host", host);
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

        // ── Phase 3 Case 1: Bạch Hổ correct managed state (prop hs8, rms hs8, row 0 hs8) -> true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0;
        assertTrue(Zeus.serverTargetSafe(), "Bạch Hổ correct managed state must be safe");
        System.out.println("  [PASS] Case 1: Bạch Hổ correct managed state (prop hs8 + rms hs8 + row 0 hs8) -> true");

        // ── Phase 3 Case 2: Native UI rewrites Bạch Hổ RMS to Chiến Thần (prop hs8, rms hs1, row 1 hs1) -> false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost("hs1.teamobi.com");
        GameCanvas.IndexServer = 1;
        assertTrue(!Zeus.serverTargetSafe(), "Native UI rewrites Bạch Hổ RMS to Chiến Thần must fail closed");
        System.out.println("  [PASS] Case 2: native UI rewrites Bạch Hổ RMS to Chiến Thần -> false");

        // ── Phase 3 Case 3: RMS still Bạch Hổ but current runtime row changed (prop hs8, rms hs8, row 1 hs1) -> false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 1;
        assertTrue(!Zeus.serverTargetSafe(), "RMS still Bạch Hổ but runtime row changed to hs1 must fail closed");
        System.out.println("  [PASS] Case 3: RMS still Bạch Hổ but current runtime row changed -> false");

        // ── Phase 3 Case 4: selectedServerHost deleted from managed profile (prop hs8, rms null, row 0 hs8) -> false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost(null);
        GameCanvas.IndexServer = 0;
        assertTrue(!Zeus.serverTargetSafe(), "Deleted selectedServerHost RMS in managed mode must fail closed");
        System.out.println("  [PASS] Case 4: selectedServerHost deleted from managed profile -> false");

        // ── Phase 3 Case 5: selectedServerHost malformed in managed profile -> false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0;
        setSavedServerHostRaw(new byte[0]);
        assertTrue(!Zeus.serverTargetSafe(), "Empty selectedServerHost RMS in managed mode must be false");
        setSavedServerHost("   ");
        assertTrue(!Zeus.serverTargetSafe(), "Whitespace selectedServerHost RMS in managed mode must be false");
        System.out.println("  [PASS] Case 5: selectedServerHost malformed/empty in managed profile -> false");

        // ── Case 6: Managed property empty/whitespace -> false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0;
        setManagedHost("");
        assertTrue(!Zeus.serverTargetSafe(), "Empty zeus.server.host property must fail closed");
        setManagedHost("   ");
        assertTrue(!Zeus.serverTargetSafe(), "Whitespace zeus.server.host property must fail closed");
        System.out.println("  [PASS] Case 6: empty/whitespace zeus.server.host property -> false");

        // ── Phase 3 Case 7: Unmanaged official / manual runtime (prop null) -> true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost(null);
        setSavedServerHost(null);
        GameCanvas.IndexServer = 0;
        assertTrue(Zeus.serverTargetSafe(), "Unmanaged mode with missing RMS must return true (preserve official behavior)");
        setSavedServerHost("hs8.teamobi.com");
        assertTrue(Zeus.serverTargetSafe(), "Unmanaged mode with present RMS must return true (preserve official behavior)");
        System.out.println("  [PASS] Case 7: unmanaged official/manual runtime (property null) -> true");

        // ── Case 8: Legacy server live validation / managed state (prop hs1, rms hs1, row 1 hs1) -> true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs1.teamobi.com");
        setSavedServerHost("hs1.teamobi.com");
        GameCanvas.IndexServer = 1;
        assertTrue(Zeus.serverTargetSafe(), "Legacy server ID 0 (hs1) managed state must be safe");
        System.out.println("  [PASS] Case 8: legacy server ID 0 (hs1) managed state -> true");

        // ── Case 9: Legacy server live validation / managed state (prop hs4, rms hs4, row 8 hs4) -> true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs4.teamobi.com");
        setSavedServerHost("hs4.teamobi.com");
        GameCanvas.IndexServer = 8;
        assertTrue(Zeus.serverTargetSafe(), "Legacy server ID 7 (hs4) managed state must be safe");
        System.out.println("  [PASS] Case 9: legacy server ID 7 (hs4) managed state -> true");

        // ── Case 10: Static fallback + saved hs8 + bootstrap IndexServer 0 (runtime host hs1) -> false ──
        invokeApplyServerList(STATIC_8_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0; // bootstrap index 0 maps to hs1 on static 8-server list
        assertTrue(!Zeus.serverTargetSafe(), "Static fallback + saved hs8 + IndexServer 0 (hs1) must be false");
        System.out.println("  [PASS] Case 10: static fallback + saved hs8 + bootstrap IndexServer 0 (hs1) -> false");

        // ── Case 11: Out-of-range IndexServer -> false without throwing ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = -1;
        assertTrue(!Zeus.serverTargetSafe(), "IndexServer = -1 must return false without throwing");
        GameCanvas.IndexServer = 9;
        assertTrue(!Zeus.serverTargetSafe(), "IndexServer = 9 must return false without throwing");
        GameCanvas.IndexServer = 127;
        assertTrue(!Zeus.serverTargetSafe(), "IndexServer = 127 must return false without throwing");
        System.out.println("  [PASS] Case 11: out-of-range IndexServer -> false without throwing");

        // ── Case 12: Null listServer -> false without throwing ──
        String[][] savedList = mSystem.listServer;
        mSystem.listServer = null;
        GameCanvas.IndexServer = 0;
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost("hs8.teamobi.com");
        assertTrue(!Zeus.serverTargetSafe(), "null listServer must return false without throwing");
        mSystem.listServer = savedList;
        System.out.println("  [PASS] Case 12: null listServer -> false without throwing");

        // ── Case 13: GameCanvas.connect guard prevents native socket-connect path when false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
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
        System.out.println("  [PASS] Case 13: GameCanvas.connect guard prevents socket-connect path when false");

        // ── Case 14: LoginScreen.login guard prevents reaching GlobalService.login/opcode 1 path when false ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
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
        System.out.println("  [PASS] Case 14: LoginScreen.login guard prevents reaching GlobalService.login/opcode 1 path when false");

        // ── Case 15: Both methods proceed normally when host validation is true ──
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        setManagedHost("hs8.teamobi.com");
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = 0; // points to hs8.teamobi.com -> serverTargetSafe is true
        assertTrue(Zeus.serverTargetSafe(), "Precondition: serverTargetSafe must be true");

        // 15a: LoginScreen.login proceeds past guard to create opcode 1
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

        // 15b: GameCanvas.connect proceeds to native connect
        fConnecting.setBoolean(Session_ME.gI(), false);
        fConnected.setBoolean(Session_ME.gI(), false);
        java.io.ByteArrayOutputStream baos15 = new java.io.ByteArrayOutputStream();
        System.setOut(new java.io.PrintStream(baos15));
        try {
            GameCanvas.connect();
        } finally {
            System.setOut(origOut);
        }
        assertTrue(baos15.toString().contains("------------IP host:hs8.teamobi.com--port:19129"),
            "GameCanvas.connect when serverTargetSafe is true MUST proceed to native connect path (host: hs8.teamobi.com)");
        fConnecting.setBoolean(Session_ME.gI(), false);
        fConnected.setBoolean(Session_ME.gI(), false);
        System.out.println("  [PASS] Case 15: both connect and login proceed normally when host validation is true");

        System.out.println("ALL 15 ServerFailClosedTest CHECKS PASSED!");
    }
}
