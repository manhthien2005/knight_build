import java.io.File;
import java.io.FileOutputStream;

/**
 * High-level Controller and State Machine for Boss Hunt in Zeus.
 * Integrates BossCatalog, BossHuntConfig, BossHuntScheduler, and safe execution gates.
 *
 * Implements BH-06 through BH-10:
 * - BH-06: State machine controlling transitions between IDLE, NEED_SCHEDULE, WAIT_DEPART,
 *          TRAVEL, SWITCH_ZONE, PATROL, COMBAT, RECOVER, PAUSED, and BLOCKED.
 * - BH-07: Grounded target discrimination (validates real boss object, type, hp, isDie, Action).
 * - BH-08: 3-second no-damage rule, patrol timeout, and recovery delay (10s).
 * - BH-09: Yields unconditionally to higher-priority subsystems (Forge, Dungeon, Revive, manual input).
 * - BH-10: Exposes minimal read-only status JSON without modifying CTL v15 or snapshot v6 contracts.
 */
public final class BossHuntController {

    public static final String CLOCK_POLICY_LABEL = "MINH_COMPAT_ABSOLUTE_GMT7";

    public interface MapResolver {
        int resolveMapId(String mapName);
    }

    private final BossHuntConfig config;
    private final BossHuntScheduler scheduler;
    private MapResolver mapResolver;

    private BossHuntState state;
    private BossHuntScheduler.ScheduleEntry activeEntry;
    private int targetMapId;
    private int targetZoneNumber;
    private int targetZoneSubIndex;

    private int activeBossTargetId;
    private String activeBossTargetName;

    private int noDamageTargetId;
    private int noDamageHp;
    private long noDamageSince;

    private long defeatedAt;
    private long arrivedAt;
    private long absentSince;
    private String lastStatus;

    private long lastZoneSwitchAttempt;
    private int zoneSwitchRetryCount;

    public BossHuntController(BossHuntConfig config) {
        this.config = (config != null) ? config : new BossHuntConfig();
        this.scheduler = new BossHuntScheduler(this.config);
        this.state = BossHuntState.IDLE;
        this.targetMapId = -1;
        this.targetZoneNumber = 1;
        this.targetZoneSubIndex = 0;
        this.activeBossTargetId = -1;
        this.activeBossTargetName = null;
        this.noDamageTargetId = -1;
        this.noDamageHp = -1;
        this.noDamageSince = 0L;
        this.defeatedAt = 0L;
        this.arrivedAt = 0L;
        this.absentSince = 0L;
        this.lastZoneSwitchAttempt = 0L;
        this.zoneSwitchRetryCount = 0;
        this.lastStatus = "Chua bat san boss";
    }

    public BossHuntConfig getConfig() {
        return config;
    }

    public BossHuntScheduler getScheduler() {
        return scheduler;
    }

    public void setMapResolver(MapResolver resolver) {
        this.mapResolver = resolver;
    }

    public BossHuntState getState() {
        return state;
    }

    public void setState(BossHuntState newState) {
        this.state = newState;
    }

    public BossHuntScheduler.ScheduleEntry getActiveEntry() {
        return activeEntry;
    }

    public void setActiveEntry(BossHuntScheduler.ScheduleEntry entry) {
        this.activeEntry = entry;
    }

    public int getBossHuntTargetMapId() {
        if (state == BossHuntState.TRAVEL || state == BossHuntState.SWITCH_ZONE
                || state == BossHuntState.PATROL || state == BossHuntState.COMBAT) {
            return targetMapId;
        }
        return -1;
    }

    public int getTargetMapId() {
        return targetMapId;
    }

    public int getTargetZoneNumber() {
        return targetZoneNumber;
    }

    public int getTargetZoneSubIndex() {
        return targetZoneSubIndex;
    }

    public int getActiveBossTargetId() {
        return activeBossTargetId;
    }

    public String getActiveBossTargetName() {
        return activeBossTargetName;
    }

    public void setActiveBossTarget(int targetId, String targetName) {
        this.activeBossTargetId = targetId;
        this.activeBossTargetName = targetName;
    }

    public void clearActiveBossTarget() {
        this.activeBossTargetId = -1;
        this.activeBossTargetName = null;
    }

    public boolean hasActiveBossTarget() {
        return activeBossTargetId >= 0;
    }

    public boolean shouldAttemptZoneSwitch(long now) {
        return (now - lastZoneSwitchAttempt >= 1500L);
    }

    public void recordZoneSwitchAttempt(long now) {
        lastZoneSwitchAttempt = now;
        zoneSwitchRetryCount++;
    }

    public void onZoneConfirmed() {
        zoneSwitchRetryCount = 0;
        lastZoneSwitchAttempt = 0L;
        state = BossHuntState.PATROL;
    }

    public boolean isZoneSwitchExhausted() {
        return zoneSwitchRetryCount >= 5;
    }

    public void onZoneSwitchFailed() {
        zoneSwitchRetryCount = 0;
        lastZoneSwitchAttempt = 0L;
        state = BossHuntState.NEED_SCHEDULE;
        activeEntry = null;
        lastStatus = "Doi khu that bai sau 5 lan thu, can cap nhat lich moi";
    }

    public void onNoDamageTimeout(long now) {
        resetNoDamageTimer();
        lastStatus = "Quai khong giam HP sau 3 giay, doi khu hoac tuan tra";
        state = BossHuntState.PATROL;
    }

    public String getStatusText() {
        return lastStatus;
    }

    public boolean isEnabled() {
        return config.isEnabled();
    }

    public void setEnabled(boolean enabled) {
        config.setEnabled(enabled);
        if (enabled) {
            if (state == BossHuntState.IDLE) {
                state = BossHuntState.NEED_SCHEDULE;
                lastStatus = "Dang cho lich san boss";
            }
        } else {
            state = BossHuntState.IDLE;
            activeEntry = null;
            targetMapId = -1;
            lastStatus = "San boss da tat";
            clearActiveBossTarget();
            resetNoDamageTimer();
            zoneSwitchRetryCount = 0;
            lastZoneSwitchAttempt = 0L;
        }
    }

    public boolean controlsMovement() {
        return config.isEnabled() && state.controlsMovement();
    }

    public static boolean isValidBossTarget(int expectedBossId, String targetName,
                                            int typeObject, int hp, boolean isDie, int action) {
        if (typeObject != 1) {
            return false; // must be a monster
        }
        if (hp <= 0 || isDie || action == 4) {
            return false; // must be alive
        }
        if (targetName == null) {
            return false;
        }
        int bossId = BossCatalog.findBossId(targetName);
        return bossId == expectedBossId;
    }

    public static boolean isBossDefeated(int expectedBossId, String targetName,
                                         int typeObject, int hp, boolean isDie, int action) {
        if (typeObject != 1) {
            return false; // must be a monster
        }
        if (targetName == null) {
            return false;
        }
        int bossId = BossCatalog.findBossId(targetName);
        if (bossId != expectedBossId) {
            return false;
        }
        return (hp <= 0 || isDie || action == 4);
    }

    public boolean checkNoDamageFor3s(int targetId, int currentHp, long now) {
        if (targetId != noDamageTargetId || currentHp != noDamageHp) {
            noDamageTargetId = targetId;
            noDamageHp = currentHp;
            noDamageSince = now;
            return false;
        }
        return (now - noDamageSince >= 3000L);
    }

    public void resetNoDamageTimer() {
        noDamageTargetId = -1;
        noDamageHp = -1;
        noDamageSince = 0L;
    }

    public void onBossDefeated(long now) {
        defeatedAt = now;
        state = BossHuntState.RECOVER;
        lastStatus = "Boss da bi tieu diet, cho 10 giay";
        clearActiveBossTarget();
        resetNoDamageTimer();
        zoneSwitchRetryCount = 0;
        lastZoneSwitchAttempt = 0L;
    }

    public void tick(long now, int currentMapId, int currentZoneIndex,
                     int playerHp, boolean isDead, boolean manualMovement, boolean higherPriorityActive) {
        if (!config.isEnabled()) {
            state = BossHuntState.IDLE;
            return;
        }

        // BH-09: Death arbitration
        if (isDead || playerHp <= 0) {
            state = BossHuntState.PAUSED;
            lastStatus = "Nhan vat chet, tam dung san boss";
            return;
        }

        // BH-09: Higher priority arbitration (Forge, Dungeon)
        if (higherPriorityActive) {
            state = BossHuntState.PAUSED;
            lastStatus = "Nhuong quyen cho tinh nang uu tien";
            return;
        }

        // BH-09: Manual movement arbitration
        if (manualMovement) {
            state = BossHuntState.PAUSED;
            lastStatus = "Nguoi choi dieu khien, tam dung san boss";
            return;
        }

        // Recovery wait (10s)
        if (state == BossHuntState.RECOVER) {
            if (now - defeatedAt >= 10000L) {
                state = BossHuntState.NEED_SCHEDULE;
                activeEntry = null;
                lastStatus = "Hoi phuc xong 10s, can cap nhat lich moi";
            }
            return;
        }

        // Resuming from PAUSED
        if (state == BossHuntState.PAUSED) {
            if (activeEntry != null) {
                state = BossHuntState.TRAVEL;
            } else {
                state = BossHuntState.NEED_SCHEDULE;
            }
        }

        // Check if we need to select next schedule entry
        if (activeEntry == null) {
            activeEntry = scheduler.nextEntry(now);
            if (activeEntry == null) {
                state = BossHuntState.NEED_SCHEDULE;
                lastStatus = "Chua co lich boss phu hop";
                return;
            }
        }

        // Resolve map ID
        if (mapResolver != null && activeEntry.mapAlias != null) {
            targetMapId = mapResolver.resolveMapId(activeEntry.mapAlias);
        } else {
            targetMapId = -1;
        }
        targetZoneNumber = activeEntry.zone;
        targetZoneSubIndex = activeEntry.zone - 1;

        // Departure check
        long budget = config.getTravelBudget(activeEntry.bossId);
        if (!BossHuntScheduler.shouldDepart(activeEntry.dueEpochMillis, now, budget)) {
            state = BossHuntState.WAIT_DEPART;
            lastStatus = "Cho den gio san boss " + BossCatalog.getBossName(activeEntry.bossId);
            return;
        }

        // Movement to map check
        if (targetMapId >= 0 && currentMapId != targetMapId) {
            state = BossHuntState.TRAVEL;
            lastStatus = "Dang di chuyen toi map " + activeEntry.mapAlias;
            return;
        }

        // Zone switch check
        if (currentZoneIndex != targetZoneSubIndex) {
            state = BossHuntState.SWITCH_ZONE;
            lastStatus = "Dang doi sang Khu " + targetZoneNumber;
            return;
        }

        // Arrived at map & zone!
        if (state != BossHuntState.COMBAT) {
            if (arrivedAt == 0L) {
                arrivedAt = now;
            }
            if (absentSince == 0L) {
                absentSince = now;
            } else if (now - absentSince > 90000L) {
                state = BossHuntState.NEED_SCHEDULE;
                activeEntry = null;
                absentSince = 0L;
                arrivedAt = 0L;
                lastStatus = "Boss khong xuat hien sau 90 giay, yeu cau lay lich moi";
                return;
            }
            state = BossHuntState.PATROL;
            lastStatus = "Dang tuan tra tim boss tren map";
        } else {
            absentSince = 0L;
        }
    }

    public String toStatusJson(long now) {
        StringBuffer sb = new StringBuffer(512);
        sb.append("{\n");
        sb.append("  \"timestamp\": ").append(now).append(",\n");
        sb.append("  \"enabled\": ").append(config.isEnabled()).append(",\n");
        sb.append("  \"state\": \"").append(state.name()).append("\",\n");
        sb.append("  \"clockPolicy\": \"").append(CLOCK_POLICY_LABEL).append("\",\n");
        sb.append("  \"statusText\": ");
        escapeJson(lastStatus, sb);
        sb.append(",\n");
        if (activeEntry != null) {
            sb.append("  \"activeBoss\": {\n");
            sb.append("    \"id\": ").append(activeEntry.bossId).append(",\n");
            sb.append("    \"name\": ");
            escapeJson(BossCatalog.getBossName(activeEntry.bossId), sb);
            sb.append(",\n");
            sb.append("    \"zone\": ").append(activeEntry.zone).append(",\n");
            sb.append("    \"map\": ");
            escapeJson(activeEntry.mapAlias, sb);
            sb.append(",\n");
            sb.append("    \"dueEpochMillis\": ").append(activeEntry.dueEpochMillis).append("\n");
            sb.append("  }\n");
        } else {
            sb.append("  \"activeBoss\": null\n");
        }
        sb.append("}\n");
        return sb.toString();
    }

    public void saveStatusToFile(String path, long now) {
        if (path == null) {
            return;
        }
        FileOutputStream fos = null;
        File temp = null;
        try {
            String json = toStatusJson(now);
            byte[] bytes = json.getBytes("UTF-8");
            File target = new File(path);
            File parent = target.getParentFile();
            if (parent != null && !parent.exists()) {
                parent.mkdirs();
            }
            temp = new File(path + ".tmp." + System.currentTimeMillis());
            fos = new FileOutputStream(temp);
            fos.write(bytes);
            fos.flush();
            try {
                fos.getFD().sync();
            } catch (Throwable ignored) {}
            fos.close();
            fos = null;

            File bak = new File(path + ".bak");
            boolean hadTarget = target.exists();
            if (hadTarget) {
                if (bak.exists()) {
                    bak.delete();
                }
                if (!target.renameTo(bak)) {
                    temp.delete();
                    return;
                }
            }
            if (temp.renameTo(target)) {
                if (hadTarget && bak.exists()) {
                    bak.delete();
                }
            } else {
                if (hadTarget && bak.exists()) {
                    bak.renameTo(target);
                }
                temp.delete();
            }
        } catch (Throwable ignored) {
            if (temp != null && temp.exists()) {
                try { temp.delete(); } catch (Throwable t2) {}
            }
        } finally {
            if (fos != null) {
                try { fos.close(); } catch (Throwable ignored) {}
            }
        }
    }

    private static void escapeJson(String s, StringBuffer sb) {
        if (s == null) {
            sb.append("null");
            return;
        }
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '"') {
                sb.append("\\\"");
            } else if (c == '\\') {
                sb.append("\\\\");
            } else if (c == '\n') {
                sb.append("\\n");
            } else if (c == '\r') {
                sb.append("\\r");
            } else if (c == '\t') {
                sb.append("\\t");
            } else {
                sb.append(c);
            }
        }
        sb.append('"');
    }
}
