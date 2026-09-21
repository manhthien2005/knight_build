// Throwaway harness: drives Zeus.revive() and Zeus.mount() offline.
//
// No network: q.a().b(...) enqueues an `ep` into l.a().o.a (a Vector) and the sender
// thread only drains it once l.c is true, which it never is here. So the queue IS the
// observation: one element per packet the module decided to send, with its opcode.
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.Vector;

public class Harness {

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

    static String text(String name) throws Exception {
        Method m = Class.forName("Zeus").getDeclaredMethod(name);
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

    static String drain() throws Exception {
        Vector<Object> q = queue();
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < q.size(); i++) {
            ep packet = (ep) q.elementAt(i);
            if (out.length() > 0) {
                out.append(',');
            }
            out.append(packet.a);
        }
        q.removeAllElements();
        return out.toString();
    }

    /** One bag item: template id `id`, kind `kind`, display name `name`. */
    static bw item(int id, int kind, String name) {
        j it = new j();
        it.O = id;
        it.u = kind;
        it.g = name;
        return it;
    }

    static et bag(bw... items) {
        et v = new et("bag");
        for (int i = 0; i < items.length; i++) {
            v.a(items[i]);
        }
        return v;
    }

    static void dead(boolean value) {
        cn.g.cG = value ? (byte) 4 : (byte) 0;
    }

    static int tick(int times, String what) throws Exception {
        for (int i = 0; i < times; i++) {
            call(what);
        }
        return times;
    }

    static void check(String label, boolean ok) {
        System.out.println((ok ? "PASS " : "FAIL ") + label);
        if (!ok) {
            failures++;
        }
    }

    static int failures = 0;

    public static void main(String[] args) throws Exception {
        // Trace to stdout is not available, so trace stays off; the queue and the fields are proof.
        set("reviveOn", Boolean.TRUE);
        set("reviveMode", Integer.valueOf(1));
        set("reviveDelay", Integer.valueOf(0));

        // ---- 1. ticket path: 3 sends, then town, then nothing new but town ----
        cn.g.ef = (byte) -1;
        bw.V = bag(item(1234, 6, "Vé Hồi sinh tại chỗ"));
        dead(true);
        drain();
        call("reviveReset");

        // First tick sends immediately: reviveWait starts at 0.
        call("revive");
        check("ticket attempt 1 sends opcode -30", drain().equals("-30"));
        check("reviveTries == 1", ((Integer) get("reviveTries")).intValue() == 1);

        // The period is REVIVE_EVERY ticks that return, then one that sends: the counter is set on
        // the sending tick and decremented on every tick after it.
        tick(60, "revive");
        check("nothing sent during the 60-tick wait", drain().isEmpty());
        call("revive");
        check("ticket attempt 2 after the wait", drain().equals("-30"));
        tick(60, "revive");
        call("revive");
        check("ticket attempt 3", drain().equals("-30"));
        check("reviveTries == 3", ((Integer) get("reviveTries")).intValue() == 3);
        tick(60, "revive");
        call("revive");
        check("4th period falls through to town (opcode 31)", drain().equals("31"));
        tick(60, "revive");
        call("revive");
        tick(60, "revive");
        check("still town, never a 4th ticket", drain().equals("31"));

        // ---- 2. no ticket: one notice, straight to town ----
        call("reviveReset");
        bw.V = bag(item(700, 4, "Ngựa trắng"));
        int before = cn.k.c();
        call("revive");
        check("no ticket goes to town at once", drain().equals("31"));
        check("noTicket flag latched", ((Boolean) get("reviveNoTicket")).booleanValue());
        check("exactly one notice queued", cn.k.c() == before + 1);
        tick(60, "revive");
        call("revive");
        tick(60, "revive");
        check("still town", drain().equals("31"));
        check("notice not repeated", cn.k.c() == before + 1);

        // ---- 3. UI clear at 40 ticks, once ----
        call("reviveReset");
        bw.V = bag(item(1234, 6, "Vé Hồi sinh tại chỗ"));
        fu.p.a = true;
        fu.s = new ah();
        fu.t = new ah();
        drain();
        tick(39, "revive");
        check("UI still up before tick 40", fu.s != null && fu.p.a);
        call("revive");
        check("menu closed at tick 40", !fu.p.a);
        check("dialog dropped at tick 40", fu.s == null && fu.t == null);
        check("cleared flag latched", ((Boolean) get("reviveCleared")).booleanValue());
        fu.p.a = true;
        tick(60, "revive");
        check("menu not closed a second time in the same death", fu.p.a);
        fu.p.a = false;

        // ---- 4. reset on standing up ----
        check("dead counter advanced", ((Integer) get("reviveDead")).intValue() > 40);
        call("reviveReset");
        check("reset zeroes deadTicks", ((Integer) get("reviveDead")).intValue() == 0);
        check("reset zeroes tries", ((Integer) get("reviveTries")).intValue() == 0);
        check("reset clears cleared", !((Boolean) get("reviveCleared")).booleanValue());
        check("reset clears noTicket", !((Boolean) get("reviveNoTicket")).booleanValue());

        // ---- 5. the switch off does nothing at all ----
        set("reviveOn", Boolean.FALSE);
        drain();                    // sends left over from case 3, which ran full periods
        tick(120, "revive");
        check("revive.on=0 sends nothing", drain().isEmpty());
        set("reviveOn", Boolean.TRUE);

        // ---- 6. mount: retry is 5 s when the bag has no mount ----
        dead(false);
        set("mountOn", Boolean.TRUE);
        set("mountId", Integer.valueOf(0));
        set("mountWait", Integer.valueOf(0));
        bw.V = bag(item(1234, 6, "Vé Hồi sinh tại chỗ"));
        call("mount");
        check("empty bag sends nothing", drain().isEmpty());
        check("retry is 5 s (125 ticks), not 900", ((Integer) get("mountWait")).intValue() == 125);
        tick(125, "mount");
        // Mount picked up mid-period: the next look is 5 s away, not 36 s.
        bw.V = bag(item(64, 4, "Tuần lộc"));
        call("mount");
        check("rides the only mount in the bag", drain().equals("32"));
        check("long period charged only for a real ride",
                ((Integer) get("mountWait")).intValue() == 900);

        // ---- 7. mount: prefers the configured id ----
        set("mountId", Integer.valueOf(62));
        set("mountWait", Integer.valueOf(0));
        bw.V = bag(item(65, 4, "Ngựa đen"), item(62, 4, "Ngựa trắng"));
        call("mount");
        check("prefers the wanted id when present", drain().equals("32"));

        // ---- 8. mount: ignores non-mount items and out-of-range ids ----
        set("mountWait", Integer.valueOf(0));
        bw.V = bag(item(62, 6, "not a mount kind"), item(70, 4, "out of range"));
        call("mount");
        check("kind and range are both enforced", drain().isEmpty());
        check("and that is a short retry", ((Integer) get("mountWait")).intValue() == 125);

        // ---- 9. mount: already riding sends nothing ----
        set("mountWait", Integer.valueOf(0));
        cn.g.ef = (byte) 0;
        bw.V = bag(item(62, 4, "Ngựa trắng"));
        call("mount");
        check("riding already: no send", drain().isEmpty());
        check("and no period charged", ((Integer) get("mountWait")).intValue() == 0);
        cn.g.ef = (byte) -1;

        // ---- 10. mount off does nothing ----
        set("mountOn", Boolean.FALSE);
        set("mountWait", Integer.valueOf(0));
        call("mount");
        check("mount.on=0 sends nothing", drain().isEmpty());
        set("mountOn", Boolean.TRUE);

        // ---- 10b. mount.id = 0 rides whatever is carried ----
        set("mountId", Integer.valueOf(0));
        set("mountWait", Integer.valueOf(0));
        bw.V = bag(item(66, 4, "Hoả kì lân"));
        call("mount");
        check("any-mount rides an id that was never configured", drain().equals("32"));

        // ---- 10c. a picked id that is not in the bag stays silent ----
        set("mountId", Integer.valueOf(63));
        set("mountWait", Integer.valueOf(0));
        bw.V = bag(item(66, 4, "Hoả kì lân"));
        call("mount");
        check("a specific mount is not substituted", drain().isEmpty());
        set("mountId", Integer.valueOf(0));

        // ---- 10d. the published list is what the tool offers by name ----
        bw.V = bag(item(62, 4, "Ngựa trắng"), item(1234, 6, "Vé"), item(66, 4, "Hoả kì lân"));
        check("only mounts are listed, id first",
                text("mountList").equals("62:Ngựa trắng|66:Hoả kì lân"));
        bw.V = bag(item(1234, 6, "Vé"));
        check("no mounts is an empty field, not a missing one", text("mountList").isEmpty());

        // ---- 11. the operator's delay holds the first attempt, then everything proceeds ----
        dead(true);
        set("reviveOn", Boolean.TRUE);
        set("reviveDelay", Integer.valueOf(3));      // 3 s = 75 ticks at 25 ticks/s
        bw.V = bag(item(1234, 6, "Vé Hồi sinh tại chỗ"));
        call("reviveReset");
        drain();
        tick(74, "revive");
        check("delay holds the first attempt", drain().isEmpty());
        check("no attempt counted while waiting", ((Integer) get("reviveTries")).intValue() == 0);
        call("revive");
        check("sends on the tick the delay expires", drain().equals("-30"));

        // The UI clear is not on the delay's schedule: a frozen menu is worth closing first.
        call("reviveReset");
        fu.p.a = true;
        fu.s = new ah();
        drain();
        tick(40, "revive");
        check("UI cleared during the delay, not after it", !fu.p.a && fu.s == null);
        check("and still nothing sent", drain().isEmpty());

        // ---- 12. standing up resets without attack() being involved ----
        dead(false);
        call("revive");
        check("alive resets the death by itself", ((Integer) get("reviveDead")).intValue() == 0);
        check("and sends nothing while alive", drain().isEmpty());
        set("reviveDelay", Integer.valueOf(0));

        // ---- 13. revive runs with auto off: it is not part of holding a spot ----
        set("atkMode", Integer.valueOf(0));
        set("atkX", Integer.valueOf(-1));
        set("atkY", Integer.valueOf(-1));
        dead(true);
        call("reviveReset");
        bw.V = bag(item(1234, 6, "Vé Hồi sinh tại chỗ"));
        drain();
        call("revive");
        check("revives with atk.mode=0 and no spot", drain().equals("-30"));

        // ---- 14. town mode never asks for a ticket, even with one in the bag ----
        set("reviveMode", Integer.valueOf(2));
        call("reviveReset");
        drain();
        call("revive");
        check("mode 2 goes straight to town", drain().equals("31"));
        check("and counts no ticket attempt", ((Integer) get("reviveTries")).intValue() == 0);
        set("reviveMode", Integer.valueOf(1));
        dead(false);

        System.out.println(failures == 0 ? "ALL PASS" : (failures + " FAILURES"));
        System.exit(failures == 0 ? 0 : 1);
    }
}
