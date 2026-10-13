import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.util.Locale;
import CLib.mVector;
import InterfaceComponents.iCommand;

/**
 * Deterministic, standalone test suite for Passive Boss Menu Capture (R2.3B Canonical Named ABI).
 * Hardened for R2.3B.1 findings:
 * - OBS-01: Proven boss context required for MATCH_SPAWNED and MATCH_UNAVAILABLE
 * - OBS-02: Distinct mapId and zone fields in capture JSON
 * - OBS-03: Locale-independent casefolding under tr_TR
 * - OBS-04: Unicode surrogate pair boundary preservation at truncation
 * - OBS-05: Profileless fallback fail-closed
 * - OBS-06: Sensitive exclusion on matching boss titles
 */
public class BossMenuObserverTest {

    private static int failures = 0;
    private static int total = 0;

    private static void check(String label, boolean ok) {
        total++;
        if (ok) {
            System.out.println("PASS: " + label);
        } else {
            System.out.println("FAIL: " + label);
            failures++;
        }
    }

    private static void checkEquals(String label, Object expected, Object actual) {
        total++;
        boolean ok;
        if (expected == null && actual == null) {
            ok = true;
        } else if (expected instanceof Number && actual instanceof Number) {
            ok = ((Number) expected).longValue() == ((Number) actual).longValue();
        } else {
            ok = (expected != null && expected.equals(actual));
        }
        if (ok) {
            System.out.println("PASS: " + label + " (" + actual + ")");
        } else {
            System.out.println("FAIL: " + label + " -> Expected [" + expected + "], got [" + actual + "]");
            failures++;
        }
    }

    public static void main(String[] args) {
        System.out.println("=== BossMenuObserverTest Suite Starting ===");

        testClassification_Positive();
        testClassification_Negative_GenericAndNonBoss();
        testClassification_SensitiveExclusion();
        testLocaleInsensitivity();
        testUnicodeSurrogateBoundaries();
        testDisabledByDefault();
        testProfilelessFallback_FailClosed();
        testRealNpcScreenshots_R3_2();
        testActiveCapture_LocalAndServer();
        testDeduplication();
        testBoundedByteLimitsAndMultibyte();
        testMenuStateAndObjectsUnmutated();

        System.out.println("\n=== Test Summary: " + (total - failures) + "/" + total + " passed ===");
        if (failures > 0) {
            System.err.println("TEST SUITE FAILED with " + failures + " failures.");
            System.exit(1);
        } else {
            System.out.println("ALL TESTS PASSED.");
        }
    }

    /**
     * Positive proven Boss menu classification (B-03 safe codes).
     */
    private static void testClassification_Positive() {
        System.out.println("\n--- testClassification_Positive ---");

        // 1. Proven by Title
        String rTitle1 = Zeus.classifyBossMenu("Thông tin Boss", new String[] { "Xem lịch", "Thoát" });
        checkEquals("Title 'Thông tin Boss' returns exact MATCH_TITLE", "MATCH_TITLE", rTitle1);

        String rTitle2 = Zeus.classifyBossMenu("Lịch Boss", new String[] { "Xem", "Đóng" });
        checkEquals("Title 'Lịch Boss' returns exact MATCH_TITLE", "MATCH_TITLE", rTitle2);

        String rAlisama = Zeus.classifyBossMenu("NPC Alisama", new String[] { "Lịch săn", "Đóng" });
        checkEquals("Title with Alisama returns exact MATCH_TITLE", "MATCH_TITLE", rAlisama);

        String rDoubar = Zeus.classifyBossMenu("Doubar", new String[] { "Nhiệm vụ boss", "Đóng" });
        checkEquals("Title with Doubar returns exact MATCH_TITLE", "MATCH_TITLE", rDoubar);

        String rGiant = Zeus.classifyBossMenu("GIANT Skeleton", new String[] { "Xem lịch" });
        checkEquals("Boss title 'GIANT Skeleton' returns exact MATCH_TITLE", "MATCH_TITLE", rGiant);

        String rGiant2 = Zeus.classifyBossMenu("GIANT Skeleton", new String[] { "Bảng boss" });
        checkEquals("Boss title 'GIANT Skeleton' with neutral caption returns MATCH_TITLE", "MATCH_TITLE", rGiant2);

        // 2. Proven by Spawned Status in caption (with boss context)
        String rSpawned1 = Zeus.classifyBossMenu("Bảng thông báo", new String[] {
            "Dơi quỷ - Vực đá - Khu 2 : đang xuất hiện",
            "Đóng"
        });
        checkEquals("Caption with 'đang xuất hiện' returns exact MATCH_SPAWNED", "MATCH_SPAWNED", rSpawned1);

        String rSpawned2 = Zeus.classifyBossMenu("Menu", new String[] {
            "Thằn lằn - Khu 1: đã xuất hiện"
        });
        checkEquals("Caption with 'đã xuất hiện' returns exact MATCH_SPAWNED", "MATCH_SPAWNED", rSpawned2);

        // 3. Proven by Unavailable Status in caption (with boss context)
        String rUnavail1 = Zeus.classifyBossMenu("Menu", new String[] {
            "Hiện tại không có thông tin boss",
            "Đóng"
        });
        checkEquals("Caption with 'không có thông tin' returns exact MATCH_UNAVAILABLE", "MATCH_UNAVAILABLE", rUnavail1);

        String rUnavail2 = Zeus.classifyBossMenu("Bảng thông báo", new String[] {
            "Không có lịch boss xuất hiện hôm nay"
        });
        checkEquals("Caption with 'không có lịch' returns exact MATCH_UNAVAILABLE", "MATCH_UNAVAILABLE", rUnavail2);

        // 4. Proven by Schedule row
        String rSched = Zeus.classifyBossMenu("Bảng thông báo", new String[] {
            "Dơi quỷ 14:30:00 Núi đá Khu 2",
            "Thoát"
        });
        checkEquals("Caption with time + zone + known boss returns exact MATCH_BOSS_SCHEDULE", "MATCH_BOSS_SCHEDULE", rSched);

        // 5. Proven via mVector overload
        mVector vec = new mVector();
        vec.addElement(new iCommand("Dơi quỷ 14:30:00 Núi đá Khu 2", 0));
        vec.addElement(new iCommand("Đóng", 1));
        String rVec = Zeus.classifyBossMenu("Menu", vec);
        checkEquals("mVector classification returns MATCH_BOSS_SCHEDULE", "MATCH_BOSS_SCHEDULE", rVec);
    }

    /**
     * Negative classification controls (OBS-01 hardening).
     */
    private static void testClassification_Negative_GenericAndNonBoss() {
        System.out.println("\n--- testClassification_Negative_GenericAndNonBoss ---");

        // OBS-FP-TITLE-01: Shop titles with animal materials (da Sói, đồ Dơi) must be rejected
        String rShopWolf = Zeus.classifyBossMenu("Thương nhân bán da Sói", new String[] {
            "Mua trang bị", "Đóng"
        });
        check("OBS-FP-TITLE-01: 'Thương nhân bán da Sói' rejected as null", rShopWolf == null);

        String rShopBat = Zeus.classifyBossMenu("Cửa hàng bán đồ Dơi", new String[] {
            "Mua", "Đóng"
        });
        check("OBS-FP-TITLE-01: 'Cửa hàng bán đồ Dơi' rejected as null", rShopBat == null);

        // OBS-01: Merchant caption with 'đang xuất hiện' but no boss context must be rejected
        String rMerchant = Zeus.classifyBossMenu("Thương nhân", new String[] {
            "Máy bán hàng đang xuất hiện trên kệ"
        });
        check("OBS-01: Merchant status 'đang xuất hiện' rejected as null", rMerchant == null);

        // OBS-01: Chest caption with 'không có lịch' but no boss context must be rejected
        String rChest = Zeus.classifyBossMenu("Rương đồ", new String[] {
            "Không có lịch bảo trì"
        });
        check("OBS-01: Chest status 'không có lịch' rejected as null", rChest == null);

        // Generic event notice
        String rEvent = Zeus.classifyBossMenu("Bảng thông báo", new String[] {
            "Đang xuất hiện sự kiện nạp thẻ tích lũy"
        });
        check("Generic event 'đang xuất hiện' rejected as null", rEvent == null);

        // Generic tournament schedule
        String rTourn = Zeus.classifyBossMenu("Menu", new String[] {
            "Không có lịch thi đấu tuần này"
        });
        check("Generic tournament 'không có lịch' rejected as null", rTourn == null);

        // B-04: Generic time + zone without a known boss token must reject as null
        String rGenericTime = Zeus.classifyBossMenu("Bảng thông báo", new String[] {
            "Bảo trì định kỳ 14:30:00 Khu 2"
        });
        check("B-04: Generic time + zone without known boss rejected as null", rGenericTime == null);

        String rGenericTime2 = Zeus.classifyBossMenu("Đấu trường", new String[] {
            "Chiến trường mở lúc 19:00:00 tại Khu 1"
        });
        check("B-04: Generic arena time without known boss rejected as null", rGenericTime2 == null);

        // Non-boss NPC dialogue
        String rNpc = Zeus.classifyBossMenu("Thương nhân", new String[] {
            "Mua vật phẩm", "Bán vật phẩm", "Học kỹ năng", "Đóng"
        });
        check("Merchant NPC menu rejected as null", rNpc == null);

        // Inventory / storage menu
        String rInv = Zeus.classifyBossMenu("Rương đồ", new String[] {
            "Cất vào rương", "Lấy ra", "Khóa rương"
        });
        check("Inventory/chest menu rejected as null", rInv == null);

        // Malformed, empty, or null captions
        check("Null title and null captions rejected as null", Zeus.classifyBossMenu(null, (String[]) null) == null);
        check("Empty captions array rejected as null", Zeus.classifyBossMenu("Menu", new String[0]) == null);
        check("Null/blank strings rejected as null", Zeus.classifyBossMenu("", new String[] { "", null, "   " }) == null);
    }

    /**
     * Sensitive text and credentials exclusion (OBS-06 hardening).
     */
    private static void testClassification_SensitiveExclusion() {
        System.out.println("\n--- testClassification_SensitiveExclusion ---");

        // Basic sensitive terms in title
        check("Title with 'password' rejected", Zeus.classifyBossMenu("Nhập password giải mã Boss", new String[] { "14:30:00 Khu 1" }) == null);
        check("Title with 'tài khoản' rejected", Zeus.classifyBossMenu("Tài khoản săn Boss", new String[] { "14:30:00 Khu 1" }) == null);

        // Basic sensitive terms in captions
        check("Caption with 'mật khẩu' rejected", Zeus.classifyBossMenu("Thông tin Boss", new String[] { "Mật khẩu bảo vệ: 123456" }) == null);
        check("Caption with 'PIN' rejected", Zeus.classifyBossMenu("Thông tin Boss", new String[] { "Mã PIN: 0000" }) == null);
        check("Caption with 'token' rejected", Zeus.classifyBossMenu("Thông tin Boss", new String[] { "auth token: abc" }) == null);

        // OBS-06: Matching boss title with OTP / verification code in caption must be rejected
        check("OBS-06: Matching title with 'Mã xác nhận 123456' rejected",
                Zeus.classifyBossMenu("Thông tin Boss", new String[] { "Mã xác nhận 123456" }) == null);
        check("OBS-06: Matching title with 'OTP' rejected",
                Zeus.classifyBossMenu("Thông tin Boss", new String[] { "Mã OTP giao dịch: 9988" }) == null);
        check("OBS-06: Matching title with 'mật khẩu cấp 2' rejected",
                Zeus.classifyBossMenu("Lịch Boss", new String[] { "Nhập mật khẩu cấp 2" }) == null);
    }

    /**
     * Locale-insensitive casefolding (OBS-03 hardening under tr_TR).
     */
    private static void testLocaleInsensitivity() {
        System.out.println("\n--- testLocaleInsensitivity ---");

        Locale saved = Locale.getDefault();
        try {
            Locale.setDefault(new Locale("tr", "TR"));

            // Turkish dotless-i insensitive checks
            check("OBS-03: containsSensitive('LOGIN') under tr_TR", Zeus.containsSensitive("LOGIN"));
            check("OBS-03: containsSensitive('PASSWORD') under tr_TR", Zeus.containsSensitive("PASSWORD"));
            check("OBS-03: containsSensitive('USER') under tr_TR", Zeus.containsSensitive("USER"));
            check("OBS-03: containsSensitive('PIN') under tr_TR", Zeus.containsSensitive("PIN"));

            // Turkish boss title recognition
            String rGiant = Zeus.classifyBossMenu("GIANT Skeleton", new String[] { "Bảng boss" });
            checkEquals("OBS-03: 'GIANT Skeleton' recognized under tr_TR", "MATCH_TITLE", rGiant);

            String rTitle = Zeus.classifyBossMenu("LỊCH BOSS", new String[] { "Xem" });
            checkEquals("OBS-03: 'LỊCH BOSS' recognized under tr_TR", "MATCH_TITLE", rTitle);
        } finally {
            Locale.setDefault(saved);
        }
    }

    /**
     * Unicode surrogate pair truncation preservation (OBS-04 hardening).
     */
    private static void testUnicodeSurrogateBoundaries() {
        System.out.println("\n--- testUnicodeSurrogateBoundaries ---");

        String emoji = new String(Character.toChars(0x1F600)); // emoji U+1F600
        String input = "abc" + emoji;

        // Truncate at maxLen=4: boundary falls exactly between high and low surrogate
        String cut4 = Zeus.cleanBossString(input, 4);
        checkEquals("OBS-04: Truncate at surrogate boundary drops lone surrogate", "abc", cut4);
        checkEquals("OBS-04: Length is 3 chars", 3, cut4.length());
        boolean hasSurr4 = cut4.length() > 0 && Character.isSurrogate(cut4.charAt(cut4.length() - 1));
        check("OBS-04: No trailing surrogate", !hasSurr4);

        // Truncate at maxLen=5: complete surrogate pair preserved
        String cut5 = Zeus.cleanBossString(input, 5);
        checkEquals("OBS-04: Complete surrogate pair preserved at maxLen=5", input, cut5);

        // Single emoji at maxLen=1: drops high surrogate, returns empty string
        String cut1 = Zeus.cleanBossString(emoji, 1);
        checkEquals("OBS-04: High surrogate alone at maxLen=1 yields empty string", "", cut1);
    }

    /**
     * Disabled by default: marker absent produces 0 writes.
     */
    private static void testDisabledByDefault() {
        System.out.println("\n--- testDisabledByDefault ---");

        File marker = new File("test_boss_capture_absent.on");
        File log = new File("test_boss_capture_absent.txt");
        if (marker.exists()) marker.delete();
        if (log.exists()) log.delete();

        Zeus.setBossCapturePaths(marker.getAbsolutePath(), log.getAbsolutePath());
        Zeus.bossCaptureReset();

        check("bossCaptureOn is false when marker absent", !Zeus.bossCaptureOn);

        mVector items = new mVector();
        items.addElement(new iCommand("Dơi quỷ - Vực đá - Khu 2 : đang xuất hiện", 0));
        boolean taken = Zeus.menu(items, "Thông tin Boss");

        check("Zeus.menu returns taken=false", !taken);
        check("Log file not created when disabled", !log.exists());
    }

    /**
     * Profileless fallback fails closed (OBS-05 hardening).
     */
    private static void testProfilelessFallback_FailClosed() {
        System.out.println("\n--- testProfilelessFallback_FailClosed ---");

        // When marker and out path are null, observer must remain disabled and fail silent
        Zeus.setBossCapturePaths(null, null);
        Zeus.bossCaptureReset();

        check("OBS-05: bossCaptureOn is false when paths are null", !Zeus.bossCaptureOn);

        mVector items = new mVector();
        items.addElement(new iCommand("Dơi quỷ - Vực đá - Khu 2 : đang xuất hiện", 0));
        Zeus.observeBossMenu("LOCAL", items, -1, -1, "Thông tin Boss", 1000L);
        Zeus.writeBossCapture("test line\n");

        File defaultMarker = new File("zeus-boss-capture.marker");
        File defaultLog = new File("zeus-boss-capture.txt");
        check("OBS-05: Default working-dir marker not created", !defaultMarker.exists());
        check("OBS-05: Default working-dir log not created", !defaultLog.exists());
    }

    /**
     * Active capture with local and server origin tagging and mapId/zone fields (OBS-02).
     */
    private static void testActiveCapture_LocalAndServer() {
        System.out.println("\n--- testActiveCapture_LocalAndServer ---");

        File marker = new File("test_boss_active.on");
        File log = new File("test_boss_active.txt");
        try {
            if (!marker.exists()) marker.createNewFile();
            if (log.exists()) log.delete();

            Zeus.setBossCapturePaths(marker.getAbsolutePath(), log.getAbsolutePath());
            Zeus.bossCaptureReset();
            check("bossCaptureOn is true when marker present", Zeus.bossCaptureOn);

            // 1. Local Menu Event (title is neutral, classified by caption -> MATCH_SPAWNED)
            mVector localItems = new mVector();
            localItems.addElement(new iCommand("Dơi quỷ - Vực đá - Khu 2 : đang xuất hiện", 1));
            localItems.addElement(new iCommand("Đóng", 2));

            long tLocal = 1791428400000L;
            Zeus.observeBossMenu("LOCAL", localItems, -1, -1, "Bảng thông báo", tLocal);

            check("Log file created after local event", log.exists());

            // 2. Server Menu Event (title has Alisama -> MATCH_TITLE)
            mVector serverItems = new mVector();
            serverItems.addElement(new iCommand("Thằn lằn - Khu 1: đã xuất hiện", 0));
            long tServer = 1791428410000L;
            Zeus.observeBossMenu("SERVER", serverItems, 7, 99, "NPC Alisama", tServer);

            // Read log content
            byte[] bytes = new byte[(int) log.length()];
            FileInputStream fis = new FileInputStream(log);
            fis.read(bytes);
            fis.close();
            String logContent = new String(bytes, "UTF-8");

            check("Log contains LOCAL origin event", logContent.indexOf("\"origin\":\"LOCAL\"") >= 0);
            check("LOCAL event has idNPC=-1", logContent.indexOf("\"idNPC\":-1") >= 0);
            check("Log contains SERVER origin event", logContent.indexOf("\"origin\":\"SERVER\"") >= 0);
            check("SERVER event has idNPC=99", logContent.indexOf("\"idNPC\":99") >= 0);
            check("SERVER event has idMenu=7", logContent.indexOf("\"idMenu\":7") >= 0);
            check("Log contains reason MATCH_SPAWNED", logContent.indexOf("\"reason\":\"MATCH_SPAWNED\"") >= 0);
            check("Log contains reason MATCH_TITLE", logContent.indexOf("\"reason\":\"MATCH_TITLE\"") >= 0);

            // OBS-02: Check distinct mapId and zone fields present in JSON
            check("OBS-02: Log contains mapId field", logContent.indexOf("\"mapId\":") >= 0);
            check("OBS-02: Log contains zone field", logContent.indexOf("\"zone\":") >= 0);
        } catch (Exception e) {
            e.printStackTrace();
            check("Active capture exception: " + e.getMessage(), false);
        } finally {
            if (marker.exists()) marker.delete();
            if (log.exists()) log.delete();
        }
    }

    /**
     * Deduplication within 5-second window.
     */
    private static void testDeduplication() {
        System.out.println("\n--- testDeduplication ---");

        File marker = new File("test_dedup.on");
        File log = new File("test_dedup.txt");
        try {
            if (!marker.exists()) marker.createNewFile();
            if (log.exists()) log.delete();

            Zeus.setBossCapturePaths(marker.getAbsolutePath(), log.getAbsolutePath());
            Zeus.bossCaptureReset();

            mVector items = new mVector();
            items.addElement(new iCommand("Dơi quỷ 14:30:00 Núi đá Khu 2", 0));

            long t0 = 1791428400000L;
            Zeus.observeBossMenu("LOCAL", items, -1, -1, "Lịch Boss", t0);

            long len1 = log.length();
            check("First event written", len1 > 0);

            // Repeat identical event at t0 + 1000ms (within 5000ms window)
            Zeus.observeBossMenu("LOCAL", items, -1, -1, "Lịch Boss", t0 + 1000L);
            long len2 = log.length();
            checkEquals("Duplicate event within 5s suppressed (file length unchanged)", len1, len2);

            // Repeat after 6000ms (window expired)
            Zeus.observeBossMenu("LOCAL", items, -1, -1, "Lịch Boss", t0 + 6000L);
            long len3 = log.length();
            check("Event after 5s window written", len3 > len2);
        } catch (Exception e) {
            e.printStackTrace();
            check("Dedup exception: " + e.getMessage(), false);
        } finally {
            if (marker.exists()) marker.delete();
            if (log.exists()) log.delete();
        }
    }

    /**
     * Bounded byte limits (256 KiB file cap, pre-write check) and UTF-8 preservation.
     */
    private static void testBoundedByteLimitsAndMultibyte() {
        System.out.println("\n--- testBoundedByteLimitsAndMultibyte ---");

        File marker = new File("test_bounds.on");
        File log = new File("test_bounds.txt");
        try {
            if (!marker.exists()) marker.createNewFile();
            if (log.exists()) log.delete();

            Zeus.setBossCapturePaths(marker.getAbsolutePath(), log.getAbsolutePath());
            Zeus.bossCaptureReset();

            // Create pre-existing file of exactly 256 KiB
            FileOutputStream fos = new FileOutputStream(log);
            byte[] fill = new byte[Zeus.BOSS_CAPTURE_MAX_BYTES];
            fos.write(fill);
            fos.close();

            checkEquals("Pre-existing log at 256 KiB cap", (long) Zeus.BOSS_CAPTURE_MAX_BYTES, log.length());

            mVector items = new mVector();
            items.addElement(new iCommand("Dơi quỷ 14:30:00 Núi đá Khu 2", 0));
            Zeus.observeBossMenu("LOCAL", items, -1, -1, "Thông tin Boss", 1791428400000L);

            checkEquals("File cap strictly respected (not appended beyond 256 KiB)", (long) Zeus.BOSS_CAPTURE_MAX_BYTES, log.length());

            // Multibyte string cleaning
            String rawLong = "Dơi quỷ - Núi đá rất dài vượt quá độ dài bình thường của một dòng menu thông báo săn boss trong game KnightOnline v403";
            String cleaned = Zeus.cleanBossString(rawLong, 64);
            check("cleanBossString truncates to max length 64", cleaned.length() <= 64);
            check("Multibyte accents preserved", cleaned.indexOf("Dơi quỷ") >= 0);
        } catch (Exception e) {
            e.printStackTrace();
            check("Bounds exception: " + e.getMessage(), false);
        } finally {
            if (marker.exists()) marker.delete();
            if (log.exists()) log.delete();
        }
    }

    /**
     * R3.2 Real NPC Menu Sequence Regression Tests (Operator Screenshots Grounding).
     */
    private static void testRealNpcScreenshots_R3_2() {
        System.out.println("\n--- testRealNpcScreenshots_R3_2 ---");

        // Screen 2: Generic MENU with exact caption "Thông tin boss" -> MATCH_BOSS_ENTRY
        String[] s2Captions = new String[] {
            "Chiến binh", "Sát thủ", "Pháp sư", "Xạ thủ",
            "Tháo mề đay", "Tháo thú cưỡi", "Thông tin boss"
        };
        String rS2 = Zeus.classifyBossMenu("MENU", s2Captions);
        checkEquals("Screen 2: MENU with 'Thông tin boss' returns exact MATCH_BOSS_ENTRY", "MATCH_BOSS_ENTRY", rS2);

        // Screen 2 Unaccented variant
        String[] s2Unacc = new String[] {
            "Chien binh", "Sat thu", "Phap su", "Thong tin boss"
        };
        String rS2Unacc = Zeus.classifyBossMenu("MENU", s2Unacc);
        checkEquals("Screen 2: MENU with unaccented 'Thong tin boss' returns exact MATCH_BOSS_ENTRY", "MATCH_BOSS_ENTRY", rS2Unacc);

        // Screen 3: Generic MENU with Boss Roster names -> MATCH_BOSS_ROSTER
        String[] s3Captions = new String[] {
            "Dê Bạc", "Dê Vàng", "Xà nữ", "Quỷ đầu bò",
            "Kỵ sĩ địa ngục", "Nhện chúa", "Giant Skeleton"
        };
        String rS3 = Zeus.classifyBossMenu("MENU", s3Captions);
        checkEquals("Screen 3: MENU with 7 boss roster names returns exact MATCH_BOSS_ROSTER", "MATCH_BOSS_ROSTER", rS3);

        // Screen 3 Minimal 2-boss multi-entry evidence
        String[] s3Min = new String[] { "Dê Vàng", "Nhện chúa", "Đóng" };
        String rS3Min = Zeus.classifyBossMenu("MENU", s3Min);
        checkEquals("Screen 3: MENU with 2 boss roster entries returns exact MATCH_BOSS_ROSTER", "MATCH_BOSS_ROSTER", rS3Min);

        // Screen 4: Generic MENU with Boss Schedule countdown rows -> MATCH_BOSS_SCHEDULE
        String[] s4Captions = new String[] {
            "Dê Vàng: 02:20:31. Rừng cao nguyên K 1",
            "Dê Vàng: 02:42:42. Rừng cao nguyên K 2",
            "Dê Vàng: 03:02:25. Rừng cao nguyên K 3",
            "Dê Vàng: 03:39:01. Rừng cao nguyên K 4",
            "Dê Vàng: 04:26:06. Rừng cao nguyên K 5"
        };
        String rS4 = Zeus.classifyBossMenu("MENU", s4Captions);
        checkEquals("Screen 4: MENU with Dê Vàng countdown rows returns exact MATCH_BOSS_SCHEDULE", "MATCH_BOSS_SCHEDULE", rS4);

        // Screen 4 other new boss countdown row
        String[] s4XaNu = new String[] {
            "Xà nữ: 01:15:00. Đầm lầy Khu 3", "Đóng"
        };
        String rS4XaNu = Zeus.classifyBossMenu("MENU", s4XaNu);
        checkEquals("Screen 4: MENU with Xà nữ countdown returns exact MATCH_BOSS_SCHEDULE", "MATCH_BOSS_SCHEDULE", rS4XaNu);

        // --- Negative Merchant and False-Positive Controls ---
        // 1. Shop prose with generic word 'boss' but not exact entry caption
        String[] negShopProse = new String[] {
            "Cửa hàng bán đồ săn boss hôm nay", "Mua trang bị", "Đóng"
        };
        String rNegShopProse = Zeus.classifyBossMenu("MENU", negShopProse);
        check("Negative: Shop prose containing 'boss' rejected as null", rNegShopProse == null);

        // 2. Single boss token alone in a non-roster shop (lacks multi-entry structural evidence)
        String[] negSingleBoss = new String[] {
            "Dê Vàng", "Mua bình HP", "Đóng"
        };
        String rNegSingleBoss = Zeus.classifyBossMenu("MENU", negSingleBoss);
        check("Negative: Single boss token without multi-entry evidence rejected as null", rNegSingleBoss == null);

        // 3. Shop selling animal materials (sừng Dê Bạc)
        String[] negAnimalMaterial = new String[] {
            "Bán sừng Dê Bạc", "Mua da Sói", "Đóng"
        };
        String rNegAnimal = Zeus.classifyBossMenu("MENU", negAnimalMaterial);
        check("Negative: Animal materials in shop rejected as null", rNegAnimal == null);

        // 4. Sensitive password guard applied to Screen 2 fixture
        String[] s2Sens = new String[] {
            "Chiến binh", "Thông tin boss", "Mật khẩu cấp 2: 123456"
        };
        String rS2Sens = Zeus.classifyBossMenu("MENU", s2Sens);
        check("Negative: Screen 2 with sensitive PIN/password rejected as null", rS2Sens == null);

        // 5. Active Capture Verification for R3.2 Screens
        File marker = new File("test_r3_2_capture.on");
        File log = new File("test_r3_2_capture.txt");
        try {
            if (!marker.exists()) marker.createNewFile();
            if (log.exists()) log.delete();

            Zeus.setBossCapturePaths(marker.getAbsolutePath(), log.getAbsolutePath());
            Zeus.bossCaptureReset();

            mVector v2 = new mVector();
            for (int i = 0; i < s2Captions.length; i++) v2.addElement(new iCommand(s2Captions[i], i));
            Zeus.observeBossMenu("LOCAL", v2, -1, 5, "MENU", 1791428500000L);

            mVector v3 = new mVector();
            for (int i = 0; i < s3Captions.length; i++) v3.addElement(new iCommand(s3Captions[i], i));
            Zeus.observeBossMenu("LOCAL", v3, -1, 5, "MENU", 1791428506000L);

            mVector v4 = new mVector();
            for (int i = 0; i < s4Captions.length; i++) v4.addElement(new iCommand(s4Captions[i], i));
            Zeus.observeBossMenu("LOCAL", v4, -1, 5, "MENU", 1791428512000L);

            byte[] b = new byte[(int) log.length()];
            FileInputStream fis = new FileInputStream(log);
            fis.read(b);
            fis.close();
            String c = new String(b, "UTF-8");

            check("Capture log contains MATCH_BOSS_ENTRY", c.indexOf("\"reason\":\"MATCH_BOSS_ENTRY\"") >= 0);
            check("Capture log contains MATCH_BOSS_ROSTER", c.indexOf("\"reason\":\"MATCH_BOSS_ROSTER\"") >= 0);
            check("Capture log contains MATCH_BOSS_SCHEDULE", c.indexOf("\"reason\":\"MATCH_BOSS_SCHEDULE\"") >= 0);
        } catch (Exception e) {
            e.printStackTrace();
            check("R3.2 capture exception: " + e.getMessage(), false);
        } finally {
            if (marker.exists()) marker.delete();
            if (log.exists()) log.delete();
        }
    }

    /**
     * Zero mutations to menu objects or return values.
     */
    private static void testMenuStateAndObjectsUnmutated() {
        System.out.println("\n--- testMenuStateAndObjectsUnmutated ---");

        File marker = new File("test_nomutate.on");
        File log = new File("test_nomutate.txt");
        try {
            if (!marker.exists()) marker.createNewFile();
            if (log.exists()) log.delete();

            Zeus.setBossCapturePaths(marker.getAbsolutePath(), log.getAbsolutePath());
            Zeus.bossCaptureReset();

            mVector items = new mVector();
            iCommand cmd1 = new iCommand("Dơi quỷ - Vực đá - Khu 2 : đang xuất hiện", 10);
            iCommand cmd2 = new iCommand("Đóng", 20);
            items.addElement(cmd1);
            items.addElement(cmd2);

            String origCap1 = cmd1.caption;
            byte origIdx1 = cmd1.indexMenu;
            int origSize = items.size();

            boolean taken = Zeus.menu(items, "Thông tin Boss");
            check("Zeus.menu returns taken=false", !taken);

            checkEquals("mVector size unmutated", origSize, items.size());
            checkEquals("cmd1.caption unmutated", origCap1, cmd1.caption);
            checkEquals("cmd1.indexMenu unmutated", origIdx1, cmd1.indexMenu);
            check("cmd1 reference unchanged", items.elementAt(0) == cmd1);
            check("cmd2 reference unchanged", items.elementAt(1) == cmd2);
        } catch (Exception e) {
            e.printStackTrace();
            check("Mutation test exception: " + e.getMessage(), false);
        } finally {
            if (marker.exists()) marker.delete();
            if (log.exists()) log.delete();
        }
    }
}
