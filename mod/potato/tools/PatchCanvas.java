/*
 * Patch com/silverknight/a.class at the bytecode level.
 *
 * Why bytecode and not source: class `a` lives in package com.silverknight but
 * calls fu, bl, bx, du and dx, which ProGuard left in the default package. Java
 * source in a named package cannot reference the default package at all, so the
 * decompiled a.java does not recompile. (fu.java is separately unrecompilable:
 * it references the class literally named `do`, a keyword.) Everything else in
 * this module is normal source; only this one class needs a transform.
 *
 * Transform, inside a.run() only:
 *   aload_0; invokevirtual a.repaint()V         -> aload_0; invokestatic POTATO.doRepaint(Canvas)V
 *   aload_0; invokevirtual a.serviceRepaints()V -> aload_0; pop
 *
 * The already-pushed `this` becomes doRepaint's argument, and the second push is
 * popped, so the stack stays balanced. Net effect: the loop asks POTATO whether
 * to paint instead of always painting. Game logic (c.b()) and the 40 ms period
 * are untouched, so tick rate stays 25 Hz.
 *
 * Uses the ASM bundled in microemulator.jar (3.x: ClassAdapter/MethodAdapter).
 */
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.util.Enumeration;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;

import org.objectweb.asm.ClassAdapter;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.ClassWriter;
import org.objectweb.asm.MethodAdapter;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

public final class PatchCanvas {
    private static final String TARGET = "com/silverknight/a";
    private static int replacedRepaint = 0;
    private static int replacedService = 0;

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.out.println("usage: PatchCanvas <in.jar> <out-class-file>");
            System.exit(2);
        }
        ZipFile zf = new ZipFile(args[0]);
        ZipEntry e = zf.getEntry(TARGET + ".class");
        if (e == null) {
            throw new IllegalStateException("missing " + TARGET + " in " + args[0]);
        }
        byte[] original = readAll(zf.getInputStream(e));
        zf.close();

        ClassReader cr = new ClassReader(original);
        ClassWriter cw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        cr.accept(new Adapter(cw), 0);
        byte[] patched = cw.toByteArray();

        if (replacedRepaint != 1 || replacedService != 1) {
            throw new IllegalStateException("expected exactly one repaint and one "
                    + "serviceRepaints call in run(); found "
                    + replacedRepaint + " and " + replacedService
                    + " — the loop is not shaped as assumed, refusing to write");
        }

        File out = new File(args[1]);
        File dir = out.getParentFile();
        if (dir != null) {
            dir.mkdirs();
        }
        FileOutputStream fos = new FileOutputStream(out);
        fos.write(patched);
        fos.close();
        System.out.println("patched " + TARGET + ": " + original.length
                + " -> " + patched.length + " bytes, wrote " + out.getPath());
    }

    private static final class Adapter extends ClassAdapter {
        Adapter(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv != null && "run".equals(name) && "()V".equals(desc)) {
                return new RunAdapter(mv);
            }
            return mv;
        }
    }

    private static final class RunAdapter extends MethodAdapter {
        RunAdapter(MethodVisitor mv) {
            super(mv);
        }

        public void visitMethodInsn(int opcode, String owner, String name, String desc) {
            if (opcode == Opcodes.INVOKEVIRTUAL && "()V".equals(desc)) {
                if ("repaint".equals(name)) {
                    ++replacedRepaint;
                    mv.visitMethodInsn(Opcodes.INVOKESTATIC, "POTATO", "doRepaint",
                            "(Ljavax/microedition/lcdui/Canvas;)V");
                    return;
                }
                if ("serviceRepaints".equals(name)) {
                    ++replacedService;
                    mv.visitInsn(Opcodes.POP);
                    return;
                }
            }
            mv.visitMethodInsn(opcode, owner, name, desc);
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
