/**
 * Phase 1 Focused TDD Test Suite: Boss Catalog, Selection, Configuration & State.
 * Verifies BH-01, BH-02, BH-03, BH-10 domain behavior against ModByTminh v4.0.3 reference.
 */
public class BossHuntPhase1Test {

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
        System.out.println("=== BossHuntPhase1Test Suite Starting ===");

        testBossCatalogDefinitions();
        testZoneBitmasks();
        testMapOverrides();
        testConfigDefaultsAndFailClosed();
        testConfigJsonRoundtrip();
        testStateTransitions();

        System.out.println("\n=== Test Summary: " + (total - failures) + "/" + total + " passed ===");
        if (failures > 0) {
            System.err.println("FAILED: " + failures + " test(s) failed.");
            System.exit(1);
        } else {
            System.out.println("ALL TESTS PASSED.");
        }
    }

    private static void testBossCatalogDefinitions() {
        System.out.println("\n--- testBossCatalogDefinitions ---");
        checkEquals("Catalog boss count is 10", BossCatalog.BOSS_COUNT, 10);

        String[] expected = {
            "D\u00ea B\u1ea1c",
            "D\u00ea V\u00e0ng",
            "X\u00e0 n\u1eef",
            "Qu\u1ef7 m\u1ed9t m\u1eaft",
            "Qu\u1ef7 \u0111\u1ea7u b\u00f2",
            "K\u1ef5 s\u0129 \u0111\u1ecba ng\u1ee5c",
            "Nh\u1ec7n ch\u00faa",
            "Giant Skeleton",
            "Th\u1ecf tr\u1eafng",
            "M\u00e8o m\u1eadp"
        };

        for (int i = 0; i < expected.length; i++) {
            checkEquals("Boss ID " + i + " matches ModByTminh name", BossCatalog.getBossName(i), expected[i]);
            checkEquals("Find boss ID for name '" + expected[i] + "'", BossCatalog.findBossId(expected[i]), i);
        }

        // Case-insensitive & normalized search
        checkEquals("Find boss case-insensitive", BossCatalog.findBossId("giant skeleton"), 7);
        checkEquals("Find boss diacritics normalized 'De Vang'", BossCatalog.findBossId("De Vang"), 1);
        checkEquals("Find non-existent boss returns -1", BossCatalog.findBossId("Trum Cuoi"), -1);
        checkEquals("Find null boss returns -1", BossCatalog.findBossId(null), -1);

        // Default map for boss 5 (Ky si dia nguc) is "Dia Nguc Tang 1"
        checkEquals("Boss 5 default map", BossCatalog.getDefaultMap(5), "\u0110\u1ecba Ng\u1ee5c T\u1ea7ng 1");
        checkEquals("Boss 0 default map is null", BossCatalog.getDefaultMap(0), null);
    }

    private static void testZoneBitmasks() {
        System.out.println("\n--- testZoneBitmasks ---");
        BossHuntConfig config = new BossHuntConfig();

        // Default zones are 0x1F (all 1..5 enabled)
        for (int b = 0; b < BossCatalog.BOSS_COUNT; b++) {
            checkEquals("Boss " + b + " initial mask is 0x1F", config.getZoneMask(b), 0x1F);
            for (int z = 1; z <= 5; z++) {
                check("Boss " + b + " zone " + z + " initially selected", config.isZoneSelected(b, z));
            }
            check("Zone 0 rejected", !config.isZoneSelected(b, 0));
            check("Zone 6 rejected", !config.isZoneSelected(b, 6));
        }

        // Toggle zone 3 on boss 1
        config.toggleZone(1, 3);
        check("Boss 1 zone 3 toggled off", !config.isZoneSelected(1, 3));
        checkEquals("Boss 1 mask after toggle", config.getZoneMask(1), 0x1B); // 0x1F & ~(1<<2) = 0x1B (27)

        config.toggleZone(1, 3);
        check("Boss 1 zone 3 toggled back on", config.isZoneSelected(1, 3));
        checkEquals("Boss 1 mask after second toggle", config.getZoneMask(1), 0x1F);

        // Explicit set
        config.setZoneSelected(1, 2, false);
        check("Boss 1 zone 2 explicitly unset", !config.isZoneSelected(1, 2));

        // Mask masking (only lower 5 bits valid)
        config.setZoneMask(2, 0xFF);
        checkEquals("Mask constrained to 0x1F", config.getZoneMask(2), 0x1F);
    }

    private static void testMapOverrides() {
        System.out.println("\n--- testMapOverrides ---");
        BossHuntConfig config = new BossHuntConfig();

        // Default map resolution
        checkEquals("Boss 1 effective map initially schedule-derived",
            config.getEffectiveMap(1, "R\u1eebng cao nguy\u00ean"), "R\u1eebng cao nguy\u00ean");
        checkEquals("Boss 5 effective map defaults to Dia Nguc Tang 1",
            config.getEffectiveMap(5, null), "\u0110\u1ecba Ng\u1ee5c T\u1ea7ng 1");

        // Operator manual override (BH-03)
        config.setMapOverride(1, "V\u1ef1c \u0111\u00e1");
        checkEquals("Boss 1 manual override active", config.getMapOverride(1), "V\u1ef1c \u0111\u00e1");
        checkEquals("Boss 1 effective map uses override",
            config.getEffectiveMap(1, "R\u1eebng cao nguy\u00ean"), "V\u1ef1c \u0111\u00e1");

        // Clear override
        config.setMapOverride(1, null);
        checkEquals("Boss 1 override cleared", config.getMapOverride(1), null);
        checkEquals("Boss 1 effective map falls back to schedule",
            config.getEffectiveMap(1, "R\u1eebng cao nguy\u00ean"), "R\u1eebng cao nguy\u00ean");
    }

    private static void testConfigDefaultsAndFailClosed() {
        System.out.println("\n--- testConfigDefaultsAndFailClosed ---");
        BossHuntConfig config = new BossHuntConfig();

        // FAIL-CLOSED: Boss Hunt is disabled by default
        check("Boss Hunt default is OFF", !config.isEnabled());

        // Default clock mode is ModByTminh GMT+7 compatibility (mode 1)
        checkEquals("Default clock mode is MINH_COMPAT_ABSOLUTE_GMT7 (1)",
            config.getClockMode(), BossHuntConfig.CLOCK_MODE_MINH_ABSOLUTE_GMT7);

        // Setting clock mode
        config.setClockMode(BossHuntConfig.CLOCK_MODE_RELATIVE_COUNTDOWN);
        checkEquals("Clock mode switched to COUNTDOWN (2)",
            config.getClockMode(), BossHuntConfig.CLOCK_MODE_RELATIVE_COUNTDOWN);

        // Invalid clock mode rejected
        config.setClockMode(99);
        checkEquals("Invalid clock mode ignored",
            config.getClockMode(), BossHuntConfig.CLOCK_MODE_RELATIVE_COUNTDOWN);
    }

    private static void testConfigJsonRoundtrip() {
        System.out.println("\n--- testConfigJsonRoundtrip ---");
        BossHuntConfig original = new BossHuntConfig();
        original.setEnabled(true);
        original.setClockMode(BossHuntConfig.CLOCK_MODE_MINH_ABSOLUTE_GMT7);
        original.setZoneMask(0, 0x05); // zones 1 and 3
        original.setZoneMask(5, 0x10); // zone 5
        original.setMapOverride(0, "Hang qu\u1ef7");
        original.setTravelBudget(0, 45000L);

        String json = original.toJson();
        check("JSON output is non-empty", json != null && json.length() > 0);
        check("JSON contains enabled: true", json.indexOf("\"enabled\": true") >= 0);
        check("JSON contains clockMode: 1", json.indexOf("\"clockMode\": 1") >= 0);

        BossHuntConfig restored = BossHuntConfig.fromJson(json);
        check("Restored config is not null", restored != null);
        check("Restored enabled is true", restored.isEnabled());
        checkEquals("Restored clockMode", restored.getClockMode(), BossHuntConfig.CLOCK_MODE_MINH_ABSOLUTE_GMT7);
        checkEquals("Restored boss 0 mask", restored.getZoneMask(0), 0x05);
        checkEquals("Restored boss 5 mask", restored.getZoneMask(5), 0x10);
        checkEquals("Restored boss 0 override", restored.getMapOverride(0), "Hang qu\u1ef7");
        checkEquals("Restored boss 0 travel budget", restored.getTravelBudget(0), 45000L);

        // Malformed JSON parsing returns safe default config (fail-closed)
        BossHuntConfig fallback = BossHuntConfig.fromJson("{ bad json }");
        check("Malformed JSON produces safe fallback", fallback != null && !fallback.isEnabled());

        // BHX-01 Fail-Closed Edge Cases
        check("Truncated JSON fails closed", !BossHuntConfig.fromJson("{\"enabled\":true").isEnabled());
        check("Malformed trailing garbage fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"clockMode\":1 GARBAGE").isEnabled());
        check("Trailing tokens after root object fail closed", !BossHuntConfig.fromJson("{\"enabled\":true} GARBAGE").isEnabled());
        check("Non-boolean enabled fails closed", !BossHuntConfig.fromJson("{\"enabled\": \"true\"}").isEnabled());
        check("Nested enabled fails closed", !BossHuntConfig.fromJson("{\"other\": {\"enabled\": true}}").isEnabled());
        check("Array root fails closed", !BossHuntConfig.fromJson("[{\"enabled\": true}]").isEnabled());

        // BHX-06 Adversarial Schema Probes (Tech Lead Audit)
        check("Non-numeric clockMode fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"clockMode\":\"invalid\"}").isEnabled());
        check("Non-list bosses fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"bosses\":\"invalid\"}").isEnabled());
        check("Non-numeric zoneMask fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"bosses\":[{\"id\":1,\"zoneMask\":\"invalid\"}]}").isEnabled());
        check("Duplicate enabled key fails closed", !BossHuntConfig.fromJson("{\"enabled\":false,\"enabled\":true}").isEnabled());
        check("Non-object boss item fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"bosses\":[1,2,3]}").isEnabled());

        // BHX-10 Non-Integral Number Probes
        check("Fractional clockMode 1.5 fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"clockMode\":1.5,\"bosses\":[]}").isEnabled());
        check("Fractional boss id 1.9 fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"clockMode\":1,\"bosses\":[{\"id\":1.9,\"zoneMask\":1}]}").isEnabled());
        check("Fractional zoneMask 1.9 fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"clockMode\":1,\"bosses\":[{\"id\":1,\"zoneMask\":1.9}]}").isEnabled());
        check("Fractional travelBudget 100.7 fails closed", !BossHuntConfig.fromJson("{\"enabled\":true,\"clockMode\":1,\"bosses\":[{\"id\":1,\"zoneMask\":1,\"travelBudget\":100.7}]}").isEnabled());
        check("Valid integral numbers succeed", BossHuntConfig.fromJson("{\"enabled\":true,\"clockMode\":1,\"bosses\":[{\"id\":1,\"zoneMask\":1}]}").isEnabled());
    }

    private static void testStateTransitions() {
        System.out.println("\n--- testStateTransitions ---");
        checkEquals("IDLE state name", BossHuntState.IDLE.name(), "IDLE");
        checkEquals("NEED_SCHEDULE state name", BossHuntState.NEED_SCHEDULE.name(), "NEED_SCHEDULE");
        checkEquals("WAIT_DEPART state name", BossHuntState.WAIT_DEPART.name(), "WAIT_DEPART");
        checkEquals("TRAVEL state name", BossHuntState.TRAVEL.name(), "TRAVEL");
        checkEquals("SWITCH_ZONE state name", BossHuntState.SWITCH_ZONE.name(), "SWITCH_ZONE");
        checkEquals("PATROL state name", BossHuntState.PATROL.name(), "PATROL");
        checkEquals("COMBAT state name", BossHuntState.COMBAT.name(), "COMBAT");
        checkEquals("RECOVER state name", BossHuntState.RECOVER.name(), "RECOVER");
        checkEquals("PAUSED state name", BossHuntState.PAUSED.name(), "PAUSED");
        checkEquals("BLOCKED state name", BossHuntState.BLOCKED.name(), "BLOCKED");

        // Action ownership flags
        check("IDLE does not control movement", !BossHuntState.IDLE.controlsMovement());
        check("TRAVEL controls movement", BossHuntState.TRAVEL.controlsMovement());
        check("SWITCH_ZONE controls movement", BossHuntState.SWITCH_ZONE.controlsMovement());
        check("PATROL controls movement", BossHuntState.PATROL.controlsMovement());
        check("COMBAT controls movement", BossHuntState.COMBAT.controlsMovement());
        check("PAUSED does not control movement", !BossHuntState.PAUSED.controlsMovement());
        check("BLOCKED does not control movement", !BossHuntState.BLOCKED.controlsMovement());
    }
}
