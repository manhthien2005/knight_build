import java.util.List;

/**
 * Schedule Integration & Departure Planner for Zeus Boss Hunt.
 * Implements BH-04 (schedule consumption), BH-05 (clock modes, rollover, departure budget).
 *
 * Fully Java 6 compatible (major 50).
 */
public final class BossHuntScheduler {

    public static final class ScheduleEntry {
        public final int bossId;
        public final int zone;
        public final String mapAlias;
        public final String rawCaption;
        public final long dueEpochMillis;
        public final BossScheduleParser.Status status;

        public ScheduleEntry(int bossId, int zone, String mapAlias, String rawCaption,
                             long dueEpochMillis, BossScheduleParser.Status status) {
            this.bossId = bossId;
            this.zone = zone;
            this.mapAlias = mapAlias;
            this.rawCaption = rawCaption;
            this.dueEpochMillis = dueEpochMillis;
            this.status = status;
        }

        public boolean isSpawned() {
            return status == BossScheduleParser.Status.SPAWNED_OR_PAST || dueEpochMillis == 0L;
        }

        public boolean isScheduled() {
            return status == BossScheduleParser.Status.SCHEDULED && dueEpochMillis > 0L;
        }
    }

    private final BossHuntConfig config;
    private final ScheduleEntry[][] table; // [bossId][zone-1]

    public BossHuntScheduler(BossHuntConfig config) {
        this.config = (config != null) ? config : new BossHuntConfig();
        this.table = new ScheduleEntry[BossCatalog.BOSS_COUNT][5];
    }

    public ScheduleEntry getEntry(int bossId, int zone) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT && zone >= 1 && zone <= 5) {
            return table[bossId][zone - 1];
        }
        return null;
    }

    public boolean ingestRow(String caption, long now) {
        if (caption == null || caption.trim().length() == 0) {
            return false;
        }

        String candidateBoss = BossScheduleParser.extractCandidateBoss(caption);
        if (candidateBoss == null) {
            return false;
        }

        int bossId = BossCatalog.findBossId(candidateBoss);
        if (bossId < 0) {
            return false;
        }

        List zones = BossScheduleParser.scanZones(caption);
        if (zones == null || zones.size() != 1) {
            return false;
        }
        int zone = ((BossScheduleParser.ZoneInfo) zones.get(0)).zone;
        if (zone < 1 || zone > 5) {
            return false;
        }

        // Check if operator disabled this zone
        if (!config.isZoneSelected(bossId, zone)) {
            table[bossId][zone - 1] = null;
            return false;
        }

        // Check spawned keywords
        String norm = BossCatalog.normalize(caption);
        if (norm.indexOf("dang xuat hien") >= 0 || norm.indexOf("da xuat hien") >= 0) {
            String mapAlias = config.getEffectiveMap(bossId, null);
            ScheduleEntry entry = new ScheduleEntry(bossId, zone, mapAlias, caption, 0L,
                    BossScheduleParser.Status.SPAWNED_OR_PAST);
            table[bossId][zone - 1] = entry;
            return true;
        }

        // Check unavailable keywords
        if (norm.indexOf("khong co thong tin") >= 0 || norm.indexOf("khong co lich") >= 0) {
            ScheduleEntry entry = new ScheduleEntry(bossId, zone, null, caption, -1L,
                    BossScheduleParser.Status.UNAVAILABLE);
            table[bossId][zone - 1] = entry;
            return true;
        }

        // Parse scheduled time using configured clock mode
        int clockMode = config.getClockMode();
        BossScheduleParser.Result res = BossScheduleParser.parse(
                caption, candidateBoss, zone, clockMode, now
        );

        if (res == null || res.status == BossScheduleParser.Status.UNKNOWN) {
            return false;
        }

        long due = (res.dueEpochMillis != null) ? res.dueEpochMillis.longValue() : -1L;
        String effectiveMap = config.getEffectiveMap(bossId, res.mapAlias);

        ScheduleEntry entry = new ScheduleEntry(bossId, zone, effectiveMap, caption, due, res.status);
        table[bossId][zone - 1] = entry;
        return true;
    }

    public void ingestReport(BossScheduleReader.ScheduleReport report, long now) {
        if (report == null || report.entries == null) {
            return;
        }
        for (int i = 0; i < report.entries.length; i++) {
            BossScheduleReader.Entry e = report.entries[i];
            if (e != null && e.rawCaption != null) {
                ingestRow(e.rawCaption, now);
            }
        }
    }

    public void rebuild() {
        for (int b = 0; b < BossCatalog.BOSS_COUNT; b++) {
            for (int z = 1; z <= 5; z++) {
                if (!config.isZoneSelected(b, z)) {
                    table[b][z - 1] = null;
                }
            }
        }
    }

    public ScheduleEntry nextEntry(long now) {
        ScheduleEntry best = null;
        for (int b = 0; b < BossCatalog.BOSS_COUNT; b++) {
            for (int z = 1; z <= 5; z++) {
                if (!config.isZoneSelected(b, z)) {
                    continue;
                }
                ScheduleEntry e = table[b][z - 1];
                if (e == null || e.mapAlias == null || e.dueEpochMillis < 0L) {
                    continue;
                }

                if (best == null) {
                    best = e;
                } else {
                    // 0L (active/spawned) always has top priority
                    if (e.dueEpochMillis == 0L && best.dueEpochMillis != 0L) {
                        best = e;
                    } else if (best.dueEpochMillis != 0L && e.dueEpochMillis < best.dueEpochMillis) {
                        best = e;
                    }
                }
            }
        }
        return best;
    }

    public static boolean shouldDepart(long dueEpochMillis, long nowEpochMillis, long travelBudgetMs) {
        if (dueEpochMillis < 0L) {
            return false;
        }
        if (dueEpochMillis == 0L) {
            return true; // spawned/active, depart immediately
        }
        long window = Math.max(30000L, travelBudgetMs);
        return nowEpochMillis >= dueEpochMillis - window;
    }
}
