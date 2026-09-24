import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.Vector;

public class InventoryCatalogTest {

    static Field f(Class<?> owner, String name) throws Exception {
        Field field = owner.getDeclaredField(name);
        field.setAccessible(true);
        return field;
    }

    static void set(String name, Object value) throws Exception {
        f(Class.forName("Zeus"), name).set(null, value);
    }

    static Object get(String name) throws Exception {
        return f(Class.forName("Zeus"), name).get(null);
    }

    static void call(String name) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod(name);
        m.setAccessible(true);
        m.invoke(null);
    }

    static String formatCatalog() throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod("formatInventoryCatalogJson");
        m.setAccessible(true);
        return (String) m.invoke(null);
    }

    @SuppressWarnings("unchecked")
    static Vector<Object> queue() throws Exception {
        Object link = l.a();
        Field o = f(l.class, "o");
        Object sender = o.get(link);
        Field a = f(sender.getClass(), "a");
        return (Vector<Object>) a.get(sender);
    }

    static int failures = 0;

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static j fullItem(int id, int kind, String name, String baseName, int level, int tier, int count, short durability, byte bind, int icon) {
        j it = new j();
        it.O = id;
        it.u = kind;
        it.g = name;
        it.i = baseName;
        it.z = (byte) level;
        it.N = tier;
        it.K = count;
        it.v = durability;
        it.B = bind;
        it.t = icon;
        return it;
    }

    static et bag(bw... items) {
        et v = new et("bag");
        for (int i = 0; i < items.length; i++) {
            v.a(items[i]);
        }
        return v;
    }

    public static void main(String[] args) throws Exception {
        System.out.println("=== InventoryCatalogTest ===");

        // Precondition: set capacity bq.x = 28
        bq.x = 28;

        // ---------------------------------------------------------------------
        // Test 1: Empty bag serializes a valid versioned catalog
        // ---------------------------------------------------------------------
        System.out.println("--- Test 1: Empty bag ---");
        bw.V = bag();
        String json1 = formatCatalog();
        check("catalog contains version 1", json1.indexOf("\"version\": 1") >= 0);
        check("catalog contains bag_capacity 28", json1.indexOf("\"bag_capacity\": 28") >= 0);
        check("catalog contains empty items array", json1.indexOf("\"items\": []") >= 0);

        // ---------------------------------------------------------------------
        // Test 2: Occupied slots retain their actual slot indexes and fields
        // ---------------------------------------------------------------------
        System.out.println("--- Test 2: Occupied slots and fields ---");
        j equip = fullItem(101, 3, "Kiếm ngắn +5", "Kiếm ngắn", 5, 2, 1, (short) 500, (byte) 1, 12);
        j mount = fullItem(62, 4, "Ngựa trắng", "Ngựa trắng", 0, 1, 1, (short) -1, (byte) 0, 25);
        j potion = fullItem(1234, 6, "Bình HP", "Bình HP", 0, 0, 99, (short) -1, (byte) 0, 35);
        bw.V = bag(equip, mount, potion);

        String json2 = formatCatalog();
        check("slot 0 present", json2.indexOf("\"slot\": 0") >= 0);
        check("slot 1 present", json2.indexOf("\"slot\": 1") >= 0);
        check("slot 2 present", json2.indexOf("\"slot\": 2") >= 0);
        check("equip candidate_for_enhancement true", json2.indexOf("\"candidate_for_enhancement\": true") >= 0);
        check("equip durability 500", json2.indexOf("\"durability\": 500") >= 0);
        check("equip bind 1", json2.indexOf("\"bind\": 1") >= 0);
        check("equip icon 12", json2.indexOf("\"icon\": 12") >= 0);
        check("equip level 5", json2.indexOf("\"level\": 5") >= 0);
        check("equip template_id 101", json2.indexOf("\"template_id\": 101") >= 0);
        check("mount candidate_for_enhancement false", json2.indexOf("\"candidate_for_enhancement\": false") >= 0);
        check("mount durability null", json2.indexOf("\"durability\": null") >= 0);
        check("potion count 99", json2.indexOf("\"count\": 99") >= 0);

        // ---------------------------------------------------------------------
        // Test 3: Two visually identical items in different slots remain distinct
        // ---------------------------------------------------------------------
        System.out.println("--- Test 3: Two identical items remain distinct ---");
        j swordA = fullItem(200, 3, "Đao +0", "Đao", 0, 1, 1, (short) 1000, (byte) 0, 50);
        j placeholder = fullItem(999, 7, "Đá", "Đá", 0, 0, 1, (short) -1, (byte) 0, 1);
        j swordB = fullItem(200, 3, "Đao +0", "Đao", 0, 1, 1, (short) 1000, (byte) 0, 50);
        bw.V = bag(swordA, placeholder, swordB);

        String json3 = formatCatalog();
        check("swordA at slot 0", json3.indexOf("\"slot\": 0") >= 0);
        check("placeholder at slot 1", json3.indexOf("\"slot\": 1") >= 0);
        check("swordB at slot 2", json3.indexOf("\"slot\": 2") >= 0);
        check("both sword templates present", json3.indexOf("\"template_id\": 200") != json3.lastIndexOf("\"template_id\": 200"));

        // ---------------------------------------------------------------------
        // Test 4: Enhancement level is serialized as mutable state
        // ---------------------------------------------------------------------
        System.out.println("--- Test 4: Enhancement level mutation ---");
        swordA.z = 7;
        swordA.g = "Đao +7";
        String json4 = formatCatalog();
        check("swordA level updated to 7", json4.indexOf("\"level\": 7") >= 0);
        check("swordA display name updated to +7", json4.indexOf("\"display_name\": \"Đao +7\"") >= 0);
        check("swordB still level 0", json4.indexOf("\"level\": 0") >= 0);

        // ---------------------------------------------------------------------
        // Test 5: Category and template IDs round-trip exactly
        // ---------------------------------------------------------------------
        System.out.println("--- Test 5: Category and template round-trip ---");
        check("template 200 present", json4.indexOf("\"template_id\": 200") >= 0);
        check("category 3 present", json4.indexOf("\"category\": 3") >= 0);
        check("category 7 present", json4.indexOf("\"category\": 7") >= 0);

        // ---------------------------------------------------------------------
        // Test 6: No inventory/action packet sent by catalog generation
        // ---------------------------------------------------------------------
        System.out.println("--- Test 6: No packet sent ---");
        Vector<Object> q = queue();
        q.removeAllElements();
        formatCatalog();
        check("queue remains empty", q.isEmpty());

        // ---------------------------------------------------------------------
        // Test 7: Catalog generation does not mutate bw.V items
        // ---------------------------------------------------------------------
        System.out.println("--- Test 7: No mutation of bw.V items ---");
        check("bw.V size unchanged", bw.V.c() == 3);
        check("swordA template unchanged", swordA.O == 200);
        check("swordA level unchanged", swordA.z == 7);
        check("swordA durability unchanged", swordA.v == 1000);

        // ---------------------------------------------------------------------
        // Test 8: Change suppression hash
        // ---------------------------------------------------------------------
        System.out.println("--- Test 8: Inventory hash detects changes ---");
        Method hashMethod = Class.forName("Zeus").getDeclaredMethod("computeInventoryHash");
        hashMethod.setAccessible(true);
        long h1 = ((Long) hashMethod.invoke(null)).longValue();
        swordB.z = 1;
        long h2 = ((Long) hashMethod.invoke(null)).longValue();
        check("hash changes when item level changes", h1 != h2);
        swordB.z = 0;
        long h3 = ((Long) hashMethod.invoke(null)).longValue();
        check("hash reverts when item reverts", h1 == h3);

        System.out.println("\nTotal failures: " + failures);
        if (failures > 0) {
            System.exit(1);
        }
    }
}
