import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Persistent configuration for Zeus Boss Hunt.
 * Implements BH-01 (boss selection), BH-02 (zone 1..5 masks), BH-03 (manual map override),
 * BH-05 (clock mode options), and BH-10 (persistence).
 *
 * Defaults:
 * - enabled: false (FAIL-CLOSED, explicit operator opt-in only)
 * - clockMode: 1 (MINH_COMPAT_ABSOLUTE_GMT7)
 * - zones: 0x1F (all zones 1..5 selected)
 * - bossMaps: null, except boss 5 ("D?a Ng?c T?ng 1")
 *
 * Fully Java 6 compatible (major 50).
 */
public final class BossHuntConfig {

    public static final int CLOCK_MODE_MINH_ABSOLUTE_GMT7 = 1;
    public static final int CLOCK_MODE_RELATIVE_COUNTDOWN = 2;

    public static final String DEFAULT_CONFIG_FILENAME = "zeus-boss-hunt-config.json";

    private boolean enabled;
    private int clockMode;
    private final int[] zones;
    private final String[] bossMaps;
    private final long[] travelBudget;

    public BossHuntConfig() {
        this.enabled = false;
        this.clockMode = CLOCK_MODE_MINH_ABSOLUTE_GMT7;
        this.zones = new int[BossCatalog.BOSS_COUNT];
        for (int i = 0; i < BossCatalog.BOSS_COUNT; i++) {
            this.zones[i] = 0x1F; // all 5 zones (1..5) enabled by default
        }
        this.bossMaps = new String[BossCatalog.BOSS_COUNT];
        this.bossMaps[5] = BossCatalog.getDefaultMap(5);
        this.travelBudget = new long[BossCatalog.BOSS_COUNT];
    }

    public boolean isEnabled() {
        return enabled;
    }

    public void setEnabled(boolean enabled) {
        this.enabled = enabled;
    }

    public int getClockMode() {
        return clockMode;
    }

    public void setClockMode(int mode) {
        if (mode == CLOCK_MODE_MINH_ABSOLUTE_GMT7 || mode == CLOCK_MODE_RELATIVE_COUNTDOWN) {
            this.clockMode = mode;
        }
    }

    public int getZoneMask(int bossId) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT) {
            return zones[bossId] & 0x1F;
        }
        return 0;
    }

    public void setZoneMask(int bossId, int mask) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT) {
            zones[bossId] = mask & 0x1F;
        }
    }

    public boolean isZoneSelected(int bossId, int zone) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT && zone >= 1 && zone <= 5) {
            return (zones[bossId] & (1 << (zone - 1))) != 0;
        }
        return false;
    }

    public void setZoneSelected(int bossId, int zone, boolean selected) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT && zone >= 1 && zone <= 5) {
            if (selected) {
                zones[bossId] |= (1 << (zone - 1));
            } else {
                zones[bossId] &= ~(1 << (zone - 1));
            }
            zones[bossId] &= 0x1F;
        }
    }

    public void toggleZone(int bossId, int zone) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT && zone >= 1 && zone <= 5) {
            zones[bossId] ^= (1 << (zone - 1));
            zones[bossId] &= 0x1F;
        }
    }

    public String getMapOverride(int bossId) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT) {
            return bossMaps[bossId];
        }
        return null;
    }

    public void setMapOverride(int bossId, String map) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT) {
            if (map != null && map.trim().length() == 0) {
                map = null;
            }
            bossMaps[bossId] = map;
        }
    }

    public String getEffectiveMap(int bossId, String scheduleMap) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT) {
            if (bossMaps[bossId] != null) {
                return bossMaps[bossId];
            }
            if (scheduleMap != null && scheduleMap.trim().length() > 0) {
                return scheduleMap;
            }
            return BossCatalog.getDefaultMap(bossId);
        }
        return scheduleMap;
    }

    public long getTravelBudget(int bossId) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT) {
            return travelBudget[bossId];
        }
        return 0L;
    }

    public void setTravelBudget(int bossId, long budget) {
        if (bossId >= 0 && bossId < BossCatalog.BOSS_COUNT) {
            travelBudget[bossId] = Math.max(0L, budget);
        }
    }

    public String toJson() {
        StringBuffer sb = new StringBuffer(1024);
        sb.append("{\n");
        sb.append("  \"enabled\": ").append(enabled).append(",\n");
        sb.append("  \"clockMode\": ").append(clockMode).append(",\n");
        sb.append("  \"bosses\": [\n");
        for (int i = 0; i < BossCatalog.BOSS_COUNT; i++) {
            if (i > 0) {
                sb.append(",\n");
            }
            sb.append("    {\n");
            sb.append("      \"id\": ").append(i).append(",\n");
            sb.append("      \"name\": ");
            escapeJson(BossCatalog.getBossName(i), sb);
            sb.append(",\n");
            sb.append("      \"zoneMask\": ").append(zones[i] & 0x1F).append(",\n");
            sb.append("      \"mapOverride\": ");
            escapeJson(bossMaps[i], sb);
            sb.append(",\n");
            sb.append("      \"travelBudget\": ").append(travelBudget[i]).append("\n");
            sb.append("    }");
        }
        sb.append("\n  ]\n");
        sb.append("}\n");
        return sb.toString();
    }

    public static BossHuntConfig fromJson(String json) {
        BossHuntConfig cfg = new BossHuntConfig();
        if (json == null || json.trim().length() == 0) {
            return cfg;
        }

        try {
            int[] pos = new int[]{0};
            skipWhitespace(json, pos);
            if (pos[0] >= json.length() || json.charAt(pos[0]) != '{') {
                return cfg; // Root must be a JSON object
            }
            Object rootObj = parseJsonValue(json, pos);
            skipWhitespace(json, pos);
            if (pos[0] < json.length()) {
                // Trailing characters after root JSON object: fail closed!
                return cfg;
            }
            if (!(rootObj instanceof Map)) {
                return cfg;
            }
            Map<?, ?> root = (Map<?, ?>) rootObj;

            // Strict root keys check
            for (Object k : root.keySet()) {
                if (!(k instanceof String)) {
                    return new BossHuntConfig();
                }
                String ks = (String) k;
                if (!ks.equals("enabled") && !ks.equals("clockMode") && !ks.equals("bosses")) {
                    return new BossHuntConfig();
                }
            }

            // Strict enabled check (must be boolean)
            Object enabledVal = root.get("enabled");
            if (enabledVal instanceof Boolean) {
                cfg.enabled = ((Boolean) enabledVal).booleanValue();
            } else if (enabledVal != null) {
                return new BossHuntConfig();
            }

            // ClockMode check (must be strict integer token, no floating/fraction)
            Object clockVal = root.get("clockMode");
            if (clockVal instanceof Long || clockVal instanceof Integer) {
                int cm = ((Number) clockVal).intValue();
                if (cm != CLOCK_MODE_MINH_ABSOLUTE_GMT7 && cm != CLOCK_MODE_RELATIVE_COUNTDOWN) {
                    return new BossHuntConfig();
                }
                cfg.setClockMode(cm);
            } else if (clockVal != null) {
                return new BossHuntConfig();
            }

            // Bosses array check
            Object bossesVal = root.get("bosses");
            if (bossesVal instanceof List) {
                List<?> list = (List<?>) bossesVal;
                for (int i = 0; i < list.size(); i++) {
                    Object item = list.get(i);
                    if (!(item instanceof Map)) {
                        return new BossHuntConfig();
                    }
                    Map<?, ?> bMap = (Map<?, ?>) item;
                    for (Object bk : bMap.keySet()) {
                        if (!(bk instanceof String)) {
                            return new BossHuntConfig();
                        }
                        String bks = (String) bk;
                        if (!bks.equals("id") && !bks.equals("name") && !bks.equals("zoneMask") && !bks.equals("zonesMask") && !bks.equals("mapOverride") && !bks.equals("travelBudget")) {
                            return new BossHuntConfig();
                        }
                    }
                    if (bMap.containsKey("zoneMask") && bMap.containsKey("zonesMask")) {
                        return new BossHuntConfig();
                    }
                    Object nameObj = bMap.get("name");
                    if (nameObj != null && !(nameObj instanceof String)) {
                        return new BossHuntConfig();
                    }
                    Object idObj = bMap.get("id");
                    if (!(idObj instanceof Long) && !(idObj instanceof Integer)) {
                        return new BossHuntConfig();
                    }
                    int id = ((Number) idObj).intValue();
                    if (id < 0 || id >= BossCatalog.BOSS_COUNT) {
                        return new BossHuntConfig();
                    }
                    Object zmObj = bMap.get("zoneMask");
                    if (zmObj == null) {
                        zmObj = bMap.get("zonesMask");
                    }
                    if (zmObj instanceof Long || zmObj instanceof Integer) {
                        int zm = ((Number) zmObj).intValue();
                        if (zm < 0 || zm > 0x1F) {
                            return new BossHuntConfig();
                        }
                        cfg.setZoneMask(id, zm);
                    } else if (zmObj != null) {
                        return new BossHuntConfig();
                    }
                    Object moObj = bMap.get("mapOverride");
                    if (moObj instanceof String) {
                        cfg.setMapOverride(id, (String) moObj);
                    } else if (moObj != null) {
                        return new BossHuntConfig();
                    }
                    Object tbObj = bMap.get("travelBudget");
                    if (tbObj instanceof Long || tbObj instanceof Integer) {
                        long tb = ((Number) tbObj).longValue();
                        if (tb < 0) {
                            return new BossHuntConfig();
                        }
                        cfg.setTravelBudget(id, tb);
                    } else if (tbObj != null) {
                        return new BossHuntConfig();
                    }
                }
            } else if (bossesVal != null) {
                return new BossHuntConfig();
            }
        } catch (Throwable t) {
            // Any parse error, syntax error, EOF, or schema violation -> fail closed (default disabled)
            return new BossHuntConfig();
        }

        return cfg;
    }

    private static void skipWhitespace(String s, int[] pos) {
        int len = s.length();
        while (pos[0] < len) {
            char c = s.charAt(pos[0]);
            if (c == ' ' || c == '\t' || c == '\n' || c == '\r') {
                pos[0]++;
            } else {
                break;
            }
        }
    }

    private static Object parseJsonValue(String s, int[] pos) throws Exception {
        skipWhitespace(s, pos);
        if (pos[0] >= s.length()) {
            throw new Exception("Unexpected EOF");
        }
        char c = s.charAt(pos[0]);
        if (c == '{') {
            return parseJsonObject(s, pos);
        } else if (c == '[') {
            return parseJsonArray(s, pos);
        } else if (c == '"') {
            return parseJsonString(s, pos);
        } else if (c == 't' || c == 'f') {
            return parseJsonBoolean(s, pos);
        } else if (c == 'n') {
            return parseJsonNull(s, pos);
        } else if (c == '-' || (c >= '0' && c <= '9')) {
            return parseJsonNumber(s, pos);
        }
        throw new Exception("Unexpected char: " + c);
    }

    private static Map<String, Object> parseJsonObject(String s, int[] pos) throws Exception {
        if (s.charAt(pos[0]) != '{') throw new Exception("Expected '{'");
        pos[0]++;
        Map<String, Object> map = new HashMap<String, Object>();
        skipWhitespace(s, pos);
        if (pos[0] < s.length() && s.charAt(pos[0]) == '}') {
            pos[0]++;
            return map;
        }
        while (true) {
            skipWhitespace(s, pos);
            if (pos[0] >= s.length() || s.charAt(pos[0]) != '"') {
                throw new Exception("Expected string key");
            }
            String key = parseJsonString(s, pos);
            skipWhitespace(s, pos);
            if (pos[0] >= s.length() || s.charAt(pos[0]) != ':') {
                throw new Exception("Expected ':'");
            }
            pos[0]++;
            Object val = parseJsonValue(s, pos);
            if (map.containsKey(key)) {
                throw new Exception("Duplicate key in object: " + key);
            }
            map.put(key, val);
            skipWhitespace(s, pos);
            if (pos[0] >= s.length()) {
                throw new Exception("Unexpected EOF in object");
            }
            char c = s.charAt(pos[0]);
            if (c == '}') {
                pos[0]++;
                break;
            } else if (c == ',') {
                pos[0]++;
            } else {
                throw new Exception("Expected ',' or '}'");
            }
        }
        return map;
    }

    private static List<Object> parseJsonArray(String s, int[] pos) throws Exception {
        if (s.charAt(pos[0]) != '[') throw new Exception("Expected '['");
        pos[0]++;
        List<Object> list = new ArrayList<Object>();
        skipWhitespace(s, pos);
        if (pos[0] < s.length() && s.charAt(pos[0]) == ']') {
            pos[0]++;
            return list;
        }
        while (true) {
            Object val = parseJsonValue(s, pos);
            list.add(val);
            skipWhitespace(s, pos);
            if (pos[0] >= s.length()) {
                throw new Exception("Unexpected EOF in array");
            }
            char c = s.charAt(pos[0]);
            if (c == ']') {
                pos[0]++;
                break;
            } else if (c == ',') {
                pos[0]++;
            } else {
                throw new Exception("Expected ',' or ']'");
            }
        }
        return list;
    }

    private static String parseJsonString(String s, int[] pos) throws Exception {
        if (s.charAt(pos[0]) != '"') throw new Exception("Expected '\"'");
        pos[0]++;
        StringBuffer sb = new StringBuffer();
        int len = s.length();
        while (pos[0] < len) {
            char c = s.charAt(pos[0]++);
            if (c == '"') {
                return sb.toString();
            } else if (c == '\\') {
                if (pos[0] >= len) throw new Exception("Unterminated escape");
                char esc = s.charAt(pos[0]++);
                if (esc == '"' || esc == '\\' || esc == '/') {
                    sb.append(esc);
                } else if (esc == 'b') {
                    sb.append('\b');
                } else if (esc == 'f') {
                    sb.append('\f');
                } else if (esc == 'n') {
                    sb.append('\n');
                } else if (esc == 'r') {
                    sb.append('\r');
                } else if (esc == 't') {
                    sb.append('\t');
                } else if (esc == 'u') {
                    if (pos[0] + 4 > len) throw new Exception("Invalid unicode escape");
                    String hex = s.substring(pos[0], pos[0] + 4);
                    pos[0] += 4;
                    sb.append((char) Integer.parseInt(hex, 16));
                } else {
                    sb.append(esc);
                }
            } else {
                sb.append(c);
            }
        }
        throw new Exception("Unterminated string");
    }

    private static Boolean parseJsonBoolean(String s, int[] pos) throws Exception {
        if (pos[0] + 4 <= s.length() && s.substring(pos[0], pos[0] + 4).equals("true")) {
            pos[0] += 4;
            return Boolean.TRUE;
        } else if (pos[0] + 5 <= s.length() && s.substring(pos[0], pos[0] + 5).equals("false")) {
            pos[0] += 5;
            return Boolean.FALSE;
        }
        throw new Exception("Invalid boolean");
    }

    private static Object parseJsonNull(String s, int[] pos) throws Exception {
        if (pos[0] + 4 <= s.length() && s.substring(pos[0], pos[0] + 4).equals("null")) {
            pos[0] += 4;
            return null;
        }
        throw new Exception("Invalid null");
    }

    private static Number parseJsonNumber(String s, int[] pos) throws Exception {
        int start = pos[0];
        int len = s.length();
        if (s.charAt(pos[0]) == '-') pos[0]++;
        if (pos[0] >= len || !Character.isDigit(s.charAt(pos[0]))) {
            throw new Exception("Invalid number");
        }
        while (pos[0] < len && Character.isDigit(s.charAt(pos[0]))) {
            pos[0]++;
        }
        boolean isFloat = false;
        if (pos[0] < len && s.charAt(pos[0]) == '.') {
            isFloat = true;
            pos[0]++;
            if (pos[0] >= len || !Character.isDigit(s.charAt(pos[0]))) {
                throw new Exception("Invalid number fraction");
            }
            while (pos[0] < len && Character.isDigit(s.charAt(pos[0]))) {
                pos[0]++;
            }
        }
        if (pos[0] < len && (s.charAt(pos[0]) == 'e' || s.charAt(pos[0]) == 'E')) {
            isFloat = true;
            pos[0]++;
            if (pos[0] < len && (s.charAt(pos[0]) == '+' || s.charAt(pos[0]) == '-')) {
                pos[0]++;
            }
            if (pos[0] >= len || !Character.isDigit(s.charAt(pos[0]))) {
                throw new Exception("Invalid number exponent");
            }
            while (pos[0] < len && Character.isDigit(s.charAt(pos[0]))) {
                pos[0]++;
            }
        }
        String numStr = s.substring(start, pos[0]);
        if (isFloat) {
            return Double.valueOf(numStr);
        }
        return Long.valueOf(numStr);
    }

    public void copyFrom(BossHuntConfig other) {
        if (other == null) {
            return;
        }
        this.enabled = other.enabled;
        this.clockMode = other.clockMode;
        for (int i = 0; i < BossCatalog.BOSS_COUNT; i++) {
            this.zones[i] = other.zones[i];
            this.bossMaps[i] = other.bossMaps[i];
            this.travelBudget[i] = other.travelBudget[i];
        }
    }

    public void reloadFromFile(String path) {
        BossHuntConfig loaded = loadFromFile(path);
        copyFrom(loaded);
    }

    public static BossHuntConfig loadFromFile(String path) {
        if (path == null) {
            return new BossHuntConfig();
        }
        File f = new File(path);
        if (!f.exists() || f.length() == 0 || f.length() > 64 * 1024) {
            return new BossHuntConfig();
        }
        FileInputStream fis = null;
        try {
            byte[] buf = new byte[(int) f.length()];
            fis = new FileInputStream(f);
            int read = 0;
            while (read < buf.length) {
                int r = fis.read(buf, read, buf.length - read);
                if (r < 0) break;
                read += r;
            }
            fis.close();
            fis = null;
            return fromJson(new String(buf, 0, read, "UTF-8"));
        } catch (Throwable t) {
            return new BossHuntConfig();
        } finally {
            if (fis != null) {
                try { fis.close(); } catch (Throwable ignored) {}
            }
        }
    }

    public void saveToFile(String path) {
        if (path == null) {
            return;
        }
        FileOutputStream fos = null;
        File temp = null;
        try {
            String json = toJson();
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
