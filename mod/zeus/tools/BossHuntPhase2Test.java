/**
 * Phase 2 Focused TDD Test Suite: Schedule Integration, Departure Planner & Clock Semantics.
 * Verifies BH-04, BH-05 domain behavior against ModByTminh v4.0.3 reference.
 */
public class BossHuntPhase2Test {

    private static int total = 0;
    private static int failures = 0;

    private static void check(String label, boolean ok) {
        total++;
        if (ok) {
            System.out.println("PASS: " + label);
        } else {
            failures++;
            System.err.println("FAIL: " + label);
        }
    }

    private static void checkEquals(String label, Object actual, Object expected) {
        total++;
        boolean ok = (expected == null && actual == null) || (expected != null && expected.equals(actual));
        if (ok) {
            System.out.println("PASS: " + label + " (" + actual + ")");
        } else {
            failures++;
            System.err.println("FAIL: " + label + " - Expected: [" + expected + "], Got: [" + actual + "]");
        }
    }

    public static void main(String[] args) {
        System.out.println("=== BossHuntPhase2Test Suite Starting ===");

        testScheduleRowIngestion();
        testScheduleReportInMemoryIngestion();
        testMidnightRolloverGMT7();
        testRelativeCountdownMode();
        testDepartureDecisionAndBudget();
        testNextEntrySelectionAndFiltering();

        System.out.println("\n=== Test Summary: " + (total - failures) + "/" + total + " passed ===");
        if (failures > 0) {
            System.err.println("FAILED: " + failures + " test(s) failed.");
            System.exit(1);
        } else {
            System.out.println("ALL TESTS PASSED.");
        }
    }

    private static void testScheduleRowIngestion() {
        System.out.println("\n--- testScheduleRowIngestion ---");
        BossHuntConfig config = new BossHuntConfig();
        BossHuntScheduler scheduler = new BossHuntScheduler(config);

        // Simulated time: 2026-10-11 10:00:00 GMT+7 (1791514800000 ms)
        long now = 1791514800000L;

        // 1. Scheduled row
        // "Dê Vàng: 10:30:00. Rừng cao nguyên K 1"
        boolean ok = scheduler.ingestRow("D\u00ea V\u00e0ng: 10:30:00. R\u1eebng cao nguy\u00ean K 1", now);
        check("Scheduled row ingested", ok);
        BossHuntScheduler.ScheduleEntry e = scheduler.getEntry(1, 1);
        check("Entry exists for boss 1 zone 1", e != null);
        checkEquals("Boss ID is 1", e.bossId, 1);
        checkEquals("Zone is 1", e.zone, 1);
        checkEquals("Map alias is Rừng cao nguyên", e.mapAlias, "R\u1eebng cao nguy\u00ean");
        check("Status is SCHEDULED", e.status == BossScheduleParser.Status.SCHEDULED);
        checkEquals("Due epoch is 10:30:00 GMT+7", e.dueEpochMillis, now + 30 * 60 * 1000L);

        // 2. Spawned row ("đang xuất hiện") -> dueEpochMillis = 0L
        ok = scheduler.ingestRow("D\u00ea V\u00e0ng - Khu 2: \u0111ang xu\u1ea5t hi\u1ec7n", now);
        check("Spawned row ingested", ok);
        BossHuntScheduler.ScheduleEntry e2 = scheduler.getEntry(1, 2);
        check("Entry exists for boss 1 zone 2", e2 != null);
        check("Status is SPAWNED_OR_PAST", e2.status == BossScheduleParser.Status.SPAWNED_OR_PAST);
        checkEquals("Due epoch for active boss is 0L", e2.dueEpochMillis, 0L);

        // 3. Unavailable row ("không có thông tin")
        ok = scheduler.ingestRow("D\u00ea V\u00e0ng - Khu 3: kh\u00f4ng c\u00f3 th\u00f4ng tin", now);
        check("Unavailable row ingested", ok);
        BossHuntScheduler.ScheduleEntry e3 = scheduler.getEntry(1, 3);
        check("Entry exists for boss 1 zone 3", e3 != null);
        check("Status is UNAVAILABLE", e3.status == BossScheduleParser.Status.UNAVAILABLE);
        checkEquals("Due epoch is -1L", e3.dueEpochMillis, -1L);
    }

    private static void testScheduleReportInMemoryIngestion() {
        System.out.println("\n--- testScheduleReportInMemoryIngestion ---");
        BossHuntConfig config = new BossHuntConfig();
        // Disable zone 4 for Dê Vàng (boss 1)
        config.setZoneSelected(1, 4, false);

        BossHuntScheduler scheduler = new BossHuntScheduler(config);
        long now = 1791514800000L;

        // Create in-memory ScheduleReport with 5 rows (zones 1..5)
        BossScheduleReader.Entry[] entries = new BossScheduleReader.Entry[] {
            new BossScheduleReader.Entry("D\u00ea V\u00e0ng", "R\u1eebng cao nguy\u00ean", 1, "10:15:00", "SCHEDULED", "UNKNOWN", "none", false, null, "D\u00ea V\u00e0ng: 10:15:00. R\u1eebng cao nguy\u00ean K 1"),
            new BossScheduleReader.Entry("D\u00ea V\u00e0ng", "R\u1eebng cao nguy\u00ean", 2, "10:20:00", "SCHEDULED", "UNKNOWN", "none", false, null, "D\u00ea V\u00e0ng: 10:20:00. R\u1eebng cao nguy\u00ean K 2"),
            new BossScheduleReader.Entry("D\u00ea V\u00e0ng", "R\u1eebng cao nguy\u00ean", 3, "10:25:00", "SCHEDULED", "UNKNOWN", "none", false, null, "D\u00ea V\u00e0ng: 10:25:00. R\u1eebng cao nguy\u00ean K 3"),
            new BossScheduleReader.Entry("D\u00ea V\u00e0ng", "R\u1eebng cao nguy\u00ean", 4, "10:30:00", "SCHEDULED", "UNKNOWN", "none", false, null, "D\u00ea V\u00e0ng: 10:30:00. R\u1eebng cao nguy\u00ean K 4"),
            new BossScheduleReader.Entry("D\u00ea V\u00e0ng", "R\u1eebng cao nguy\u00ean", 5, "10:35:00", "SCHEDULED", "UNKNOWN", "none", false, null, "D\u00ea V\u00e0ng: 10:35:00. R\u1eebng cao nguy\u00ean K 5")
        };
        BossScheduleReader.ScheduleReport report = new BossScheduleReader.ScheduleReport(now, "SERVER", 100, -22, "D\u00ea V\u00e0ng", entries);

        scheduler.ingestReport(report, now);

        // Zones 1, 2, 3, 5 should be accepted
        check("Zone 1 accepted", scheduler.getEntry(1, 1) != null);
        check("Zone 2 accepted", scheduler.getEntry(1, 2) != null);
        check("Zone 3 accepted", scheduler.getEntry(1, 3) != null);
        check("Zone 5 accepted", scheduler.getEntry(1, 5) != null);

        // Zone 4 was unselected in config, so it must NOT be scheduled
        check("Zone 4 omitted because unselected in config", scheduler.getEntry(1, 4) == null);
    }

    private static void testMidnightRolloverGMT7() {
        System.out.println("\n--- testMidnightRolloverGMT7 ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setClockMode(BossHuntConfig.CLOCK_MODE_MINH_ABSOLUTE_GMT7);
        BossHuntScheduler scheduler = new BossHuntScheduler(config);

        // In GMT+7: 2026-10-11 23:55:00 (1791564900000 ms)
        long now = 1791564900000L;

        // Schedule row for 00:05:00 (10 minutes in the future, across midnight into tomorrow!)
        boolean ok = scheduler.ingestRow("D\u00ea V\u00e0ng: 00:05:00. R\u1eebng cao nguy\u00ean K 1", now);
        check("Midnight crossover row ingested", ok);
        BossHuntScheduler.ScheduleEntry e = scheduler.getEntry(1, 1);
        check("Entry exists", e != null);
        checkEquals("Rolled over to tomorrow (+10 min from now)", e.dueEpochMillis, now + 10 * 60 * 1000L);

        // Schedule row for 23:50:00 (5 minutes in the past -> > 60s in the past -> rolls over to tomorrow!)
        ok = scheduler.ingestRow("D\u00ea V\u00e0ng: 23:50:00. R\u1eebng cao nguy\u00ean K 2", now);
        check("Past row ingested", ok);
        BossHuntScheduler.ScheduleEntry e2 = scheduler.getEntry(1, 2);
        check("Entry exists", e2 != null);
        checkEquals("Rolled over to tomorrow (now - 5 min + 24h)", e2.dueEpochMillis, now - 5 * 60 * 1000L + 86400000L);
    }

    private static void testRelativeCountdownMode() {
        System.out.println("\n--- testRelativeCountdownMode ---");
        BossHuntConfig config = new BossHuntConfig();
        config.setClockMode(BossHuntConfig.CLOCK_MODE_RELATIVE_COUNTDOWN);
        BossHuntScheduler scheduler = new BossHuntScheduler(config);

        long now = 1000000000L;

        // "Dê Vàng: 00:05:00. Rừng cao nguyên K 1" in relative mode = now + 300 seconds
        boolean ok = scheduler.ingestRow("D\u00ea V\u00e0ng: 00:05:00. R\u1eebng cao nguy\u00ean K 1", now);
        check("Relative countdown row ingested", ok);
        BossHuntScheduler.ScheduleEntry e = scheduler.getEntry(1, 1);
        check("Entry exists", e != null);
        checkEquals("Relative countdown due time is now + 300s", e.dueEpochMillis, now + 300 * 1000L);
    }

    private static void testDepartureDecisionAndBudget() {
        System.out.println("\n--- testDepartureDecisionAndBudget ---");
        // ModByTminh departure rule:
        // shouldDepart(due, now, budget)
        // If due == 0L -> true (active boss, depart immediately)
        // If due > 0L -> now >= due - max(30000, budget)
        // If due < 0L -> false

        long now = 1000000000L;
        long travelBudget = 45000L; // 45 seconds

        // Active boss (due = 0)
        check("Active boss departs immediately", BossHuntScheduler.shouldDepart(0L, now, travelBudget));

        // Upcoming boss: due in 60s -> not yet
        check("Boss in 60s does not depart yet", !BossHuntScheduler.shouldDepart(now + 60000L, now, travelBudget));

        // Upcoming boss: due in 45s (exactly travel budget) -> depart!
        check("Boss in 45s departs on budget window", BossHuntScheduler.shouldDepart(now + 45000L, now, travelBudget));

        // Upcoming boss: due in 20s (inside 30s default threshold) -> depart!
        check("Boss in 20s departs on 30s window", BossHuntScheduler.shouldDepart(now + 20000L, now, 10000L));

        // Unavailable boss (due = -1) -> never depart
        check("Unavailable boss does not depart", !BossHuntScheduler.shouldDepart(-1L, now, travelBudget));
    }

    private static void testNextEntrySelectionAndFiltering() {
        System.out.println("\n--- testNextEntrySelectionAndFiltering ---");
        BossHuntConfig config = new BossHuntConfig();
        // Custom map override for boss 0 (Dê Bạc)
        config.setMapOverride(0, "Hang qu\u1ef7");

        BossHuntScheduler scheduler = new BossHuntScheduler(config);
        long now = 1791514800000L;

        // Ingest:
        // Dê Vàng (boss 1) zone 1 at +20 min (10:20)
        // Dê Vàng (boss 1) zone 2 at +10 min (10:10)
        // Dê Bạc (boss 0) zone 1 at +5 min (10:05)
        scheduler.ingestRow("D\u00ea V\u00e0ng: 10:20:00. R\u1eebng cao nguy\u00ean K 1", now);
        scheduler.ingestRow("D\u00ea V\u00e0ng: 10:10:00. R\u1eebng cao nguy\u00ean K 2", now);
        scheduler.ingestRow("D\u00ea B\u1ea1c: 10:05:00. K 1", now);

        BossHuntScheduler.ScheduleEntry next = scheduler.nextEntry(now);
        check("Next entry found", next != null);
        checkEquals("Soonest entry is Dê Bạc (boss 0)", next.bossId, 0);
        checkEquals("Zone is 1", next.zone, 1);
        checkEquals("Effective map uses manual override 'Hang quỷ'", next.mapAlias, "Hang qu\u1ef7");

        // Now disable boss 0 zone 1
        config.setZoneSelected(0, 1, false);
        scheduler.rebuild();

        BossHuntScheduler.ScheduleEntry next2 = scheduler.nextEntry(now);
        check("Next entry after disabling boss 0 zone 1 is boss 1 zone 2 (10:10)", next2 != null);
        checkEquals("Boss ID is 1", next2.bossId, 1);
        checkEquals("Zone is 2", next2.zone, 2);
    }
}
