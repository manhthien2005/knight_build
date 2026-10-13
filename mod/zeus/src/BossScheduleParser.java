import java.util.ArrayList;
import java.util.Calendar;
import java.util.Date;
import java.util.List;
import java.util.Locale;
import java.util.TimeZone;

/**
 * Deterministic, side-effect-free Boss Schedule Parser for Zeus.
 * Based on independently verified ModByTminh v4.0.3 bytecode semantics
 * (BossHunt.consume, scheduleRow, firstTime, mapInRow, parseDue).
 *
 * Fully Java 6 compatible, pure Java with zero external/game-client dependencies.
 */
public final class BossScheduleParser {

    public static final int CLOCK_MODE_UNKNOWN = 0;
    public static final int CLOCK_MODE_ABSOLUTE_GMT7 = 1;
    public static final int CLOCK_MODE_RELATIVE_COUNTDOWN = 2;

    public static final String TIMEZONE_GMT7 = "GMT+07:00";
    private static final TimeZone TZ_GMT7 = TimeZone.getTimeZone(TIMEZONE_GMT7);

    public enum Status {
        SCHEDULED,
        SPAWNED_OR_PAST,
        UNAVAILABLE,
        UNKNOWN
    }

    public static final class Result {
        public final Status status;
        public final String expectedBoss;
        public final int expectedZone;
        public final Long dueEpochMillis;
        public final String mapAlias;
        public final String rawCaption;
        public final String reason;

        public Result(Status status, String expectedBoss, int expectedZone,
                      Long dueEpochMillis, String mapAlias, String rawCaption, String reason) {
            this.status = status;
            this.expectedBoss = expectedBoss;
            this.expectedZone = expectedZone;
            this.dueEpochMillis = dueEpochMillis;
            this.mapAlias = mapAlias;
            this.rawCaption = rawCaption;
            this.reason = reason;
        }

        public boolean isScheduled() {
            return status == Status.SCHEDULED;
        }

        public boolean isSpawnedOrPast() {
            return status == Status.SPAWNED_OR_PAST;
        }

        public boolean isUnavailable() {
            return status == Status.UNAVAILABLE;
        }

        public boolean isUnknown() {
            return status == Status.UNKNOWN;
        }

        public String toString() {
            return "Result[status=" + status + ", boss=" + expectedBoss + ", zone=" + expectedZone
                    + ", due=" + dueEpochMillis + ", map=" + mapAlias + ", reason=" + reason + "]";
        }
    }

    public static final class ZoneInfo {
        public final int zone;
        public final int startIndex;
        public final int endIndex;

        public ZoneInfo(int zone, int startIndex, int endIndex) {
            this.zone = zone;
            this.startIndex = startIndex;
            this.endIndex = endIndex;
        }
    }

    private BossScheduleParser() {
        // Pure static utility class
    }

    /**
     * Extracts the candidate boss name from the beginning of a caption row.
     * Prevents false prefix substring matching (B-05).
     */
    public static String extractCandidateBoss(String caption) {
        if (caption == null) {
            return null;
        }
        String trimmed = caption.trim();
        if (trimmed.length() == 0) {
            return null;
        }

        int endIdx = trimmed.length();

        // 1. Time token (HH:mm:ss) index
        int timeIdx = firstTime(trimmed);
        if (timeIdx >= 0 && timeIdx < endIdx) {
            endIdx = timeIdx;
        }

        // 2. Delimiter hyphens or colons
        int dashIdx = trimmed.indexOf('-');
        if (dashIdx >= 0 && dashIdx < endIdx) {
            endIdx = dashIdx;
        }
        int colonIdx = trimmed.indexOf(':');
        if (colonIdx >= 0 && colonIdx < endIdx) {
            endIdx = colonIdx;
        }

        // 3. Zone token ("khu", "k" followed by digit)
        int zoneIdx = zoneSuffixStart(trimmed);
        if (zoneIdx >= 0 && zoneIdx < endIdx) {
            endIdx = zoneIdx;
        }

        // 4. Status keyword phrases
        String normLower = normSemantic(trimmed);
        int statusIdx = -1;
        String[] keywords = new String[] {
            "dang xuat hien",
            "da xuat hien",
            "khong co thong tin",
            "khong co lich"
        };
        for (int i = 0; i < keywords.length; i++) {
            int kIdx = normLower.indexOf(keywords[i]);
            if (kIdx >= 0 && (statusIdx < 0 || kIdx < statusIdx)) {
                statusIdx = kIdx;
            }
        }
        if (statusIdx >= 0 && statusIdx < endIdx) {
            endIdx = statusIdx;
        }

        String candidate = trimmed.substring(0, endIdx).trim();
        // Strip trailing punctuation
        while (candidate.length() > 0) {
            char c = candidate.charAt(candidate.length() - 1);
            if (c == '-' || c == ':' || c == '.' || c == ',' || c <= ' ') {
                candidate = candidate.substring(0, candidate.length() - 1).trim();
            } else {
                break;
            }
        }
        // Strip leading punctuation if any
        while (candidate.length() > 0) {
            char c = candidate.charAt(0);
            if (c == '-' || c == ':' || c == '.' || c == ',' || c <= ' ') {
                candidate = candidate.substring(1).trim();
            } else {
                break;
            }
        }

        return candidate.length() > 0 ? candidate : null;
    }

    /**
     * Parse an NPC / menu caption row for boss schedule information.
     *
     * @param caption          Raw caption string from NPC menu.
     * @param expectedBoss     Expected boss name/identity.
     * @param expectedZone     Expected zone (1-indexed).
     * @param clockMode        Clock mode: 1 (ABSOLUTE GMT+07:00) or 2 (RELATIVE countdown).
     * @param nowEpochMillis   Injected current epoch timestamp in milliseconds.
     * @return Structured Parse Result.
     */
    public static Result parse(String caption, String expectedBoss, int expectedZone,
                               int clockMode, long nowEpochMillis) {
        if (caption == null || caption.trim().length() == 0) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Empty or null caption");
        }
        if (expectedBoss == null || expectedBoss.trim().length() == 0) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Missing expected boss identity");
        }
        if (expectedZone <= 0) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Invalid expected zone: " + expectedZone);
        }

        // B-06: Guard nowEpochMillis bounds against negative values or overflow near Long.MAX_VALUE
        if (nowEpochMillis < 0L || nowEpochMillis > Long.MAX_VALUE - 86400000L) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Current epoch timestamp out of safe bounds: " + nowEpochMillis);
        }

        // B-05: Exact candidate boss identity matching
        String candidateBoss = extractCandidateBoss(caption);
        if (candidateBoss == null) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Unable to extract boss identity from caption");
        }

        String normCandidate = normSemantic(candidateBoss);
        String normBoss = normSemantic(expectedBoss);

        if (!normCandidate.equals(normBoss)) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Caption does not match expected boss: candidate [" + candidateBoss + "] != expected [" + expectedBoss + "]");
        }

        // B-07, B-08: Check for malformed time tokens before attempting scheduled parsing
        if (hasMalformedTimeToken(caption)) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Malformed time token detected in caption");
        }

        // B-09, B-10: Scan and validate zone tokens across caption
        List<ZoneInfo> zones = scanZones(caption);
        if (zones == null) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Malformed zone token detected in caption");
        }
        if (zones.size() > 1) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Ambiguous caption: multiple zone tokens detected (" + zones.size() + ")");
        }

        String normCaption = normSemantic(caption);

        // Check for unavailable status keywords
        if (normCaption.indexOf("khong co thong tin") >= 0 || normCaption.indexOf("khong co lich") >= 0) {
            if (zones.size() == 1) {
                int foundZone = zones.get(0).zone;
                if (foundZone != expectedZone) {
                    return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                            "Zone mismatch for unavailable status: found " + foundZone + ", expected " + expectedZone);
                }
            }
            return new Result(Status.UNAVAILABLE, expectedBoss, expectedZone, null, null, caption,
                    "Boss schedule unavailable in source");
        }

        // Check for active spawned status keywords ("dang xuat hien", "da xuat hien")
        if (normCaption.indexOf("dang xuat hien") >= 0 || normCaption.indexOf("da xuat hien") >= 0) {
            if (zones.size() != 1) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Zone required for spawned status");
            }
            int foundZone = zones.get(0).zone;
            if (foundZone != expectedZone) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Zone mismatch for spawned status: found " + foundZone + ", expected " + expectedZone);
            }
            String mapAlias = extractMapFromSpawnedCaption(caption, foundZone);
            return new Result(Status.SPAWNED_OR_PAST, expectedBoss, expectedZone, null, mapAlias, caption,
                    "Boss currently spawned or active");
        }

        // Scheduled status parsing
        if (clockMode != CLOCK_MODE_ABSOLUTE_GMT7 && clockMode != CLOCK_MODE_RELATIVE_COUNTDOWN) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Unsupported or unknown clock mode: " + clockMode);
        }

        // Count time tokens (HH:mm:ss). Multiple time tokens fail closed due to ambiguity
        int tokenCount = countTimeTokens(caption);
        if (tokenCount == 0) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "No valid time token (HH:mm:ss) found in caption");
        }
        if (tokenCount > 1) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Ambiguous caption: multiple time tokens detected (" + tokenCount + ")");
        }

        // Verify zone matching in scheduled row
        if (zones.size() != 1) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Zone required in scheduled row");
        }
        int foundZone = zones.get(0).zone;
        if (foundZone != expectedZone) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Zone mismatch: found " + foundZone + ", expected " + expectedZone);
        }

        int firstTimeIdx = firstTime(caption);
        if (firstTimeIdx < 0) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Failed to locate time token index");
        }

        // Extract hour, minute, second
        int h;
        int m;
        int s;
        try {
            h = Integer.parseInt(caption.substring(firstTimeIdx, firstTimeIdx + 2));
            m = Integer.parseInt(caption.substring(firstTimeIdx + 3, firstTimeIdx + 5));
            s = Integer.parseInt(caption.substring(firstTimeIdx + 6, firstTimeIdx + 8));
        } catch (Exception e) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Malformed time digits: " + e.getMessage());
        }

        if (m < 0 || m > 59) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Minute out of range [0..59]: " + m);
        }
        if (s < 0 || s > 59) {
            return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                    "Second out of range [0..59]: " + s);
        }

        long due;
        Status status;

        if (clockMode == CLOCK_MODE_ABSOLUTE_GMT7) {
            if (h < 0 || h > 23) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Hour out of range [0..23] for absolute mode: " + h);
            }

            // Calculate timestamp in GMT+07:00 independent of host timezone
            Calendar cal = Calendar.getInstance(TZ_GMT7);
            cal.setTime(new Date(nowEpochMillis));
            cal.set(Calendar.HOUR_OF_DAY, h);
            cal.set(Calendar.MINUTE, m);
            cal.set(Calendar.SECOND, s);
            cal.set(Calendar.MILLISECOND, 0);

            due = cal.getTime().getTime();
            if (due < 0L) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Invalid negative due timestamp calculated: " + due);
            }

            // Rollover logic: if due is more than 60 seconds in the past today, it is scheduled for tomorrow
            if (due < nowEpochMillis - 60000L) {
                if (due > Long.MAX_VALUE - 86400000L) {
                    return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                            "Epoch overflow during midnight rollover");
                }
                due += 86400000L; // +24 hours
                if (due < 0L) {
                    return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                            "Invalid negative due timestamp after rollover: " + due);
                }
                status = Status.SCHEDULED;
            } else if (due <= nowEpochMillis) {
                // Within the past 60 seconds (due <= now && due >= now - 60s): boss has spawned / arrived
                status = Status.SPAWNED_OR_PAST;
            } else {
                status = Status.SCHEDULED;
            }
        } else {
            // CLOCK_MODE_RELATIVE_COUNTDOWN
            if (h < 0) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Hour cannot be negative in countdown: " + h);
            }
            if (h > 99) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Countdown hour exceeds safe maximum (99): " + h);
            }

            long durationMillis = (h * 3600L + m * 60L + s) * 1000L;

            // B-06: Guard relative countdown addition against overflow
            if (nowEpochMillis > Long.MAX_VALUE - durationMillis) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Countdown epoch overflow");
            }

            due = nowEpochMillis + durationMillis;
            if (due < 0L) {
                return new Result(Status.UNKNOWN, expectedBoss, expectedZone, null, null, caption,
                        "Invalid negative countdown epoch: " + due);
            }

            if (durationMillis == 0L) {
                status = Status.SPAWNED_OR_PAST;
            } else {
                status = Status.SCHEDULED;
            }
        }

        // Extract map alias if present
        int zoneStartIdx = zoneSuffixStart(caption);
        String mapAlias = mapInRow(caption, firstTimeIdx, zoneStartIdx);

        return new Result(status, expectedBoss, expectedZone, Long.valueOf(due), mapAlias, caption,
                "Parsed successfully");
    }

    /**
     * Checks if any colon adjacent to digits represents a malformed time token
     * (e.g. 123:45:00, 12:45:006, 12:345:00, a12:45:00, 12:45:00z).
     */
    public static boolean hasMalformedTimeToken(String str) {
        if (str == null) {
            return false;
        }
        int len = str.length();
        int i = 0;
        while (i < len) {
            char c = str.charAt(i);
            if (c == ':') {
                boolean prevDigit = (i > 0 && Character.isDigit(str.charAt(i - 1)));
                boolean nextDigit = (i + 1 < len && Character.isDigit(str.charAt(i + 1)));
                if (prevDigit && nextDigit) {
                    int start = i;
                    while (start > 0) {
                        char p = str.charAt(start - 1);
                        if (Character.isLetterOrDigit(p) || p == ':') {
                            start--;
                        } else {
                            break;
                        }
                    }
                    int end = i + 1;
                    while (end < len) {
                        char n = str.charAt(end);
                        if (Character.isLetterOrDigit(n) || n == ':') {
                            end++;
                        } else {
                            break;
                        }
                    }
                    // Valid HH:mm:ss must be length 8, chars 2 and 5 are ':', other 6 are digits
                    if (end - start != 8) {
                        return true;
                    }
                    if (str.charAt(start + 2) != ':' || str.charAt(start + 5) != ':') {
                        return true;
                    }
                    for (int j = 0; j < 8; j++) {
                        if (j == 2 || j == 5) {
                            continue;
                        }
                        char tc = str.charAt(start + j);
                        if (tc < '0' || tc > '9') {
                            return true;
                        }
                    }
                    i = end;
                    continue;
                }
            }
            i++;
        }
        return false;
    }

    /**
     * Finds the index of the first HH:mm:ss occurrence (8 characters).
     * Enforces token boundaries on both sides (B-07, B-08).
     */
    public static int firstTime(String str) {
        if (str == null) {
            return -1;
        }
        int len = str.length();
        for (int i = 0; i + 7 < len; i++) {
            if (str.charAt(i + 2) == ':' && str.charAt(i + 5) == ':') {
                // Check boundary before
                if (i > 0) {
                    char prev = str.charAt(i - 1);
                    if (Character.isLetterOrDigit(prev) || prev == ':') {
                        continue;
                    }
                }
                // Check boundary after
                if (i + 8 < len) {
                    char next = str.charAt(i + 8);
                    if (Character.isLetterOrDigit(next) || next == ':') {
                        continue;
                    }
                }
                boolean allDigits = true;
                for (int j = 0; j < 8; j++) {
                    if (j == 2 || j == 5) {
                        continue;
                    }
                    char c = str.charAt(i + j);
                    if (c < '0' || c > '9') {
                        allDigits = false;
                        break;
                    }
                }
                if (allDigits) {
                    return i;
                }
            }
        }
        return -1;
    }

    /**
     * Counts how many HH:mm:ss patterns are in the string.
     * Enforces token boundaries on both sides (B-07, B-08).
     */
    public static int countTimeTokens(String str) {
        if (str == null) {
            return 0;
        }
        int count = 0;
        int len = str.length();
        int i = 0;
        while (i + 7 < len) {
            if (str.charAt(i + 2) == ':' && str.charAt(i + 5) == ':') {
                boolean validBefore = (i == 0) || (!Character.isLetterOrDigit(str.charAt(i - 1)) && str.charAt(i - 1) != ':');
                boolean validAfter = (i + 8 == len) || (!Character.isLetterOrDigit(str.charAt(i + 8)) && str.charAt(i + 8) != ':');
                if (validBefore && validAfter) {
                    boolean allDigits = true;
                    for (int j = 0; j < 8; j++) {
                        if (j == 2 || j == 5) {
                            continue;
                        }
                        char c = str.charAt(i + j);
                        if (c < '0' || c > '9') {
                            allDigits = false;
                            break;
                        }
                    }
                    if (allDigits) {
                        count++;
                        i += 8;
                        continue;
                    }
                }
            }
            i++;
        }
        return count;
    }

    /**
     * Scans caption for all zone tokens ("Khu N", "kN").
     * Returns null if any malformed zone token is detected (e.g. trailing letters "Khu 2xyz").
     * Returns List of valid ZoneInfo instances otherwise.
     */
    public static List<ZoneInfo> scanZones(String caption) {
        if (caption == null) {
            return new ArrayList<ZoneInfo>();
        }
        String lower = caption.toLowerCase(Locale.US);
        int len = lower.length();
        List<ZoneInfo> zones = new ArrayList<ZoneInfo>();

        // 1. Scan for "khu" tokens
        int idx = 0;
        while (idx < len) {
            int khuIdx = lower.indexOf("khu", idx);
            if (khuIdx < 0) {
                break;
            }
            boolean validBefore = (khuIdx == 0) || !Character.isLetterOrDigit(lower.charAt(khuIdx - 1));
            if (validBefore) {
                int p = khuIdx + 3;
                while (p < len && (lower.charAt(p) == ' ' || lower.charAt(p) == ':')) {
                    p++;
                }
                if (p < len && Character.isDigit(lower.charAt(p))) {
                    int digitStart = p;
                    while (p < len && Character.isDigit(lower.charAt(p))) {
                        p++;
                    }
                    // B-10: Trailing letter after zone digits is invalid (e.g. "Khu 2xyz", "Khu 2a")
                    // B-12: Ambiguous numeric/subzone suffix (e.g. "Khu 2_3", "Khu 2.3", "Khu 2-3")
                    if (p < len) {
                        char c = lower.charAt(p);
                        if (Character.isLetter(c)) {
                            return null;
                        }
                        if (c == '_' || ((c == '.' || c == '-' || c == '/') && p + 1 < len && Character.isLetterOrDigit(lower.charAt(p + 1)))) {
                            return null;
                        }
                    }
                    try {
                        int zVal = Integer.parseInt(lower.substring(digitStart, p));
                        zones.add(new ZoneInfo(zVal, khuIdx, p));
                    } catch (Exception e) {
                        return null;
                    }
                    idx = p;
                    continue;
                }
            }
            idx = khuIdx + 3;
        }

        // 2. Scan for standalone 'k' tokens followed by digits
        idx = 0;
        while (idx < len) {
            int kIdx = lower.indexOf('k', idx);
            if (kIdx < 0) {
                break;
            }
            // Skip if this 'k' is part of "khu"
            if (lower.startsWith("khu", kIdx)) {
                idx = kIdx + 3;
                continue;
            }
            boolean validBefore = (kIdx == 0) || !Character.isLetterOrDigit(lower.charAt(kIdx - 1));
            if (validBefore) {
                int p = kIdx + 1;
                while (p < len && (lower.charAt(p) == ' ' || lower.charAt(p) == ':')) {
                    p++;
                }
                if (p < len && Character.isDigit(lower.charAt(p))) {
                    int digitStart = p;
                    while (p < len && Character.isDigit(lower.charAt(p))) {
                        p++;
                    }
                    // B-10: Trailing letter after zone digits is invalid (e.g. "k2xyz")
                    // B-12: Ambiguous numeric/subzone suffix (e.g. "k2_3", "k2.3", "k2-3")
                    if (p < len) {
                        char c = lower.charAt(p);
                        if (Character.isLetter(c)) {
                            return null;
                        }
                        if (c == '_' || ((c == '.' || c == '-' || c == '/') && p + 1 < len && Character.isLetterOrDigit(lower.charAt(p + 1)))) {
                            return null;
                        }
                    }
                    try {
                        int zVal = Integer.parseInt(lower.substring(digitStart, p));
                        zones.add(new ZoneInfo(zVal, kIdx, p));
                    } catch (Exception e) {
                        return null;
                    }
                    idx = p;
                    continue;
                }
            }
            idx = kIdx + 1;
        }

        return zones;
    }

    /**
     * Finds the starting index in raw caption of the zone token (e.g. "Khu 1", "k2").
     * Returns -1 if no zone or multiple/ambiguous zones exist.
     */
    public static int zoneSuffixStart(String caption) {
        if (caption == null) {
            return -1;
        }
        List<ZoneInfo> zones = scanZones(caption);
        if (zones == null || zones.size() != 1) {
            return -1;
        }
        return zones.get(0).startIndex;
    }

    /**
     * Extracts map alias located between firstTime (+8) and zoneSuffixStart.
     */
    public static String mapInRow(String caption, int firstTimeIdx, int zoneStartIdx) {
        if (caption == null || firstTimeIdx < 0 || zoneStartIdx < 0 || zoneStartIdx <= firstTimeIdx + 8) {
            return null;
        }
        String mapPart = caption.substring(firstTimeIdx + 8, zoneStartIdx).trim();

        // Strip leading delimiters
        while (mapPart.length() > 0) {
            char c = mapPart.charAt(0);
            if (c == '.' || c == '-' || c == ':' || c == ',' || c <= ' ') {
                mapPart = mapPart.substring(1).trim();
            } else {
                break;
            }
        }

        // Strip trailing delimiters
        while (mapPart.length() > 0) {
            char c = mapPart.charAt(mapPart.length() - 1);
            if (c == '.' || c == '-' || c == ':' || c == ',' || c <= ' ') {
                mapPart = mapPart.substring(0, mapPart.length() - 1).trim();
            } else {
                break;
            }
        }

        if (mapPart.length() == 0) {
            return null;
        }
        return mapPart;
    }

    /**
     * Extracts map alias from a spawned status caption without Unicode index misalignment.
     * Operates purely on raw indices of rawCaption.
     */
    private static String extractMapFromSpawnedCaption(String rawCaption, int zone) {
        int zoneIdx = zoneSuffixStart(rawCaption);
        if (zoneIdx <= 0) {
            return null;
        }
        String beforeZone = rawCaption.substring(0, zoneIdx).trim();

        // Delimiter separating boss name from map
        int firstDash = beforeZone.indexOf('-');
        if (firstDash < 0) {
            return null;
        }

        String mapPart = beforeZone.substring(firstDash + 1).trim();

        // Clean delimiters
        while (mapPart.length() > 0) {
            char c = mapPart.charAt(0);
            if (c == '.' || c == '-' || c == ':' || c == ',' || c <= ' ') {
                mapPart = mapPart.substring(1).trim();
            } else {
                break;
            }
        }
        while (mapPart.length() > 0) {
            char c = mapPart.charAt(mapPart.length() - 1);
            if (c == '.' || c == '-' || c == ':' || c == ',' || c <= ' ') {
                mapPart = mapPart.substring(0, mapPart.length() - 1).trim();
            } else {
                break;
            }
        }

        return mapPart.length() > 0 ? mapPart : null;
    }

    /**
     * Strips Vietnamese accents and lowercases using Locale.US (B-11).
     * Java 6 clean manual lookup.
     */
    public static String norm(String value) {
        if (value == null) {
            return "";
        }
        String source = "àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡ"
                + "ùúụủũưừứựửữỳýỵỷỹđ"
                + "ÀÁẠẢÃÂẦẤẬẨẪĂẰẮẶẲẴÈÉẸẺẼÊỀẾỆỂỄÌÍỊỈĨÒÓỌỎÕÔỒỐỘỔỖƠỜỚỢỞỠ"
                + "ÙÚỤỦŨƯỪỨỰỬỮỲÝỴỶỸĐ";
        String target = "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooo"
                + "uuuuuuuuuuuyyyyyd"
                + "aaaaaaaaaaaaaaaaaeeeeeeeeeeeiiiiiooooooooooooooooo"
                + "uuuuuuuuuuuyyyyyd";
        String lower = value.toLowerCase(Locale.US);
        StringBuffer out = new StringBuffer(lower.length());
        for (int i = 0; i < lower.length(); i++) {
            char c = lower.charAt(i);
            int at = source.indexOf(c);
            if (at >= 0 && at < target.length()) {
                out.append(target.charAt(at));
            } else {
                out.append(c);
            }
        }
        return out.toString();
    }

    /**
     * Strips accents, lowercases (Locale.US), and collapses whitespace.
     */
    public static String normSemantic(String value) {
        if (value == null) {
            return "";
        }
        String stripped = norm(value).trim();
        StringBuffer out = new StringBuffer(stripped.length());
        boolean lastSpace = false;
        for (int i = 0; i < stripped.length(); i++) {
            char c = stripped.charAt(i);
            if (c <= ' ') {
                if (!lastSpace && out.length() > 0) {
                    out.append(' ');
                    lastSpace = true;
                }
            } else {
                out.append(c);
                lastSpace = false;
            }
        }
        return out.toString();
    }
}
