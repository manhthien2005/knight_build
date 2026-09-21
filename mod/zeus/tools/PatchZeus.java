/*
 * PatchZeus — inject Zeus_Knight hooks into the vanilla jar.
 *
 * Unlike the potato build (which rewrites com.silverknight/a, ey, br), the
 * Zeus round-1 patch is tiny and low-risk:
 *
 *   1. Hook end of fu.b(): inject `invokestatic Zeus.tick()V` at the RETURN.
 *      fu.b() is the main tick — every frame. Verified at orig_decomp/fu.java:261.
 *
 *   2. Widen x.k from private to public.
 *      x.k is the character-select cursor (orig_decomp/x.java:7). Zeus needs
 *      to set it. One access-modifier bit change; every other byte stays.
 *
 *   3. Widen bq.j(II)Z from private to public.
 *      The client's own "can this skill fire?" predicate (orig_decomp/bq.java:1147).
 *      Attack presses hotkeys through it instead of re-deriving the per-level MP
 *      table, which is the kind of duplicated rule that drifts.
 *
 *   4. Widen ah.C from private to public.
 *      The dialog's own button list (orig_decomp/ah.java:21). Dismissing a dialog
 *      means finding its OK button and calling bt.a() — the same call a tap makes.
 *      Pressing at a coordinate instead only works if the guess lands on the button.
 *
 *   5. Trace hooks in ef.b() and fr.a(...): call Zeus.sent/Zeus.menu. Diagnostic only;
 *      Zeus returns immediately unless the operator drops a marker file, so a shipped jar
 *      carries the two calls and no behaviour.
 *
 *   6. (Optional, off by default) Gate the 9 dialog constructors in fu with
 *      `if (Zeus.dialog(text)) return;`. Round 1 does NOT swallow dialogs —
 *      the vanilla reconnect loop lives inside ah.a(), which only runs while
 *      the dialog is alive, so swallowing would kill it. Kept here so a later
 *      round can flip it on per-module without touching this tool.
 *
 * usage: PatchZeus <in.jar> <out-class-dir>
 */
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import org.objectweb.asm.ClassAdapter;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.Label;
import org.objectweb.asm.MethodAdapter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

public final class PatchZeus {

    /** fu methods that build a dialog; third element = slot of the text arg. */
    private static final String[][] DIALOG_METHODS = {
        { "a", "(Ljava/lang/String;)V",                "0" },
        { "a", "(Ljava/lang/String;B)V",               "0" },
        { "a", "(Ljava/lang/String;Ljava/lang/String;)V", "0" },
        { "a", "(Ljava/lang/String;Lbt;)V",            "0" },
        { "a", "(Ljava/lang/String;Let;)V",            "0" },
        { "a", "(Ljava/lang/String;Ljava/lang/String;IIB)V", "0" },
    };

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.out.println("usage: PatchZeus <in.jar> <out-class-dir>");
            System.exit(2);
        }
        ZipFile zf = new ZipFile(args[0]);
        File outDir = new File(args[1]);

        // ── 1. Hook fu.b() ──────────────────────────────────────────────
        byte[] fu = readAll(zf.getInputStream(zf.getEntry("fu.class")));
        ClassReader cr = new ClassReader(fu);
        ClassWriter cw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        TickHook tick = new TickHook(cw);
        cr.accept(tick, 0);
        if (!tick.hooked) {
            throw new IllegalStateException("fu.b()V: no RETURN found — refusing to write");
        }
        writeClass(outDir, "fu.class", cw.toByteArray());

        // ── 2. Widen x.k private -> public ──────────────────────────────
        byte[] xc = readAll(zf.getInputStream(zf.getEntry("x.class")));
        ClassReader xcr = new ClassReader(xc);
        ClassWriter xcw = new ClassWriter(0);
        FieldWidener widen = new FieldWidener(xcw, "k");
        xcr.accept(widen, 0);
        if (!widen.widened) {
            throw new IllegalStateException("x.k field not found — refusing to write");
        }
        writeClass(outDir, "x.class", xcw.toByteArray());

        // ── 3. Widen bq.j(II)Z private -> public ────────────────────────
        // The client's own "can this skill fire?" predicate: learned, off cooldown, MP paid,
        // not already casting (orig_decomp/bq.java:1147). Attack presses hotkeys through it
        // rather than re-deriving the per-level MP table, which would mean re-implementing
        // ct.c(id).k[I[id]+J[id]-1].a and getting to be wrong about it independently.
        byte[] bqc = readAll(zf.getInputStream(zf.getEntry("bq.class")));
        ClassReader bqcr = new ClassReader(bqc);
        ClassWriter bqcw = new ClassWriter(0);
        MethodWidener widenSkill = new MethodWidener(bqcw, "j", "(II)Z");
        bqcr.accept(widenSkill, 0);
        if (!widenSkill.widened) {
            throw new IllegalStateException("bq.j(II)Z not found — refusing to write");
        }
        writeClass(outDir, "bq.class", bqcw.toByteArray());

        // ── 4. Widen ah.C private -> public ─────────────────────────────
        // The dialog's own button list (orig_decomp/ah.java:21, `private et C`). A dialog is
        // dismissed by finding its OK button and invoking `bt.a()` — the same call the operator's
        // tap makes — instead of synthesising a press at some coordinate and hoping it lands on
        // the button. KnightMod reaches the same field the same way, which is what confirmed the
        // approach; its build ships `ah.C` already widened, so it needed no patch of its own.
        byte[] ahc = readAll(zf.getInputStream(zf.getEntry("ah.class")));
        ClassReader ahcr = new ClassReader(ahc);
        ClassWriter ahcw = new ClassWriter(0);
        FieldWidener widenButtons = new FieldWidener(ahcw, "C");
        ahcr.accept(widenButtons, 0);
        if (!widenButtons.widened) {
            throw new IllegalStateException("ah.C field not found — refusing to write");
        }
        writeClass(outDir, "ah.class", ahcw.toByteArray());

        // ── 5. Trace hooks: ef.b() and fr.a(et,int,String,boolean,et) ────
        // Both are diagnostic. They call into Zeus unconditionally, and Zeus returns on its first
        // line unless the operator dropped the marker file, so a shipped jar carries the two calls
        // and no behaviour. They exist because two questions cannot be answered by reading the
        // client: what packet a native menu actually sends, and what a clickable board on the map
        // really is. Recording the real flow beats guessing at it.
        byte[] efc = readAll(zf.getInputStream(zf.getEntry("ef.class")));
        ClassReader efcr = new ClassReader(efc);
        ClassWriter efcw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        SendHook sendHook = new SendHook(efcw);
        efcr.accept(sendHook, 0);
        if (!sendHook.hooked) {
            throw new IllegalStateException("ef.b()V not found — refusing to write");
        }
        writeClass(outDir, "ef.class", efcw.toByteArray());

        byte[] frc = readAll(zf.getInputStream(zf.getEntry("fr.class")));
        ClassReader frcr = new ClassReader(frc);
        ClassWriter frcw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        MenuHook menuHook = new MenuHook(frcw);
        frcr.accept(menuHook, 0);
        if (!menuHook.hooked) {
            throw new IllegalStateException("fr.a menu builder not found — refusing to write");
        }
        if (!menuHook.hookedServer) {
            throw new IllegalStateException("fr.a server menu builder not found — refusing to write");
        }
        writeClass(outDir, "fr.class", frcw.toByteArray());

        // The range ring: a world-space overlay, so it has to be drawn from the world paint rather
        // than from the tick. cn is the game screen, and `a(bx)` is where it paints itself.
        byte[] cnc = readAll(zf.getInputStream(zf.getEntry("cn.class")));
        ClassReader cncr = new ClassReader(cnc);
        ClassWriter cncw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        PaintHook paintHook = new PaintHook(cncw);
        cncr.accept(paintHook, 0);
        if (!paintHook.hooked) {
            throw new IllegalStateException("cn.a(Lbx;)V not found — refusing to write");
        }
        writeClass(outDir, "cn.class", cncw.toByteArray());

        // ── 6. Dialog gates (off by default) ─────────────────────────────
        // Round 1 ships without dialog swallowing. Flip GATE_DIALOGS to true
        // when a module needs it; each gate is a two-instruction prologue:
        //   aload_0
        //   invokestatic Zeus.dialog(Ljava/lang/String;)Z
        //   ifeq L  /  return
        if (GATE_DIALOGS) {
            ClassWriter dcw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
            DialogGates dg = new DialogGates(dcw);
            cr.accept(dg, 0);
            writeClass(outDir, "fu.class", dcw.toByteArray());
        }

        zf.close();
        System.out.println("patched fu (tick hook), x (k public), bq (j public), ah (C public),"
                + " ef (send trace), fr (menu trace)"
                + (GATE_DIALOGS ? ", fu (dialog gates)" : ""));
    }

    private static final boolean GATE_DIALOGS = false;

    // ── fu.b() tick hook ────────────────────────────────────────────────
    private static final class TickHook extends ClassAdapter {
        boolean hooked = false;

        TickHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv != null && "b".equals(name) && "()V".equals(desc)) {
                return new MethodAdapter(mv) {
                    public void visitInsn(int opcode) {
                        if (opcode == Opcodes.RETURN && !hooked) {
                            // inject before the final return
                            visitMethodInsn(Opcodes.INVOKESTATIC,
                                    "Zeus", "tick", "()V");
                            hooked = true;
                        }
                        super.visitInsn(opcode);
                    }
                };
            }
            return mv;
        }
    }

    // ── one named field private -> public ────────────────────────────────
    private static final class FieldWidener extends ClassAdapter {
        private final String targetName;
        boolean widened = false;

        FieldWidener(ClassWriter cw, String name) {
            super(cw);
            this.targetName = name;
        }

        public org.objectweb.asm.FieldVisitor visitField(int access, String name,
                String desc, String signature, Object value) {
            if (targetName.equals(name) && (access & Opcodes.ACC_PRIVATE) != 0) {
                access = (access & ~Opcodes.ACC_PRIVATE) | Opcodes.ACC_PUBLIC;
                widened = true;
            }
            return super.visitField(access, name, desc, signature, value);
        }
    }

    // ── one named method private -> public ───────────────────────────────
    private static final class MethodWidener extends ClassAdapter {
        private final String targetName;
        private final String targetDesc;
        boolean widened = false;

        MethodWidener(ClassWriter cw, String name, String desc) {
            super(cw);
            this.targetName = name;
            this.targetDesc = desc;
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            if (targetName.equals(name) && targetDesc.equals(desc)
                    && (access & Opcodes.ACC_PRIVATE) != 0) {
                access = (access & ~Opcodes.ACC_PRIVATE) | Opcodes.ACC_PUBLIC;
                widened = true;
            }
            return super.visitMethod(access, name, desc, signature, exceptions);
        }
    }

    // ── prologue hook: record every outbound packet ──────────────────────
    //
    // `ef.b()` is the single choke point every sender funnels through: `q extends ef`, and
    // every `q` method ends in `this.b()`. Injecting there means one hook instead of the
    // eighty-odd senders in `q`, and the payload is already complete at that point.
    private static final class SendHook extends ClassAdapter {
        boolean hooked = false;

        SendHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null || !"b".equals(name) || !"()V".equals(desc)) {
                return mv;
            }
            hooked = true;
            return new MethodAdapter(mv) {
                public void visitCode() {
                    super.visitCode();
                    // Zeus.sent(this.b)
                    visitVarInsn(Opcodes.ALOAD, 0);
                    visitFieldInsn(Opcodes.GETFIELD, "ef", "b", "Lep;");
                    visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "sent", "(Lep;)V");
                }
            };
        }
    }

    // ── PaintHook ────────────────────────────────────────────────────────
    // `cn.a(bx)` is the world paint. The prologue runs while the graphics context is still
    // translated into world space (`bx2.a(-p.d.a, -p.d.b)` is the second statement of the method),
    // so a shape drawn from it lands on the map rather than on the screen — which is what an
    // overlay tied to the character needs.
    //
    // Appended, not gated: it draws nothing unless a module asks, and the alternative — drawing
    // from the client's own overlay list — would mean owning an entity the client also owns.
    private static final class PaintHook extends ClassAdapter {
        boolean hooked;

        PaintHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null || !"a".equals(name) || !"(Lbx;)V".equals(desc)) {
                return mv;
            }
            hooked = true;
            return new MethodAdapter(mv) {
                // Drawn on the way OUT, not on the way in. A prologue draws before `fu.q.a(bx2)` paints
                // the map, so the map covered the ring completely — nothing appeared at all. At every
                // return the map and its entities are already down, and the context is still translated
                // into world space, so world coordinates land where they belong with no arithmetic.
                public void visitInsn(int opcode) {
                    if (opcode == Opcodes.RETURN) {
                        visitVarInsn(Opcodes.ALOAD, 1);
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "paint", "(Lbx;)V");
                    }
                    super.visitInsn(opcode);
                }
            };
        }
    }

    // ── prologue hook: record every menu the client shows ────────────────
    //
    // `fr.a(et,int,String,boolean,et)` is the menu builder. The captions and their command ids
    // are the only place a server-driven menu says what selecting an entry would send, so this
    // is what makes a native GUI reproducible instead of guessed at.
    private static final class MenuHook extends ClassAdapter {
        // Two overloads, two different menus. The first builds a locally-assembled menu, which is
        // what the zone board arrives as; the second builds a server-driven one (er.v, opcode −30),
        // which is what a teleport stone arrives as. Hooking only the first is why the stone's
        // destination list never showed up in a trace.
        private static final String LOCAL_DESC = "(Let;ILjava/lang/String;ZLet;)V";
        private static final String SERVER_DESC = "(Let;IIILjava/lang/String;)V";
        boolean hooked = false;
        boolean hookedServer = false;

        MenuHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null || !"a".equals(name)) {
                return mv;
            }
            if (LOCAL_DESC.equals(desc)) {
                hooked = true;
                return new MethodAdapter(mv) {
                    public void visitCode() {
                        super.visitCode();
                        // Zeus.menu(items, title): slot 1 is the et, slot 3 the title (slot 2 is int).
                        // It returns true when a module took the menu for itself, and the builder then
                        // returns before assigning `this.a = true` — so the menu is never drawn at all.
                        // Dismissing it afterwards was not enough: the frame in between still showed.
                        visitVarInsn(Opcodes.ALOAD, 1);
                        visitVarInsn(Opcodes.ALOAD, 3);
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "menu",
                                "(Let;Ljava/lang/String;)Z");
                        Label body = new Label();
                        visitJumpInsn(Opcodes.IFEQ, body);
                        visitInsn(Opcodes.RETURN);
                        visitLabel(body);
                    }
                };
            }
            if (SERVER_DESC.equals(desc)) {
                hookedServer = true;
                return new MethodAdapter(mv) {
                    public void visitCode() {
                        super.visitCode();
                        // fr.a(et, 2, idMenu, idNPC, title) stores idMenu in fr.B and idNPC in fr.C
                        // (fr.java:277-278), and fr.a(2, _) sends q.a().b(C, B, fr.h) — so those two
                        // numbers plus the entry index are the whole selection. Both are private, so
                        // reading them off the builder call is the only way to see them.
                        //
                        // Same swallow as the local overload: a module that answers the menu itself
                        // has no use for it on screen, and the operator did not ask to see it.
                        visitVarInsn(Opcodes.ALOAD, 1);
                        visitVarInsn(Opcodes.ILOAD, 3);
                        visitVarInsn(Opcodes.ILOAD, 4);
                        visitVarInsn(Opcodes.ALOAD, 5);
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "serverMenu",
                                "(Let;IILjava/lang/String;)Z");
                        Label body = new Label();
                        visitJumpInsn(Opcodes.IFEQ, body);
                        visitInsn(Opcodes.RETURN);
                        visitLabel(body);
                    }
                };
            }
            return mv;
        }
    }

    // ── Dialog gates ─────────────────────────────────────────────────────
    private static final class DialogGates extends ClassAdapter {
        int gated = 0;

        DialogGates(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null) {
                return mv;
            }
            for (int i = 0; i < DIALOG_METHODS.length; i++) {
                if (DIALOG_METHODS[i][0].equals(name) && DIALOG_METHODS[i][1].equals(desc)) {
                    final int textArg = Integer.parseInt(DIALOG_METHODS[i][2]);
                    ++gated;
                    return new MethodAdapter(mv) {
                        public void visitCode() {
                            super.visitCode();
                            Label body = new Label();
                            // load the text argument (all targets are static)
                            switch (textArg) {
                                case 0: visitVarInsn(Opcodes.ALOAD, 0); break;
                                default:
                                    throw new IllegalStateException("arg " + textArg + " unsupported");
                            }
                            visitMethodInsn(Opcodes.INVOKESTATIC,
                                    "Zeus", "dialog", "(Ljava/lang/String;)Z");
                            visitJumpInsn(Opcodes.IFEQ, body);
                            visitInsn(Opcodes.RETURN);
                            visitLabel(body);
                        }
                    };
                }
            }
            return mv;
        }
    }

    private static void writeClass(File outDir, String name, byte[] bytes) throws Exception {
        File out = new File(outDir, name);
        File parent = out.getParentFile();
        if (parent != null) {
            parent.mkdirs();
        }
        FileOutputStream fos = new FileOutputStream(out);
        fos.write(bytes);
        fos.close();
        System.out.println("wrote " + name + " (" + bytes.length + " bytes)");
    }

    private static byte[] readAll(InputStream in) throws Exception {
        ByteArrayOutputStream bos = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = in.read(buf)) > 0) {
            bos.write(buf, 0, n);
        }
        in.close();
        return bos.toByteArray();
    }
}
