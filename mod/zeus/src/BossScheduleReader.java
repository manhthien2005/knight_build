import java.io.File;
import java.io.FileOutputStream;
import java.util.ArrayList;
import java.util.List;

/**
 * Deterministic, read-only Boss Schedule Reader for Zeus.
 * Integrates BossScheduleParser with real SERVER Boss schedule menu observations.
 *
 * Safety & semantic guarantees:
 * - Read-only: never drives menus, never sends packets, never moves player.
 * - Fail-closed time semantics: single runtime snapshot cannot prove relative countdown vs absolute GMT+7;
 *   clockMode is exposed strictly as UNKNOWN, dueTimeVerified is false, and dueEpochMillis is null.
 * - Player location isolation: preserves map alias from schedule row, never confused with player's mapId/zone.
 * - Pure Java 6 bytecode compatibility (major version 50).
 */
public final class BossScheduleReader {

    public static final String CLOCK_MODE_UNKNOWN = "UNKNOWN";
    public static final String CLOCK_MODE_EVIDENCE_NONE = "none: single runtime snapshot without delta does not distinguish relative countdown from absolute GMT+7";
    public static final String SIDE_CAR_FILENAME = "zeus-boss-schedule.json";
    public static final int MAX_SIDECAR_BYTES = 256 * 1024;

    private static String lastScheduleFingerprint = null;
    private static long lastScheduleTime = 0L;
    private static volatile ScheduleReport latestReport = null;

    public static ScheduleReport getLatestReport() {
        return latestReport;
    }

    public static final class Entry {
        public final String boss;
        public final String mapAlias;
        public final int zone;
        public final String rawTimeText;
        public final String parseStatus;
        public final String clockMode;
        public final String clockModeEvidence;
        public final boolean dueTimeVerified;
        public final Long dueEpochMillis;
        public final String rawCaption;

        public Entry(String boss, String mapAlias, int zone, String rawTimeText,
                     String parseStatus, String clockMode, String clockModeEvidence,
                     boolean dueTimeVerified, Long dueEpochMillis, String rawCaption) {
            this.boss = boss;
            this.mapAlias = mapAlias;
            this.zone = zone;
            this.rawTimeText = rawTimeText;
            this.parseStatus = parseStatus;
            this.clockMode = clockMode;
            this.clockModeEvidence = clockModeEvidence;
            this.dueTimeVerified = dueTimeVerified;
            this.dueEpochMillis = dueEpochMillis;
            this.rawCaption = rawCaption;
        }
    }

    public static final class ScheduleReport {
        public final long timestamp;
        public final String origin;
        public final int idMenu;
        public final int idNPC;
        public final String boss;
        public final String clockMode;
        public final String clockModeEvidence;
        public final boolean dueTimeVerified;
        public final Entry[] entries;

        public ScheduleReport(long timestamp, String origin, int idMenu, int idNPC,
                              String boss, Entry[] entries) {
            this.timestamp = timestamp;
            this.origin = origin;
            this.idMenu = idMenu;
            this.idNPC = idNPC;
            this.boss = boss;
            this.clockMode = CLOCK_MODE_UNKNOWN;
            this.clockModeEvidence = CLOCK_MODE_EVIDENCE_NONE;
            this.dueTimeVerified = false;
            this.entries = (entries != null) ? entries : new Entry[0];
        }

        public int rowCount() {
            return entries.length;
        }

        public String toJson() {
            StringBuffer sb = new StringBuffer(1024);
            sb.append("{\n");
            sb.append("  \"timestamp\": ").append(timestamp).append(",\n");
            sb.append("  \"origin\": ");
            escapeJson(origin, sb);
            sb.append(",\n");
            sb.append("  \"idMenu\": ").append(idMenu).append(",\n");
            sb.append("  \"idNPC\": ").append(idNPC).append(",\n");
            sb.append("  \"boss\": ");
            escapeJson(boss, sb);
            sb.append(",\n");
            sb.append("  \"clockMode\": ");
            escapeJson(clockMode, sb);
            sb.append(",\n");
            sb.append("  \"clockModeEvidence\": ");
            escapeJson(clockModeEvidence, sb);
            sb.append(",\n");
            sb.append("  \"dueTimeVerified\": ").append(dueTimeVerified).append(",\n");
            sb.append("  \"rowCount\": ").append(entries.length).append(",\n");
            sb.append("  \"rows\": [");
            for (int i = 0; i < entries.length; i++) {
                Entry e = entries[i];
                if (i > 0) {
                    sb.append(",");
                }
                sb.append("\n    {\n");
                sb.append("      \"boss\": ");
                escapeJson(e.boss, sb);
                sb.append(",\n");
                sb.append("      \"mapAlias\": ");
                escapeJson(e.mapAlias, sb);
                sb.append(",\n");
                sb.append("      \"zone\": ").append(e.zone).append(",\n");
                sb.append("      \"rawTimeText\": ");
                escapeJson(e.rawTimeText, sb);
                sb.append(",\n");
                sb.append("      \"parseStatus\": ");
                escapeJson(e.parseStatus, sb);
                sb.append(",\n");
                sb.append("      \"clockMode\": ");
                escapeJson(e.clockMode, sb);
                sb.append(",\n");
                sb.append("      \"clockModeEvidence\": ");
                escapeJson(e.clockModeEvidence, sb);
                sb.append(",\n");
                sb.append("      \"dueTimeVerified\": ").append(e.dueTimeVerified).append(",\n");
                sb.append("      \"dueEpochMillis\": null,\n");
                sb.append("      \"rawCaption\": ");
                escapeJson(e.rawCaption, sb);
                sb.append("\n    }");
            }
            if (entries.length > 0) {
                sb.append("\n  ");
            }
            sb.append("]\n");
            sb.append("}\n");
            return sb.toString();
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
                sb.append('\\');
                sb.append('"');
            } else if (c == '\\') {
                sb.append('\\');
                sb.append('\\');
            } else if (c == '\b') {
                sb.append('\\');
                sb.append('b');
            } else if (c == '\f') {
                sb.append('\\');
                sb.append('f');
            } else if (c == '\n') {
                sb.append('\\');
                sb.append('n');
            } else if (c == '\r') {
                sb.append('\\');
                sb.append('r');
            } else if (c == '\t') {
                sb.append('\\');
                sb.append('t');
            } else if (c < 32) {
                String hex = Integer.toHexString(c);
                sb.append('\\');
                sb.append('u');
                for (int k = hex.length(); k < 4; k++) {
                    sb.append('0');
                }
                sb.append(hex);
            } else {
                sb.append(c);
            }
        }
        sb.append('"');
    }

    private BossScheduleReader() {
    }

    public static ScheduleReport readSchedule(String origin, String[] captions, int idMenu, int idNPC, long now) {
        if (origin == null || !origin.equals("SERVER")) {
            return null;
        }
        if (captions == null || captions.length == 0) {
            return null;
        }

        // Sensitive check: Fail closed if any caption contains sensitive data
        for (int i = 0; i < captions.length; i++) {
            if (Zeus.containsSensitive(captions[i])) {
                return null;
            }
        }

        // Check if menu is classified as MATCH_BOSS_SCHEDULE
        String classification = Zeus.classifyBossMenu("MENU", captions);
        if (classification == null || !classification.equals("MATCH_BOSS_SCHEDULE")) {
            return null;
        }

        List validEntries = new ArrayList();
        String detectedBoss = null;

        for (int i = 0; i < captions.length; i++) {
            String caption = captions[i];
            if (caption == null || caption.trim().length() == 0) {
                continue;
            }

            // Extract candidate boss name and zone info
            String candidateBoss = BossScheduleParser.extractCandidateBoss(caption);
            List zones = BossScheduleParser.scanZones(caption);
            int zone = (zones != null && zones.size() == 1) ? ((BossScheduleParser.ZoneInfo) zones.get(0)).zone : -1;

            if (candidateBoss != null && detectedBoss == null) {
                detectedBoss = candidateBoss;
            }

            // Extract raw time text
            int timeIdx = BossScheduleParser.firstTime(caption);
            String rawTime = "";
            if (timeIdx >= 0) {
                int endT = timeIdx;
                while (endT < caption.length() && (Character.isDigit(caption.charAt(endT)) || caption.charAt(endT) == ':')) {
                    endT++;
                }
                rawTime = caption.substring(timeIdx, endT).trim();
            }

            // Probe with BossScheduleParser in relative mode solely to validate syntax, map alias, and row structure
            String mapAlias = null;
            String statusStr = "UNKNOWN";

            if (candidateBoss != null && zone > 0) {
                BossScheduleParser.Result res = BossScheduleParser.parse(
                    caption, candidateBoss, zone, BossScheduleParser.CLOCK_MODE_RELATIVE_COUNTDOWN, now
                );
                if (res != null) {
                    mapAlias = res.mapAlias;
                    if (res.status == BossScheduleParser.Status.SCHEDULED) {
                        statusStr = "SCHEDULED";
                    } else if (res.status == BossScheduleParser.Status.SPAWNED_OR_PAST) {
                        statusStr = "SPAWNED_OR_PAST";
                    } else if (res.status == BossScheduleParser.Status.UNAVAILABLE) {
                        statusStr = "UNAVAILABLE";
                    }
                }
            }

            // Fallback for mapAlias if null
            if (mapAlias == null && timeIdx >= 0 && zones != null && zones.size() == 1) {
                int zoneStart = ((BossScheduleParser.ZoneInfo) zones.get(0)).startIndex;
                mapAlias = BossScheduleParser.mapInRow(caption, timeIdx, zoneStart);
            }

            Entry entry = new Entry(
                candidateBoss != null ? candidateBoss : "",
                mapAlias != null ? mapAlias : "",
                zone,
                rawTime,
                statusStr,
                CLOCK_MODE_UNKNOWN,
                CLOCK_MODE_EVIDENCE_NONE,
                false,  // dueTimeVerified strictly false
                null,   // dueEpochMillis strictly null
                caption
            );
            validEntries.add(entry);
        }

        if (validEntries.isEmpty()) {
            return null;
        }

        Entry[] array = (Entry[]) validEntries.toArray(new Entry[validEntries.size()]);
        return new ScheduleReport(now, origin, idMenu, idNPC, detectedBoss != null ? detectedBoss : "", array);
    }

    public static void writeScheduleJson(ScheduleReport report, String targetPath) {
        if (report == null || targetPath == null) {
            return;
        }
        try {
            String json = report.toJson();
            byte[] bytes = json.getBytes("UTF-8");
            if (bytes.length > MAX_SIDECAR_BYTES) {
                return;
            }

            File targetFile = new File(targetPath);
            File parentDir = targetFile.getParentFile();
            if (parentDir != null && !parentDir.exists()) {
                parentDir.mkdirs();
            }

            File tempFile = new File(targetPath + ".tmp." + System.currentTimeMillis());
            FileOutputStream fos = new FileOutputStream(tempFile);
            fos.write(bytes);
            fos.flush();
            try { fos.getFD().sync(); } catch (Throwable ignored) {}
            fos.close();

            File bak = new File(targetPath + ".bak");
            boolean hadTarget = targetFile.exists();
            if (hadTarget) {
                if (bak.exists()) {
                    bak.delete();
                }
                if (!targetFile.renameTo(bak)) {
                    tempFile.delete();
                    return;
                }
            }
            if (tempFile.renameTo(targetFile)) {
                if (hadTarget && bak.exists()) {
                    bak.delete();
                }
            } else {
                if (hadTarget && bak.exists()) {
                    bak.renameTo(targetFile);
                }
                tempFile.delete();
            }
        } catch (Throwable ignored) {
            // Fail silent
        }
    }

    public static void processSchedule(String origin, String[] captions, int idMenu, int idNPC,
                                        String title, long now, String targetPath) {
        try {
            if (captions == null || captions.length == 0) {
                return;
            }

            // Fingerprint deduplication (5 seconds)
            StringBuffer fp = new StringBuffer(128);
            fp.append(origin).append('|').append(title != null ? title : "").append('|')
              .append(idNPC).append('|').append(idMenu);
            for (int i = 0; i < captions.length; i++) {
                if (captions[i] != null) {
                    fp.append('|').append(captions[i]);
                }
            }
            String fpStr = fp.toString();
            if (fpStr.equals(lastScheduleFingerprint) && (now - lastScheduleTime < 5000L)) {
                return;
            }
            lastScheduleFingerprint = fpStr;
            lastScheduleTime = now;

            ScheduleReport report = readSchedule(origin, captions, idMenu, idNPC, now);
            if (report != null) {
                latestReport = report;
                if (targetPath != null) {
                    writeScheduleJson(report, targetPath);
                }
                try {
                    Zeus.bossHunt.getScheduler().ingestReport(report, now);
                } catch (Throwable ignored) {
                }
            }
        } catch (Throwable ignored) {
            // Fail silent
        }
    }
}
