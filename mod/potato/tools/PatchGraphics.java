/*
 * Patch CLib/mGraphics.class to count primitive drawing operations.
 *
 * Replaces the old v4.0.2 bx.java source replacement with a clean, low-risk ASM patcher.
 * Injects `invokestatic POTATO.countDraw()V` into the 7 primitive rendering methods
 * that directly touch javax.microedition.lcdui.Graphics:
 *
 *   1. drawImage(LCLib/mImage;IIIZ)V
 *   2. drawLine(IIIIZ)V
 *   3. drawRect(IIIIZ)V
 *   4. drawRegion(LCLib/mImage;IIIIIIIIZ)V
 *   5. fillRect(IIIIZ)V
 *   6. fillRoundRect(IIIIIIZ)V
 *   7. fillTriangle(IIIIIIZ)V
 *
 * Each primitive increments POTATO.draws exactly once.
 * Preserves the pristine bytecode of CLib/mGraphics.class completely without recompilation.
 *
 * usage: PatchGraphics <in.jar> <out-class-dir>
 */
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.util.HashMap;
import java.util.Map;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import org.objectweb.asm.ClassAdapter;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodAdapter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

public final class PatchGraphics {
    private static final String TARGET = "CLib/mGraphics";

    private static final String[][] PRIMITIVES = {
        { "drawImage",      "(LCLib/mImage;IIIZ)V" },
        { "drawLine",       "(IIIIZ)V" },
        { "drawRect",       "(IIIIZ)V" },
        { "drawRegion",     "(LCLib/mImage;IIIIIIIIZ)V" },
        { "fillRect",       "(IIIIZ)V" },
        { "fillRoundRect",  "(IIIIIIZ)V" },
        { "fillTriangle",   "(IIIIIIZ)V" },
    };

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.out.println("usage: PatchGraphics <in.jar> <out-class-dir>");
            System.exit(2);
        }
        ZipFile zf = new ZipFile(args[0]);
        File outDir = new File(args[1]);

        ZipEntry e = zf.getEntry(TARGET + ".class");
        if (e == null) {
            throw new IllegalStateException("missing " + TARGET + " in " + args[0]);
        }
        byte[] original = readAll(zf.getInputStream(e));
        zf.close();

        ClassReader cr = new ClassReader(original);
        ClassWriter cw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        GraphicsAdapter adapter = new GraphicsAdapter(cw);
        cr.accept(adapter, 0);

        for (int i = 0; i < PRIMITIVES.length; i++) {
            String key = PRIMITIVES[i][0] + PRIMITIVES[i][1];
            Integer count = adapter.counts.get(key);
            if (count == null || count.intValue() != 1) {
                throw new IllegalStateException("expected primitive " + key + " exactly once, found "
                        + count + " — refusing to write");
            }
        }

        if (adapter.totalPatched != PRIMITIVES.length) {
            throw new IllegalStateException("expected exactly " + PRIMITIVES.length
                    + " patched methods in " + TARGET + ", found " + adapter.totalPatched
                    + " — refusing to write");
        }

        byte[] patched = cw.toByteArray();
        File out = new File(outDir, TARGET + ".class");
        File dir = out.getParentFile();
        if (dir != null) {
            dir.mkdirs();
        }
        FileOutputStream fos = new FileOutputStream(out);
        fos.write(patched);
        fos.close();
        System.out.println("patched " + TARGET + ": " + original.length
                + " -> " + patched.length + " bytes (" + adapter.totalPatched + " primitives counted)");
    }

    private static final class GraphicsAdapter extends ClassAdapter {
        private final Map<String, Integer> counts = new HashMap<String, Integer>();
        int totalPatched = 0;

        GraphicsAdapter(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null) {
                return mv;
            }
            for (int i = 0; i < PRIMITIVES.length; i++) {
                if (PRIMITIVES[i][0].equals(name) && PRIMITIVES[i][1].equals(desc)) {
                    String key = name + desc;
                    Integer prev = counts.get(key);
                    counts.put(key, prev == null ? 1 : prev + 1);
                    ++totalPatched;
                    return new CountDrawPrologue(mv);
                }
            }
            return mv;
        }
    }

    private static final class CountDrawPrologue extends MethodAdapter {
        CountDrawPrologue(MethodVisitor mv) {
            super(mv);
        }

        public void visitCode() {
            super.visitCode();
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, "POTATO", "countDraw", "()V");
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
