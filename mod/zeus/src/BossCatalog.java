/**
 * Deterministic Boss Catalog based on ModByTminh v4.0.3 reference.
 * Defines 10 canonical boss definitions, lookup by ID/name, and default map mappings.
 *
 * Fully Java 6 compatible (major 50).
 */
public final class BossCatalog {

    public static final int BOSS_COUNT = 10;

    public static final String[] BOSSES = new String[] {
        "D\u00ea B\u1ea1c",           // 0: Dê Bạc
        "D\u00ea V\u00e0ng",          // 1: Dê Vàng
        "X\u00e0 n\u1eef",            // 2: Xà nữ
        "Qu\u1ef7 m\u1ed9t m\u1eaft", // 3: Quỷ một mắt
        "Qu\u1ef7 \u0111\u1ea7u b\u00f2", // 4: Quỷ đầu bò
        "K\u1ef5 s\u0129 \u0111\u1ecba ng\u1ee5c", // 5: Kỵ sĩ địa ngục
        "Nh\u1ec7n ch\u00faa",        // 6: Nhện chúa
        "Giant Skeleton",             // 7: Giant Skeleton
        "Th\u1ecf tr\u1eafng",        // 8: Thỏ trắng
        "M\u00e8o m\u1eadp"           // 9: Mèo mập
    };

    private static final String DEFAULT_MAP_BOSS_5 = "\u0110\u1ecba Ng\u1ee5c T\u1ea7ng 1";

    private BossCatalog() {
    }

    public static String getBossName(int id) {
        if (id >= 0 && id < BOSS_COUNT) {
            return BOSSES[id];
        }
        return null;
    }

    public static String getDefaultMap(int id) {
        if (id == 5) {
            return DEFAULT_MAP_BOSS_5;
        }
        return null;
    }

    public static int findBossId(String name) {
        if (name == null) {
            return -1;
        }
        String clean = name.trim();
        if (clean.length() == 0) {
            return -1;
        }

        // 1. Exact match
        for (int i = 0; i < BOSS_COUNT; i++) {
            if (BOSSES[i].equals(clean)) {
                return i;
            }
        }

        // 2. Case-insensitive match
        for (int i = 0; i < BOSS_COUNT; i++) {
            if (BOSSES[i].equalsIgnoreCase(clean)) {
                return i;
            }
        }

        // 3. Normalized diacritics match
        String normTarget = normalize(clean);
        for (int i = 0; i < BOSS_COUNT; i++) {
            if (normalize(BOSSES[i]).equals(normTarget)) {
                return i;
            }
        }

        return -1;
    }

    public static String normalize(String s) {
        if (s == null) {
            return "";
        }
        StringBuffer sb = new StringBuffer(s.length());
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '\u00e0': case '\u00e1': case '\u1ea1': case '\u1ea3': case '\u00e3':
                case '\u00e2': case '\u1ea7': case '\u1ea5': case '\u1ead': case '\u1ea9': case '\u1eab':
                case '\u0103': case '\u1eb1': case '\u1eaf': case '\u1eb7': case '\u1eb3': case '\u1eb5':
                case '\u00c0': case '\u00c1': case '\u1ea0': case '\u1ea2': case '\u00c3':
                case '\u00c2': case '\u1ea6': case '\u1ea4': case '\u1eac': case '\u1ea8': case '\u1eaa':
                case '\u0102': case '\u1eb0': case '\u1eae': case '\u1eb6': case '\u1eb2': case '\u1eb4':
                    sb.append('a'); break;
                case '\u0111': case '\u0110':
                    sb.append('d'); break;
                case '\u00e8': case '\u00e9': case '\u1eb9': case '\u1ebb': case '\u1ebd':
                case '\u00ea': case '\u1ec1': case '\u1ebf': case '\u1ec7': case '\u1ec3': case '\u1ec5':
                case '\u00c8': case '\u00c9': case '\u1eb8': case '\u1eba': case '\u1ebc':
                case '\u00ca': case '\u1ec0': case '\u1ebe': case '\u1ec6': case '\u1ec2': case '\u1ec4':
                    sb.append('e'); break;
                case '\u00ec': case '\u00ed': case '\u1ecb': case '\u1ec9': case '\u0129':
                case '\u00cc': case '\u00cd': case '\u1eca': case '\u1ec8': case '\u0128':
                    sb.append('i'); break;
                case '\u00f2': case '\u00f3': case '\u1ecd': case '\u1ecf': case '\u00f5':
                case '\u00f4': case '\u1ed3': case '\u1ed1': case '\u1ed9': case '\u1ed5': case '\u1ed7':
                case '\u01a1': case '\u1edd': case '\u1edb': case '\u1ee3': case '\u1edf': case '\u1ee1':
                case '\u00d2': case '\u00d3': case '\u1ecc': case '\u1ece': case '\u00d5':
                case '\u00d4': case '\u1ed2': case '\u1ed0': case '\u1ed8': case '\u1ed4': case '\u1ed6':
                case '\u01a0': case '\u1edc': case '\u1eda': case '\u1ee2': case '\u1ede': case '\u1ee0':
                    sb.append('o'); break;
                case '\u00f9': case '\u00fa': case '\u1ee5': case '\u1ee7': case '\u0169':
                case '\u01b0': case '\u1eeb': case '\u1ee9': case '\u1ef1': case '\u1eed': case '\u1eef':
                case '\u00d9': case '\u00da': case '\u1ee4': case '\u1ee6': case '\u0168':
                case '\u01af': case '\u1eea': case '\u1ee8': case '\u1ef0': case '\u1eec': case '\u1eee':
                    sb.append('u'); break;
                case '\u1ef3': case '\u00fd': case '\u1ef5': case '\u1ef7': case '\u1ef9':
                case '\u1ef2': case '\u00dd': case '\u1ef4': case '\u1ef6': case '\u1ef8':
                    sb.append('y'); break;
                default:
                    if (c >= 'A' && c <= 'Z') {
                        sb.append((char) (c + 32));
                    } else {
                        sb.append(c);
                    }
                    break;
            }
        }
        return sb.toString();
    }
}
