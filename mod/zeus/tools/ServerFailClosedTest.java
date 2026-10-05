import GameScreen.LogoScreen;
import Main.GameCanvas;
import CLib.mSystem;
import CLib.Session_ME;

/**
 * ServerFailClosedTest — Verifies Phase 3 and Phase 4:
 * 1. LogoScreen.applyServerList host resolution and fail-closed host-miss patch:
 *    - hs8 -> live index 0 when present.
 *    - hs1 -> live index 1 when present.
 *    - hs4 -> live index 8 when present.
 *    - unknown saved host -> fail-closed (IndexServer = -1, NEVER live index 0).
 *    - saved host null -> official manual behavior preserved (IndexServer = 0).
 * 2. Downstream connection path safety:
 *    - IndexServer = -1 returns immediately without opening socket.
 *    - IndexServer = 8 against 8-server list returns immediately without opening socket.
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
            java.lang.reflect.Field field = LogoScreen.class.getDeclaredField("savedServerHost");
            field.setAccessible(true);
            field.set(null, host);
        } catch (Exception e) {
            throw new RuntimeException("failed to set savedServerHost: " + e.getMessage(), e);
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
            java.lang.reflect.Field uf = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
            uf.setAccessible(true);
            Object unsafe = uf.get(null);
            java.lang.reflect.Method alloc = unsafe.getClass().getMethod("allocateInstance", Class.class);
            CLib.mFont mockFont = (CLib.mFont) alloc.invoke(unsafe, HeadlessFont.class);
            CLib.mFont.tahoma_7b_white = mockFont;
            CLib.mFont.tahoma_7_white = mockFont;
            CLib.mFont.tahoma_7b_yellow = mockFont;
            CLib.mFont.tahoma_7_yellow = mockFont;
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
            // GameCanvas.IndexServer and listServer RMS have already been committed before setChangeLang().
        }
    }

    public static void main(String[] args) {
        System.out.println("Running ServerFailClosedTest...");
        initHeadlessEnvironment();

        // ── Test 1: hs8 resolves to live index 0 ──
        setSavedServerHost("hs8.teamobi.com");
        GameCanvas.IndexServer = -99;
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        assertEquals(0, GameCanvas.IndexServer, "hs8 must resolve to live index 0");
        System.out.println("  [PASS] hs8 -> live index 0");

        // ── Test 2: hs1 resolves to live index 1 ──
        setSavedServerHost("hs1.teamobi.com");
        GameCanvas.IndexServer = -99;
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        assertEquals(1, GameCanvas.IndexServer, "hs1 must resolve to live index 1");
        System.out.println("  [PASS] hs1 -> live index 1");

        // ── Test 3: hs4 resolves to live index 8 ──
        setSavedServerHost("hs4.teamobi.com");
        GameCanvas.IndexServer = -99;
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        assertEquals(8, GameCanvas.IndexServer, "hs4 must resolve to live index 8");
        System.out.println("  [PASS] hs4 -> live index 8");

        // ── Test 4: Unknown host -> FAIL-CLOSED sentinel -1, NEVER index 0 ──
        setSavedServerHost("hs99.unknown.teamobi.com");
        GameCanvas.IndexServer = 0; // deliberately set to 0 beforehand
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        assertEquals(-1, GameCanvas.IndexServer, "Unknown saved host must fail-closed to -1, never index 0");
        System.out.println("  [PASS] unknown saved host -> fail-closed (IndexServer = -1)");

        // ── Test 5: null saved host -> official manual behavior preserved (index 0) ──
        setSavedServerHost(null);
        GameCanvas.IndexServer = -99;
        invokeApplyServerList(CAPTURED_9_SERVER_PAYLOAD, false);
        assertEquals(0, GameCanvas.IndexServer, "Null saved host must preserve official manual behavior (index 0)");
        System.out.println("  [PASS] null saved host -> official manual behavior preserved (index 0)");

        // ── Test 6: Downstream connect safety with IndexServer = -1 ──
        GameCanvas.IndexServer = -1;
        // GameCanvas.connect() must check if IndexServer < 0 and return immediately
        try {
            GameCanvas.connect();
            boolean connected = Session_ME.gI().isConnected();
            assertTrue(!connected, "connect() with IndexServer=-1 must not establish a connection");
            System.out.println("  [PASS] downstream connect safely rejects IndexServer = -1");
        } catch (Exception e) {
            throw new AssertionError("connect() with IndexServer=-1 threw unexpected exception: " + e.getMessage());
        }

        // ── Test 7: Downstream connect safety with IndexServer = 8 on 8-server list ──
        String static8ServerPayload =
            "Chiến Thần:hs1.teamobi.com:19129:0," +
            "Rồng Lửa:hs2.teamobi.com:19129:0," +
            "Global Server:hsglobal.teamobi.com:19129:1," +
            "Phượng Hoàng:hs3.teamobi.com:19129:0," +
            "Nhân Mã:hs5.teamobi.com:19129:0," +
            "Kì Lân:hs6.teamobi.com:19129:0," +
            "Thiên Hà:hs7.teamobi.com:19129:0," +
            "Thách Đấu:hs4.teamobi.com:19129:0,";
        setSavedServerHost(null);
        invokeApplyServerList(static8ServerPayload, false);
        assertEquals(8, mSystem.listServer.length, "8-server list length must be 8");

        GameCanvas.IndexServer = 8; // out-of-range for 8-server table
        try {
            GameCanvas.connect();
            boolean connected = Session_ME.gI().isConnected();
            assertTrue(!connected, "connect() with IndexServer=8 on 8-server table must not establish connection");
            System.out.println("  [PASS] downstream connect safely rejects IndexServer = 8 on 8-server list");
        } catch (Exception e) {
            throw new AssertionError("connect() with IndexServer=8 on 8-server list threw unexpected exception: " + e.getMessage());
        }

        System.out.println("ALL ServerFailClosedTest CHECKS PASSED!");
    }
}
