/*
 * Inject a POTATO gate at the entry of individual draw-layer methods.
 *
 * Why bytecode and not source: every layer call site lives in cn.a(bx), and cn
 * does not survive a round trip through CFR (probe-layers.sh: "incompatible
 * types: possible lossy conversion from int to byte"). The layer classes ey and
 * br do recompile, but recompiling a decompiled class trades a verified .class
 * for a plausible one; a two-instruction prologue keeps every other byte of the
 * original method exactly as ProGuard emitted it.
 *
 * Injected at the head of each target, before its first instruction:
 *
 *     ldc <bit>
 *     invokestatic POTATO.skipLayer(I)Z
 *     ifeq  L
 *     return
 *   L: ...original body...
 *
 * Targets, and why each is safe to drop (all verified in mod/src_decomp):
 *
 *   ey.a(Lbx;)V   bit 1  minimap. Pure drawing: reads cn.j, ey.f, bq.N and the
 *                        player, writes nothing. Local n2/n3 clamping only.
 *   br.a(Lbx;)V   bit 2  effects. Dispatches dy.a(bx) over the live list. The
 *                        effect lifecycle is in br.a() and dy.a() — separate
 *                        methods, still driven every tick — so a skipped draw
 *                        cannot leak an effect. The one subclass that mutates
 *                        inside its draw (i.a(bx)) touches this.ax, a private
 *                        in-call marker read only as `ax == 0`, and this.f, a
 *                        frame index; retirement goes through this.x = true in
 *                        a(), which is untouched.
 *
 * Both are also the two classes that touch Graphics outside bx (ey directly),
 * so gating them here covers the minimap path bx never sees.
 *
 * usage: PatchLayers <in.jar> <out-class-dir>
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

public final class PatchLayers {

    /** class / method / descriptor / POTATO layer bit. */
    private static final String[][] TARGETS = {
        { "ey", "a", "(Lbx;)V", "1" },
        { "br", "a", "(Lbx;)V", "2" },
    };

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.out.println("usage: PatchLayers <in.jar> <out-class-dir>");
            System.exit(2);
        }
        ZipFile zf = new ZipFile(args[0]);
        File outDir = new File(args[1]);

        for (int i = 0; i < TARGETS.length; i++) {
            String owner = TARGETS[i][0];
            String name = TARGETS[i][1];
            String desc = TARGETS[i][2];
            int bit = Integer.parseInt(TARGETS[i][3]);

            ZipEntry e = zf.getEntry(owner + ".class");
            if (e == null) {
                throw new IllegalStateException("missing " + owner + " in " + args[0]);
            }
            byte[] original = readAll(zf.getInputStream(e));

            ClassReader cr = new ClassReader(original);
            ClassWriter cw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
            Injector inj = new Injector(cw, name, desc, bit);
            cr.accept(inj, 0);

            if (inj.injected != 1) {
                throw new IllegalStateException(owner + "." + name + desc
                        + ": expected exactly one match, found " + inj.injected
                        + " — refusing to write");
            }

            byte[] patched = cw.toByteArray();
            File out = new File(outDir, owner + ".class");
            File dir = out.getParentFile();
            if (dir != null) {
                dir.mkdirs();
            }
            FileOutputStream fos = new FileOutputStream(out);
            fos.write(patched);
            fos.close();
            System.out.println("gated " + owner + "." + name + desc
                    + " on layer bit " + bit + ": " + original.length
                    + " -> " + patched.length + " bytes");
        }
        zf.close();
    }

    private static final class Injector extends ClassAdapter {
        private final String name;
        private final String desc;
        private final int bit;
        int injected = 0;

        Injector(ClassWriter cw, String name, String desc, int bit) {
            super(cw);
            this.name = name;
            this.desc = desc;
            this.bit = bit;
        }

        public MethodVisitor visitMethod(int access, String mname, String mdesc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, mname, mdesc, signature, exceptions);
            if (mv != null && this.name.equals(mname) && this.desc.equals(mdesc)) {
                if (!mdesc.endsWith(")V")) {
                    throw new IllegalStateException("target must return void: " + mdesc);
                }
                ++injected;
                return new Prologue(mv, bit);
            }
            return mv;
        }
    }

    private static final class Prologue extends MethodAdapter {
        private final int bit;

        Prologue(MethodVisitor mv, int bit) {
            super(mv);
            this.bit = bit;
        }

        public void visitCode() {
            super.visitCode();
            Label body = new Label();
            mv.visitLdcInsn(new Integer(bit));
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, "POTATO", "skipLayer", "(I)Z");
            mv.visitJumpInsn(Opcodes.IFEQ, body);
            mv.visitInsn(Opcodes.RETURN);
            mv.visitLabel(body);
        }
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
