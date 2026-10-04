/*
 * PatchZeus — inject Zeus_Knight hooks into the official v4.0.3 vanilla jar.
 *
 * Hooks:
 *   1. Main/GameCanvas.update()V: inject `invokestatic Zeus.tick()V` at the RETURN.
 *   2. GameScreen/SelectCharScreen.selectChar: widen private/package -> public.
 *   3. InterfaceComponents/MsgDialog.cmdList: widen private/package -> public.
 *   4. netcommand/Cmd_Message.send()V: inject `Zeus.sent(this.m)` at prologue.
 *   5. Model/Menu2:
 *      - startAt(mVector, int, String, boolean, mVector): inject local menu hook Zeus.menu(mVector, String).
 *      - setinfoDynamic(mVector, int, int, int, String): inject server menu hook Zeus.serverMenu(mVector, idMenu, idNPC, String).
 *   6. GameScreen/GameScreen.paint(mGraphics)V: inject `Zeus.paint(g)` before RETURN.
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

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            System.out.println("usage: PatchZeus <in.jar> <out-class-dir>");
            System.exit(2);
        }
        ZipFile zf = new ZipFile(args[0]);
        File outDir = new File(args[1]);

        // ── 1. Hook Main/GameCanvas.update()V ───────────────────────────
        byte[] gcBytes = readAll(zf.getInputStream(zf.getEntry("Main/GameCanvas.class")));
        ClassReader gcr = new ClassReader(gcBytes);
        ClassWriter gcw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        TickHook tick = new TickHook(gcw);
        gcr.accept(tick, 0);
        if (!tick.hooked) {
            throw new IllegalStateException("Main/GameCanvas.update()V: no RETURN found — refusing to write");
        }
        writeClass(outDir, "Main/GameCanvas.class", gcw.toByteArray());

        // ── 2. Widen GameScreen/SelectCharScreen.selectChar -> public ────
        byte[] scBytes = readAll(zf.getInputStream(zf.getEntry("GameScreen/SelectCharScreen.class")));
        ClassReader scr = new ClassReader(scBytes);
        ClassWriter scw = new ClassWriter(0);
        FieldWidener widenSelectChar = new FieldWidener(scw, "selectChar");
        scr.accept(widenSelectChar, 0);
        if (!widenSelectChar.widened) {
            throw new IllegalStateException("GameScreen/SelectCharScreen.selectChar not found — refusing to write");
        }
        writeClass(outDir, "GameScreen/SelectCharScreen.class", scw.toByteArray());

        // ── 3. Widen InterfaceComponents/MsgDialog.cmdList -> public ────
        byte[] mdBytes = readAll(zf.getInputStream(zf.getEntry("InterfaceComponents/MsgDialog.class")));
        ClassReader mdr = new ClassReader(mdBytes);
        ClassWriter mdw = new ClassWriter(0);
        FieldWidener widenCmdList = new FieldWidener(mdw, "cmdList");
        mdr.accept(widenCmdList, 0);
        if (!widenCmdList.widened) {
            throw new IllegalStateException("InterfaceComponents/MsgDialog.cmdList not found — refusing to write");
        }
        writeClass(outDir, "InterfaceComponents/MsgDialog.class", mdw.toByteArray());

        // ── 4. Trace hook netcommand/Cmd_Message.send()V ─────────────────
        byte[] cmdBytes = readAll(zf.getInputStream(zf.getEntry("netcommand/Cmd_Message.class")));
        ClassReader cmdr = new ClassReader(cmdBytes);
        ClassWriter cmdw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        SendHook sendHook = new SendHook(cmdw);
        cmdr.accept(sendHook, 0);
        if (!sendHook.hooked) {
            throw new IllegalStateException("netcommand/Cmd_Message.send()V not found — refusing to write");
        }
        writeClass(outDir, "netcommand/Cmd_Message.class", cmdw.toByteArray());

        // ── 5. Menu hooks Model/Menu2 ───────────────────────────────────
        byte[] menuBytes = readAll(zf.getInputStream(zf.getEntry("Model/Menu2.class")));
        ClassReader menur = new ClassReader(menuBytes);
        ClassWriter menuw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        MenuHook menuHook = new MenuHook(menuw);
        menur.accept(menuHook, 0);
        if (!menuHook.hooked) {
            throw new IllegalStateException("Model/Menu2.startAt menu builder not found — refusing to write");
        }
        if (!menuHook.hookedServer) {
            throw new IllegalStateException("Model/Menu2.setinfoDynamic server menu builder not found — refusing to write");
        }
        writeClass(outDir, "Model/Menu2.class", menuw.toByteArray());

        // ── 6. Paint hook GameScreen/GameScreen.paint(mGraphics)V ────────
        byte[] gsBytes = readAll(zf.getInputStream(zf.getEntry("GameScreen/GameScreen.class")));
        ClassReader gsr = new ClassReader(gsBytes);
        ClassWriter gsw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        PaintHook paintHook = new PaintHook(gsw);
        gsr.accept(paintHook, 0);
        if (!paintHook.hooked) {
            throw new IllegalStateException("GameScreen/GameScreen.paint(LCLib/mGraphics;)V not found — refusing to write");
        }
        writeClass(outDir, "GameScreen/GameScreen.class", gsw.toByteArray());

        zf.close();
        System.out.println("PatchZeus complete: patched GameCanvas, SelectCharScreen, MsgDialog, Cmd_Message, Menu2, GameScreen");
    }


    // ── GameCanvas.update() tick hook ───────────────────────────────────
    private static final class TickHook extends ClassAdapter {
        boolean hooked = false;

        TickHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv != null && "update".equals(name) && "()V".equals(desc)) {
                return new MethodAdapter(mv) {
                    public void visitInsn(int opcode) {
                        if (opcode == Opcodes.RETURN && !hooked) {
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

    // ── Field widener (private/package-private -> public) ───────────────
    private static final class FieldWidener extends ClassAdapter {
        private final String targetName;
        boolean widened = false;

        FieldWidener(ClassWriter cw, String name) {
            super(cw);
            this.targetName = name;
        }

        public org.objectweb.asm.FieldVisitor visitField(int access, String name,
                String desc, String signature, Object value) {
            if (targetName.equals(name)) {
                access = (access & ~(Opcodes.ACC_PRIVATE | Opcodes.ACC_PROTECTED)) | Opcodes.ACC_PUBLIC;
                widened = true;
            }
            return super.visitField(access, name, desc, signature, value);
        }
    }

    // ── Send hook: Cmd_Message.send() ───────────────────────────────────
    private static final class SendHook extends ClassAdapter {
        boolean hooked = false;

        SendHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null || !"send".equals(name) || !"()V".equals(desc)) {
                return mv;
            }
            hooked = true;
            return new MethodAdapter(mv) {
                public void visitCode() {
                    super.visitCode();
                    visitVarInsn(Opcodes.ALOAD, 0);
                    visitFieldInsn(Opcodes.GETFIELD, "netcommand/Cmd_Message", "m", "Lnet/Message;");
                    visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "sent", "(Lnet/Message;)V");
                }
            };
        }
    }

    // ── Paint hook: GameScreen.paint(mGraphics) ─────────────────────────
    private static final class PaintHook extends ClassAdapter {
        boolean hooked = false;

        PaintHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null || !"paint".equals(name) || !"(LCLib/mGraphics;)V".equals(desc)) {
                return mv;
            }
            hooked = true;
            return new MethodAdapter(mv) {
                public void visitInsn(int opcode) {
                    if (opcode == Opcodes.RETURN) {
                        visitVarInsn(Opcodes.ALOAD, 1);
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "paint", "(LCLib/mGraphics;)V");
                    }
                    super.visitInsn(opcode);
                }
            };
        }
    }

    // ── Menu hooks: Menu2.startAt and Menu2.setinfoDynamic ──────────────
    private static final class MenuHook extends ClassAdapter {
        private static final String LOCAL_DESC = "(LCLib/mVector;ILjava/lang/String;ZLCLib/mVector;)V";
        private static final String SERVER_DESC = "(LCLib/mVector;IIILjava/lang/String;)V";
        boolean hooked = false;
        boolean hookedServer = false;

        MenuHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null) {
                return mv;
            }
            if ("startAt".equals(name) && LOCAL_DESC.equals(desc)) {
                hooked = true;
                return new MethodAdapter(mv) {
                    public void visitCode() {
                        super.visitCode();
                        visitVarInsn(Opcodes.ALOAD, 1);
                        visitVarInsn(Opcodes.ALOAD, 3);
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "menu",
                                "(LCLib/mVector;Ljava/lang/String;)Z");
                        Label body = new Label();
                        visitJumpInsn(Opcodes.IFEQ, body);
                        visitInsn(Opcodes.RETURN);
                        visitLabel(body);
                    }
                };
            }
            if ("setinfoDynamic".equals(name) && SERVER_DESC.equals(desc)) {
                hookedServer = true;
                return new MethodAdapter(mv) {
                    public void visitCode() {
                        super.visitCode();
                        visitVarInsn(Opcodes.ALOAD, 1);
                        visitVarInsn(Opcodes.ILOAD, 3);
                        visitVarInsn(Opcodes.ILOAD, 4);
                        visitVarInsn(Opcodes.ALOAD, 5);
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "serverMenu",
                                "(LCLib/mVector;IILjava/lang/String;)Z");
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
