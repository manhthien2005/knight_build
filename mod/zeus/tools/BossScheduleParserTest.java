import java.util.Locale;
import java.util.TimeZone;

/**
 * Deterministic, standalone test suite for BossScheduleParser.
 * Java 6 compatible, pure Java with zero runtime dependencies.
 *
 * NOTE: All fixtures in this suite are either:
 * 1) VERIFIED BYTECODE PATTERNS extracted from ModByTminh v4.0.3 javap analysis, OR
 * 2) SYNTHETIC TEST FIXTURES designed to test edge cases, timezone independence, and safety bounds.
 * (No live NPC responses were captured in R1).
 */
public class BossScheduleParserTest {

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
        boolean ok = (expected == null && actual == null) || (expected != null && expected.equals(actual));
        if (ok) {
            System.out.println("PASS: " + label + " (" + actual + ")");
        } else {
            System.out.println("FAIL: " + label + " -> Expected [" + expected + "], got [" + actual + "]");
            failures++;
        }
    }

    public static void main(String[] args) {
        System.out.println("=== BossScheduleParserTest Suite Starting ===");

        testAbsoluteClockMode_Gmt7Independence();
        testMidnightRollover_Boundaries();
        testRelativeCountdownMode_Boundaries();
        testStatus_SpawnedActive_BytecodePatterns();
        testStatus_Unavailable_BytecodePatterns();
        testRejection_MalformedTimeAndRanges();
        testRejection_BossMismatch();
        testRejection_ZoneMismatch();
        testRejection_ConflictingMultipleTimeTokens();
        testRejection_UnknownClockMode();
        testRejection_NullAndEmptyInputs();
        testDiacriticsAndFormattingVariations();

        testB05_CollisionIdentitiesAndFalseMatches();
        testB06_EpochArithmeticAndOverflowGuards();
        testTimezoneProbes();

        testB07_To_B11_AdversarialHarden();
        testB12_And_B13_BoundaryHardening();

        System.out.println("\n=== Test Summary: " + (total - failures) + "/" + total + " passed ===");
        if (failures > 0) {
            System.err.println("TEST SUITE FAILED with " + failures + " failures.");
            System.exit(1);
        } else {
            System.out.println("ALL TESTS PASSED.");
        }
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Validates that absolute clock mode always uses GMT+07:00 regardless of the host JVM's default timezone.
     */
    private static void testAbsoluteClockMode_Gmt7Independence() {
        System.out.println("\n--- testAbsoluteClockMode_Gmt7Independence ---");
        // 2026-10-08 10:00:00 GMT+07:00 = 2026-10-08 03:00:00 UTC = 1791428400000L
        long now = 1791428400000L;
        // Expected target: 14:30:00 GMT+07:00 on 2026-10-08 = 2026-10-08 07:30:00 UTC = 1791444600000L
        long expectedDue = 1791444600000L;

        String caption = "Dơi quỷ 14:30:00 Đèo băng giá Khu 2";
        TimeZone defaultTz = TimeZone.getDefault();

        try {
            String[] testTzs = new String[] { "GMT", "America/New_York", "Asia/Tokyo", "Europe/London", "UTC", "Pacific/Honolulu" };
            for (int i = 0; i < testTzs.length; i++) {
                TimeZone.setDefault(TimeZone.getTimeZone(testTzs[i]));
                BossScheduleParser.Result r = BossScheduleParser.parse(
                    caption, "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
                );
                check("Host TZ " + testTzs[i] + " parse status SCHEDULED", r.status == BossScheduleParser.Status.SCHEDULED);
                checkEquals("Host TZ " + testTzs[i] + " dueEpochMillis", Long.valueOf(expectedDue), r.dueEpochMillis);
                checkEquals("Host TZ " + testTzs[i] + " mapAlias", "Đèo băng giá", r.mapAlias);
                checkEquals("Host TZ " + testTzs[i] + " expectedBoss", "Dơi quỷ", r.expectedBoss);
                check("Host TZ " + testTzs[i] + " expectedZone", r.expectedZone == 2);
            }
        } finally {
            TimeZone.setDefault(defaultTz);
        }
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Validates exact boundary conditions around the 60-second rollover rule:
     * - due < now - 60000L: rollover to next day (+86400000L), status SCHEDULED
     * - now - 60000L <= due <= now: within past 60s, NO rollover, status SPAWNED_OR_PAST
     * - due > now: future today, NO rollover, status SCHEDULED
     */
    private static void testMidnightRollover_Boundaries() {
        System.out.println("\n--- testMidnightRollover_Boundaries ---");
        // now = 2026-10-08 12:01:00 GMT+07:00 = 1791435660000L
        long now = 1791435660000L;

        // Case A: due = 12:00:00 GMT+07:00 (exact 60 seconds ago = now - 60000L).
        // Since due == now - 60000L, condition (due < now - 60000L) is FALSE.
        // Thus NO rollover, and due <= now -> status SPAWNED_OR_PAST
        BossScheduleParser.Result rA = BossScheduleParser.parse(
            "Dơi quỷ 12:00:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Exact 60s past status is SPAWNED_OR_PAST", rA.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        checkEquals("Exact 60s past dueEpochMillis preserved", Long.valueOf(1791435600000L), rA.dueEpochMillis);

        // Case B: due = 11:59:59 GMT+07:00 (61 seconds ago < now - 60000L).
        // Condition (due < now - 60000L) is TRUE -> rolls over to tomorrow (+86400000L).
        BossScheduleParser.Result rB = BossScheduleParser.parse(
            "Dơi quỷ 11:59:59 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("61s past rolls over to tomorrow (SCHEDULED)", rB.status == BossScheduleParser.Status.SCHEDULED);
        long tomorrowDue = 1791435599000L + 86400000L;
        checkEquals("61s past rolled over dueEpochMillis", Long.valueOf(tomorrowDue), rB.dueEpochMillis);

        // Case C: late night rollover across midnight
        // now = 2026-10-08 23:55:00 GMT+07:00 = 1791478500000L
        long lateNow = 1791478500000L;
        // due = 00:05:00 GMT+07:00 (which was today morning at 00:05:00, almost 24 hours ago)
        // condition (due < now - 60000L) is TRUE -> rolls over to 2026-10-09 00:05:00 GMT+07:00 (+86400000L)
        BossScheduleParser.Result rC = BossScheduleParser.parse(
            "Thằn lằn 00:05:00 Khu 2", "Thằn lằn", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, lateNow
        );
        check("Midnight rollover status SCHEDULED", rC.status == BossScheduleParser.Status.SCHEDULED);
        // 2026-10-08 00:05:00 GMT+07:00 = 1791392700000L + 86400000L = 1791479100000L
        checkEquals("Midnight rollover tomorrow dueEpochMillis", Long.valueOf(1791479100000L), rC.dueEpochMillis);
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Validates relative countdown calculation and bounds.
     */
    private static void testRelativeCountdownMode_Boundaries() {
        System.out.println("\n--- testRelativeCountdownMode_Boundaries ---");
        long now = 1000000000L;

        // Normal countdown: 02:15:30 = 2*3600 + 15*60 + 30 = 8130s = 8,130,000ms
        BossScheduleParser.Result r1 = BossScheduleParser.parse(
            "Sói quỷ 02:15:30 Núi đá k3", "Sói quỷ", 3, BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, now
        );
        check("Countdown status SCHEDULED", r1.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Countdown dueEpochMillis", Long.valueOf(now + 8130000L), r1.dueEpochMillis);
        checkEquals("Countdown mapAlias", "Núi đá", r1.mapAlias);

        // Zero countdown -> SPAWNED_OR_PAST
        BossScheduleParser.Result rZero = BossScheduleParser.parse(
            "Sói quỷ 00:00:00 Núi đá k3", "Sói quỷ", 3, BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, now
        );
        check("Zero countdown status is SPAWNED_OR_PAST", rZero.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        checkEquals("Zero countdown dueEpochMillis equals now", Long.valueOf(now), rZero.dueEpochMillis);

        // Boundary: 99 hours (maximum allowed duration)
        // 99 * 3600 * 1000 = 356,400,000 ms
        BossScheduleParser.Result rMax = BossScheduleParser.parse(
            "Sói quỷ 99:00:00 Núi đá k3", "Sói quỷ", 3, BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, now
        );
        check("99h countdown accepted", rMax.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("99h countdown dueEpochMillis", Long.valueOf(now + 356400000L), rMax.dueEpochMillis);
    }

    /**
     * [VERIFIED BYTECODE PATTERN]
     * Patterns extracted from ModByTminh BossHunt.consume:
     * - "dang xuat hien"
     * - "da xuat hien"
     */
    private static void testStatus_SpawnedActive_BytecodePatterns() {
        System.out.println("\n--- testStatus_SpawnedActive_BytecodePatterns ---");
        long now = 1791428400000L;

        // Pattern: "dang xuat hien"
        String caption1 = "Dơi quỷ - Vực đá - Khu 2 : đang xuất hiện";
        BossScheduleParser.Result r1 = BossScheduleParser.parse(
            caption1, "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("dang xuat hien status SPAWNED_OR_PAST", r1.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        check("dang xuat hien dueEpochMillis is null", r1.dueEpochMillis == null);
        checkEquals("dang xuat hien mapAlias", "Vực đá", r1.mapAlias);

        // Pattern: "da xuat hien"
        String caption2 = "Thằn lằn Khu 1 : đã xuất hiện";
        BossScheduleParser.Result r2 = BossScheduleParser.parse(
            caption2, "Thằn lằn", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("da xuat hien status SPAWNED_OR_PAST", r2.status == BossScheduleParser.Status.SPAWNED_OR_PAST);

        // Mismatched zone on spawned caption fails closed
        BossScheduleParser.Result rMismatch = BossScheduleParser.parse(
            caption1, "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Spawned caption with wrong zone rejected as UNKNOWN", rMismatch.status == BossScheduleParser.Status.UNKNOWN);
    }

    /**
     * [VERIFIED BYTECODE PATTERN]
     * Patterns extracted from ModByTminh BossHunt.consume:
     * - "khong co thong tin"
     * - "khong co lich"
     */
    private static void testStatus_Unavailable_BytecodePatterns() {
        System.out.println("\n--- testStatus_Unavailable_BytecodePatterns ---");
        long now = 1791428400000L;

        // Pattern: "khong co thong tin"
        String caption1 = "Dơi quỷ : không có thông tin";
        BossScheduleParser.Result r1 = BossScheduleParser.parse(
            caption1, "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("khong co thong tin status UNAVAILABLE", r1.status == BossScheduleParser.Status.UNAVAILABLE);
        check("khong co thong tin dueEpochMillis is null", r1.dueEpochMillis == null);

        // Pattern: "khong co lich"
        String caption2 = "Dơi quỷ : không có lịch";
        BossScheduleParser.Result r2 = BossScheduleParser.parse(
            caption2, "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("khong co lich status UNAVAILABLE", r2.status == BossScheduleParser.Status.UNAVAILABLE);

        // Pattern with zone mismatch fails closed
        String caption3 = "Dơi quỷ Khu 3 : không có lịch";
        BossScheduleParser.Result r3 = BossScheduleParser.parse(
            caption3, "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Unavailable with zone mismatch rejected as UNKNOWN", r3.status == BossScheduleParser.Status.UNKNOWN);
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Rejection of invalid time tokens, ranges, and formats.
     */
    private static void testRejection_MalformedTimeAndRanges() {
        System.out.println("\n--- testRejection_MalformedTimeAndRanges ---");
        long now = 1791428400000L;

        // Hour 24 (invalid for absolute 0..23)
        check("Hour 24 rejected",
            BossScheduleParser.parse("Dơi quỷ 24:00:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        // Hour 25
        check("Hour 25 rejected",
            BossScheduleParser.parse("Dơi quỷ 25:00:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        // Minute 60
        check("Minute 60 rejected",
            BossScheduleParser.parse("Dơi quỷ 12:60:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        // Second 60
        check("Second 60 rejected",
            BossScheduleParser.parse("Dơi quỷ 12:00:60 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        // Second 99
        check("Second 99 rejected",
            BossScheduleParser.parse("Dơi quỷ 12:00:99 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        // Partial time token: 12:00
        check("Incomplete time token (HH:mm) rejected",
            BossScheduleParser.parse("Dơi quỷ 12:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        // Non-digit time token: 12:aa:00
        check("Non-digit token rejected",
            BossScheduleParser.parse("Dơi quỷ 12:aa:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Rejection of wrong boss identity.
     */
    private static void testRejection_BossMismatch() {
        System.out.println("\n--- testRejection_BossMismatch ---");
        long now = 1791428400000L;

        check("Different boss name rejected",
            BossScheduleParser.parse("Thằn lằn 14:00:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Rejection of wrong zone or missing zone.
     */
    private static void testRejection_ZoneMismatch() {
        System.out.println("\n--- testRejection_ZoneMismatch ---");
        long now = 1791428400000L;

        // Zone 3 in caption, expected 2
        check("Different zone rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Khu 3", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        // Missing zone in caption
        check("Missing zone rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Vực đá", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Rejection when multiple conflicting time tokens exist.
     */
    private static void testRejection_ConflictingMultipleTimeTokens() {
        System.out.println("\n--- testRejection_ConflictingMultipleTimeTokens ---");
        long now = 1791428400000L;

        check("Two distinct times rejected",
            BossScheduleParser.parse("Dơi quỷ 12:00:00 13:00:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        check("Duplicate times rejected",
            BossScheduleParser.parse("Dơi quỷ 12:00:00 12:00:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Rejection when clock mode is 0 or unsupported.
     */
    private static void testRejection_UnknownClockMode() {
        System.out.println("\n--- testRejection_UnknownClockMode ---");
        long now = 1791428400000L;

        check("Clock mode 0 rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Khu 1", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_UNKNOWN, now).isUnknown());

        check("Clock mode 99 rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Khu 1", "Dơi quỷ", 1, 99, now).isUnknown());
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Rejection of null, empty, or negative inputs.
     */
    private static void testRejection_NullAndEmptyInputs() {
        System.out.println("\n--- testRejection_NullAndEmptyInputs ---");
        long now = 1791428400000L;

        check("Null caption rejected",
            BossScheduleParser.parse(null, "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        check("Empty caption rejected",
            BossScheduleParser.parse("", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        check("Whitespace caption rejected",
            BossScheduleParser.parse("   ", "Dơi quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        check("Null boss rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Khu 1", null, 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        check("Empty boss rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Khu 1", "", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        check("Zone 0 rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Khu 1", "Dơi quỷ", 0, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());

        check("Negative zone rejected",
            BossScheduleParser.parse("Dơi quỷ 14:00:00 Khu 1", "Dơi quỷ", -1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now).isUnknown());
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Tests diacritics normalization and flexible punctuation around map names.
     */
    private static void testDiacriticsAndFormattingVariations() {
        System.out.println("\n--- testDiacriticsAndFormattingVariations ---");
        long now = 1791428400000L;
        long expectedDue = 1791444600000L;

        // Unaccented boss input matching accented caption
        BossScheduleParser.Result r1 = BossScheduleParser.parse(
            "Doi quy 14:30:00 Deo bang gia k2", "dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Unaccented match status SCHEDULED", r1.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Unaccented dueEpochMillis", Long.valueOf(expectedDue), r1.dueEpochMillis);
        checkEquals("Unaccented mapAlias", "Deo bang gia", r1.mapAlias);

        // Trailing delimiter stripped from map
        BossScheduleParser.Result r2 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 - Núi lửa : Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Punctuation in map status SCHEDULED", r2.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Punctuation stripped mapAlias", "Núi lửa", r2.mapAlias);
    }

    /**
     * [SYNTHETIC TEST FIXTURE - B-05]
     * Validates that substring matching is strictly prevented.
     * Expected "Sói" must NOT match "Sói quỷ"; expected "Dê" must NOT match "Dê bạc".
     * Mismatched substrings later or earlier in row must not cause false matches.
     */
    private static void testB05_CollisionIdentitiesAndFalseMatches() {
        System.out.println("\n--- testB05_CollisionIdentitiesAndFalseMatches ---");
        long now = 1791428400000L;

        // 1. "Sói" vs "Sói quỷ" scheduled row
        BossScheduleParser.Result r1 = BossScheduleParser.parse(
            "Sói quỷ 14:30:00 Núi đá Khu 1", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' rejected against 'Sói quỷ' scheduled", r1.isUnknown());

        // 2. "Sói quỷ" vs "Sói" scheduled row
        BossScheduleParser.Result r2 = BossScheduleParser.parse(
            "Sói 14:30:00 Núi đá Khu 1", "Sói quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói quỷ' rejected against 'Sói' scheduled", r2.isUnknown());

        // 3. "Dê" vs "Dê bạc" scheduled row
        BossScheduleParser.Result r3 = BossScheduleParser.parse(
            "Dê bạc 14:30:00 Núi đá Khu 1", "Dê", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Dê' rejected against 'Dê bạc' scheduled", r3.isUnknown());

        // 4. "Dê bạc" vs "Dê" scheduled row
        BossScheduleParser.Result r4 = BossScheduleParser.parse(
            "Dê 14:30:00 Núi đá Khu 1", "Dê bạc", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Dê bạc' rejected against 'Dê' scheduled", r4.isUnknown());

        // 5. "Dơi" vs "Dơi quỷ" scheduled row
        BossScheduleParser.Result r5 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2", "Dơi", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Dơi' rejected against 'Dơi quỷ' scheduled", r5.isUnknown());

        // 6. "Sói" vs "Sói quỷ" in spawned row
        BossScheduleParser.Result r6 = BossScheduleParser.parse(
            "Sói quỷ - Vực đá - Khu 1 : đang xuất hiện", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' rejected against 'Sói quỷ' spawned", r6.isUnknown());

        // 7. "Sói" vs "Sói quỷ" in unavailable row
        BossScheduleParser.Result r7 = BossScheduleParser.parse(
            "Sói quỷ : không có lịch", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' rejected against 'Sói quỷ' unavailable with colon", r7.isUnknown());

        // 8. "Sói" vs "Sói quỷ" in unavailable row with zone
        BossScheduleParser.Result r8 = BossScheduleParser.parse(
            "Sói quỷ Khu 1 : không có thông tin", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' rejected against 'Sói quỷ' unavailable with zone", r8.isUnknown());

        // 9. Misleading substring embedded in map name: "Sói" expected, row is "Sói quỷ 14:30:00 Đồi Sói Khu 1"
        BossScheduleParser.Result r9 = BossScheduleParser.parse(
            "Sói quỷ 14:30:00 Đồi Sói Khu 1", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' rejected when 'Sói' appears in map of 'Sói quỷ' row", r9.isUnknown());

        // 10. Misleading prefix: "Sói" expected, row is "Thợ săn Sói 14:30:00 Khu 1"
        BossScheduleParser.Result r10 = BossScheduleParser.parse(
            "Thợ săn Sói 14:30:00 Khu 1", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' rejected when preceded by other text", r10.isUnknown());

        // 11. Positive match: exact "Sói" scheduled row
        BossScheduleParser.Result r11 = BossScheduleParser.parse(
            "Sói 14:30:00 Núi đá Khu 1", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' accepted for exact 'Sói' scheduled", r11.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Sói scheduled mapAlias", "Núi đá", r11.mapAlias);

        // 12. Positive match: exact "Sói quỷ" scheduled row
        BossScheduleParser.Result r12 = BossScheduleParser.parse(
            "Sói quỷ 14:30:00 Núi đá Khu 1", "Sói quỷ", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói quỷ' accepted for exact 'Sói quỷ' scheduled", r12.status == BossScheduleParser.Status.SCHEDULED);

        // 13. Positive match: exact "Dê" scheduled row
        BossScheduleParser.Result r13 = BossScheduleParser.parse(
            "Dê 14:30:00 Núi đá Khu 1", "Dê", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Dê' accepted for exact 'Dê' scheduled", r13.status == BossScheduleParser.Status.SCHEDULED);

        // 14. Positive match: exact "Dê bạc" scheduled row
        BossScheduleParser.Result r14 = BossScheduleParser.parse(
            "Dê bạc 14:30:00 Núi đá Khu 1", "Dê bạc", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Dê bạc' accepted for exact 'Dê bạc' scheduled", r14.status == BossScheduleParser.Status.SCHEDULED);

        // 15. Positive match: exact "Sói" in spawned row
        BossScheduleParser.Result r15 = BossScheduleParser.parse(
            "Sói - Vực đá - Khu 1 : đang xuất hiện", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' accepted for exact 'Sói' spawned", r15.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        checkEquals("Sói spawned mapAlias", "Vực đá", r15.mapAlias);

        // 16. Positive match: exact "Sói" in unavailable row
        BossScheduleParser.Result r16 = BossScheduleParser.parse(
            "Sói : không có lịch", "Sói", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Expected 'Sói' accepted for exact 'Sói' unavailable", r16.status == BossScheduleParser.Status.UNAVAILABLE);
    }

    /**
     * [SYNTHETIC TEST FIXTURE - B-06]
     * Validates epoch arithmetic guards against overflow, underflow, and wrapped timestamps.
     * All out-of-bounds or invalid timestamps must return UNKNOWN, never wrapped/negative SCHEDULED due times.
     */
    private static void testB06_EpochArithmeticAndOverflowGuards() {
        System.out.println("\n--- testB06_EpochArithmeticAndOverflowGuards ---");

        // 1. Countdown mode: Long.MAX_VALUE nowEpochMillis
        BossScheduleParser.Result r1 = BossScheduleParser.parse(
            "Dơi quỷ 01:00:00 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, Long.MAX_VALUE
        );
        check("Countdown with Long.MAX_VALUE rejected as UNKNOWN", r1.isUnknown());

        // 2. Countdown mode: near Long.MAX_VALUE that would overflow
        BossScheduleParser.Result r2 = BossScheduleParser.parse(
            "Dơi quỷ 00:00:10 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, Long.MAX_VALUE - 5000L
        );
        check("Countdown overflow near Long.MAX_VALUE rejected as UNKNOWN", r2.isUnknown());

        // 3. Countdown mode: negative nowEpochMillis
        BossScheduleParser.Result r3 = BossScheduleParser.parse(
            "Dơi quỷ 00:00:10 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, -1L
        );
        check("Countdown with negative now rejected as UNKNOWN", r3.isUnknown());

        // 4. Countdown mode: Long.MIN_VALUE nowEpochMillis
        BossScheduleParser.Result r4 = BossScheduleParser.parse(
            "Dơi quỷ 00:00:10 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, Long.MIN_VALUE
        );
        check("Countdown with Long.MIN_VALUE rejected as UNKNOWN", r4.isUnknown());

        // 5. Absolute mode: Long.MAX_VALUE nowEpochMillis
        BossScheduleParser.Result r5 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, Long.MAX_VALUE
        );
        check("Absolute mode with Long.MAX_VALUE rejected as UNKNOWN", r5.isUnknown());

        // 6. Absolute mode: Long.MIN_VALUE nowEpochMillis
        BossScheduleParser.Result r6 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, Long.MIN_VALUE
        );
        check("Absolute mode with Long.MIN_VALUE rejected as UNKNOWN", r6.isUnknown());

        // 7. Absolute mode: negative nowEpochMillis
        BossScheduleParser.Result r7 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, -1000L
        );
        check("Absolute mode with negative now rejected as UNKNOWN", r7.isUnknown());

        // 8. Absolute mode: near Long.MAX_VALUE rollover boundary
        BossScheduleParser.Result r8 = BossScheduleParser.parse(
            "Dơi quỷ 00:01:00 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, Long.MAX_VALUE - 100000L
        );
        check("Absolute mode rollover near Long.MAX_VALUE rejected as UNKNOWN", r8.isUnknown());

        // 9. Valid boundary: Countdown exactly at zero
        long validNow = 1791428400000L;
        BossScheduleParser.Result r9 = BossScheduleParser.parse(
            "Dơi quỷ 00:00:00 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, validNow
        );
        check("Valid zero countdown accepted as SPAWNED_OR_PAST", r9.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        checkEquals("Valid zero countdown due equals now", Long.valueOf(validNow), r9.dueEpochMillis);

        // 10. Valid boundary: Countdown with normal duration
        BossScheduleParser.Result r10 = BossScheduleParser.parse(
            "Dơi quỷ 00:01:00 Núi đá Khu 1", "Dơi quỷ", 1,
            BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, validNow
        );
        check("Valid countdown accepted as SCHEDULED", r10.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Valid countdown due is now + 60000ms", Long.valueOf(validNow + 60000L), r10.dueEpochMillis);
    }

    /**
     * [SYNTHETIC TEST FIXTURE]
     * Validates timezone-independent behavior across GMT, America/New_York, Asia/Tokyo, and Asia/Ho_Chi_Minh.
     */
    private static void testTimezoneProbes() {
        System.out.println("\n--- testTimezoneProbes ---");
        long now = 1791428400000L; // 2026-10-08 10:00:00 GMT+7
        long expectedDue = 1791444600000L; // 14:30:00 GMT+7
        String caption = "Thằn lằn 14:30:00 Khu 1";

        String[] tzs = new String[] { "GMT", "America/New_York", "Asia/Tokyo", "Asia/Ho_Chi_Minh" };
        TimeZone defaultTz = TimeZone.getDefault();
        try {
            for (int i = 0; i < tzs.length; i++) {
                TimeZone.setDefault(TimeZone.getTimeZone(tzs[i]));
                BossScheduleParser.Result r = BossScheduleParser.parse(
                    caption, "Thằn lằn", 1, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
                );
                check("Explicit probe " + tzs[i] + " SCHEDULED", r.status == BossScheduleParser.Status.SCHEDULED);
                checkEquals("Explicit probe " + tzs[i] + " dueEpochMillis", Long.valueOf(expectedDue), r.dueEpochMillis);
            }
        } finally {
            TimeZone.setDefault(defaultTz);
        }
    }

    /**
     * [SYNTHETIC TEST FIXTURES - B-07 through B-11]
     * Validates hardening against:
     * - B-07: 3-digit hour token boundary violation
     * - B-08: 3-digit second / adjacent character token boundary violation
     * - B-09: Ambiguous multiple zone tokens (identical or conflicting)
     * - B-10: Zone numeric prefix followed by trailing alphanumeric characters
     * - B-11: JVM default locale dependency (e.g. Turkish dotted/dotless I)
     */
    private static void testB07_To_B11_AdversarialHarden() {
        System.out.println("\n--- testB07_To_B11_AdversarialHarden ---");
        long now = 1791428400000L;
        long expectedDue = 1791444600000L;

        // B-07: 3-digit hour
        BossScheduleParser.Result rB07_1 = BossScheduleParser.parse(
            "Dơi quỷ: 123:45:00 Đèo băng giá Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-07: 3-digit hour with colon prefix rejected as UNKNOWN", rB07_1.isUnknown());

        BossScheduleParser.Result rB07_2 = BossScheduleParser.parse(
            "Dơi quỷ 123:45:00 Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-07: 3-digit hour standard rejected as UNKNOWN", rB07_2.isUnknown());

        BossScheduleParser.Result rB07_3 = BossScheduleParser.parse(
            "Dơi quỷ 12:345:00 Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-07: 3-digit minute rejected as UNKNOWN", rB07_3.isUnknown());

        // B-08: 3-digit second / adjacent boundary
        BossScheduleParser.Result rB08_1 = BossScheduleParser.parse(
            "Dơi quỷ - 12:45:006 Đèo băng giá Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-08: 3-digit second rejected as UNKNOWN", rB08_1.isUnknown());

        BossScheduleParser.Result rB08_2 = BossScheduleParser.parse(
            "Dơi quỷ 12:45:006 Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-08: 3-digit second without hyphen rejected as UNKNOWN", rB08_2.isUnknown());

        BossScheduleParser.Result rB08_3 = BossScheduleParser.parse(
            "Dơi quỷ a12:45:00 Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-08: Leading letter before time rejected as UNKNOWN", rB08_3.isUnknown());

        BossScheduleParser.Result rB08_4 = BossScheduleParser.parse(
            "Dơi quỷ 12:45:00z Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-08: Trailing letter after time rejected as UNKNOWN", rB08_4.isUnknown());

        // Positive control: proper HH:mm:ss
        BossScheduleParser.Result rTimePos1 = BossScheduleParser.parse(
            "Dơi quỷ: 14:30:00 Đèo băng giá Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Positive Control: HH:mm:ss with colon prefix accepted", rTimePos1.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Time pos 1 dueEpochMillis", Long.valueOf(expectedDue), rTimePos1.dueEpochMillis);

        BossScheduleParser.Result rTimePos2 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Positive Control: HH:mm:ss standard accepted", rTimePos2.status == BossScheduleParser.Status.SCHEDULED);

        // B-09: Multiple zone tokens
        BossScheduleParser.Result rB09_1 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2 Khu 3", "Dơi quỷ", 3, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-09: Multiple conflicting zones (target 3) rejected as UNKNOWN", rB09_1.isUnknown());

        BossScheduleParser.Result rB09_2 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2 Khu 3", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-09: Multiple conflicting zones (target 2) rejected as UNKNOWN", rB09_2.isUnknown());

        BossScheduleParser.Result rB09_3 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2 Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-09: Duplicate identical zones rejected as UNKNOWN", rB09_3.isUnknown());

        BossScheduleParser.Result rB09_4 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá k2 k2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-09: Duplicate identical k-zones rejected as UNKNOWN", rB09_4.isUnknown());

        // B-10: Zone number with trailing letters
        BossScheduleParser.Result rB10_1 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2xyz", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-10: Zone Khu 2xyz rejected as UNKNOWN", rB10_1.isUnknown());

        BossScheduleParser.Result rB10_2 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá k2xyz", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-10: Zone k2xyz rejected as UNKNOWN", rB10_2.isUnknown());

        BossScheduleParser.Result rB10_3 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2a", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-10: Zone Khu 2a rejected as UNKNOWN", rB10_3.isUnknown());

        // Positive control: proper zones
        BossScheduleParser.Result rZonePos1 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Positive Control: Khu 2 accepted", rZonePos1.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Zone pos 1 mapAlias", "Đèo băng giá", rZonePos1.mapAlias);

        BossScheduleParser.Result rZonePos2 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Đèo băng giá k2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("Positive Control: k2 accepted", rZonePos2.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Zone pos 2 mapAlias", "Đèo băng giá", rZonePos2.mapAlias);

        // B-11: JVM default locale dependency (Turkish dotted/dotless I)
        Locale oldLocale = Locale.getDefault();
        try {
            Locale.setDefault(Locale.US);
            BossScheduleParser.Result rUs = BossScheduleParser.parse(
                "GIANT Skeleton 14:30:00 Núi đá Khu 2", "Giant Skeleton", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
            );
            check("B-11: Locale.US accepts uppercase GIANT Skeleton", rUs.status == BossScheduleParser.Status.SCHEDULED);
            checkEquals("Locale.US mapAlias", "Núi đá", rUs.mapAlias);

            Locale.setDefault(new Locale("tr", "TR"));
            BossScheduleParser.Result rTr = BossScheduleParser.parse(
                "GIANT Skeleton 14:30:00 Núi đá Khu 2", "Giant Skeleton", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
            );
            check("B-11: Turkish locale (tr_TR) accepts uppercase GIANT Skeleton", rTr.status == BossScheduleParser.Status.SCHEDULED);
            checkEquals("Locale tr_TR mapAlias", "Núi đá", rTr.mapAlias);
        } finally {
            Locale.setDefault(oldLocale);
        }
    }

    /**
     * [SYNTHETIC TEST FIXTURES - B-12 and B-13]
     * Validates boundary hardening against:
     * - B-12: Ambiguous numeric/subzone zone suffix (e.g. "Khu 2_3", "Khu 2.3", "Khu 2-3")
     * - B-13: False malformed clock rejection on valid zone colon punctuation (e.g. "Khu 2:", "k2:")
     */
    private static void testB12_And_B13_BoundaryHardening() {
        System.out.println("\n--- testB12_And_B13_BoundaryHardening ---");
        long now = 1791428400000L;
        long expectedDue = 1791444600000L;

        // B-13: Colon following zone digits in spawned captions
        BossScheduleParser.Result rB13_1 = BossScheduleParser.parse(
            "Dơi quỷ - Núi đá - Khu 2: đang xuất hiện", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-13: Spawned with 'Khu 2:' accepted as SPAWNED_OR_PAST", rB13_1.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        checkEquals("B-13: Spawned with 'Khu 2:' mapAlias", "Núi đá", rB13_1.mapAlias);

        BossScheduleParser.Result rB13_2 = BossScheduleParser.parse(
            "Dơi quỷ - Núi đá - k2: đang xuất hiện", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-13: Spawned with 'k2:' accepted as SPAWNED_OR_PAST", rB13_2.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        checkEquals("B-13: Spawned with 'k2:' mapAlias", "Núi đá", rB13_2.mapAlias);

        // B-13: Colon following zone digits in unavailable caption
        BossScheduleParser.Result rB13_3 = BossScheduleParser.parse(
            "Dơi quỷ - Núi đá - Khu 2: không có lịch", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-13: Unavailable with 'Khu 2:' accepted as UNAVAILABLE", rB13_3.status == BossScheduleParser.Status.UNAVAILABLE);

        // B-13: Trailing colon after zone in scheduled caption
        BossScheduleParser.Result rB13_4 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 2:", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-13: Scheduled with trailing 'Khu 2:' accepted as SCHEDULED", rB13_4.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("B-13: Scheduled trailing colon dueEpochMillis", Long.valueOf(expectedDue), rB13_4.dueEpochMillis);
        checkEquals("B-13: Scheduled trailing colon mapAlias", "Núi đá", rB13_4.mapAlias);

        // B-12: Ambiguous numeric/subzone suffix rejects as UNKNOWN
        BossScheduleParser.Result rB12_1 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 2_3", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-12: Zone 'Khu 2_3' rejected as UNKNOWN", rB12_1.isUnknown());

        BossScheduleParser.Result rB12_2 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 2.3", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-12: Zone 'Khu 2.3' rejected as UNKNOWN", rB12_2.isUnknown());

        BossScheduleParser.Result rB12_3 = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 2-3", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-12: Zone 'Khu 2-3' rejected as UNKNOWN", rB12_3.isUnknown());

        // Control: clean 'Khu 2' accepted
        BossScheduleParser.Result rControl = BossScheduleParser.parse(
            "Dơi quỷ 14:30:00 Núi đá Khu 2", "Dơi quỷ", 2, BossScheduleParser.CLOCK_MODE_ABSOLUTE_GMT7, now
        );
        check("B-12 Control: Clean 'Khu 2' accepted as SCHEDULED", rControl.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("B-12 Control mapAlias", "Núi đá", rControl.mapAlias);
    }
}
