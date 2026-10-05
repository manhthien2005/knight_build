/*
 * PatchZeus — inject Zeus_Knight hooks into the official v4.0.3 vanilla jar.
 *
 * Hooks:
 *   1. Main/GameCanvas:
 *      - update()V: inject `invokestatic Zeus.tick()V` at the RETURN.
 *      - connect()V: inject `invokestatic Zeus.serverTargetSafe()Z` guard at prologue.
 *   2. GameScreen/SelectCharScreen.selectChar: widen private/package -> public.
 *   3. InterfaceComponents/MsgDialog.cmdList: widen private/package -> public.
 *   4. netcommand/Cmd_Message.send()V: inject `Zeus.sent(this.m)` at prologue.
 *   5. Model/Menu2:
 *      - startAt(mVector, int, String, boolean, mVector): inject local menu hook Zeus.menu(mVector, String).
 *      - setinfoDynamic(mVector, int, int, int, String): inject server menu hook Zeus.serverMenu(mVector, idMenu, idNPC, String).
 *   6. GameScreen/GameScreen.paint(mGraphics)V: inject `Zeus.paint(g)` before RETURN.
 *   7. GameScreen/LoginScreen.login(String, String)V: inject `invokestatic Zeus.serverTargetSafe()Z` guard at prologue.
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

        // ── 1. Hook Main/GameCanvas.update()V (tick) & connect()V (guard) ──
        byte[] gcBytes = readAll(zf.getInputStream(zf.getEntry("Main/GameCanvas.class")));
        ClassReader gcr = new ClassReader(gcBytes);
        ClassWriter gcw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        GameCanvasHook gcHook = new GameCanvasHook(gcw);
        gcr.accept(gcHook, 0);
        if (gcHook.updateMethodCount != 1 || gcHook.tickReturnCount != 1) {
            throw new IllegalStateException("Main/GameCanvas.update()V: expected exactly 1 method and 1 RETURN, found "
                    + gcHook.updateMethodCount + " methods and " + gcHook.tickReturnCount + " RETURNs — refusing to write");
        }
        if (gcHook.connectMethodCount != 1 || gcHook.connectGuardCount != 1) {
            throw new IllegalStateException("Main/GameCanvas.connect()V: expected exactly 1 method and 1 guard, found "
                    + gcHook.connectMethodCount + " methods and " + gcHook.connectGuardCount + " guards — refusing to write");
        }
        writeClass(outDir, "Main/GameCanvas.class", gcw.toByteArray());

        // ── 2. Widen GameScreen/SelectCharScreen.selectChar -> public ────
        byte[] scBytes = readAll(zf.getInputStream(zf.getEntry("GameScreen/SelectCharScreen.class")));
        ClassReader scr = new ClassReader(scBytes);
        ClassWriter scw = new ClassWriter(0);
        FieldWidener widenSelectChar = new FieldWidener(scw, "selectChar", "I");
        scr.accept(widenSelectChar, 0);
        if (widenSelectChar.widenedCount != 1) {
            throw new IllegalStateException("GameScreen/SelectCharScreen.selectChar:I expected exactly once, found "
                    + widenSelectChar.widenedCount + " — refusing to write");
        }
        writeClass(outDir, "GameScreen/SelectCharScreen.class", scw.toByteArray());

        // ── 3. Widen InterfaceComponents/MsgDialog.cmdList -> public ────
        byte[] mdBytes = readAll(zf.getInputStream(zf.getEntry("InterfaceComponents/MsgDialog.class")));
        ClassReader mdr = new ClassReader(mdBytes);
        ClassWriter mdw = new ClassWriter(0);
        FieldWidener widenCmdList = new FieldWidener(mdw, "cmdList", "LCLib/mVector;");
        mdr.accept(widenCmdList, 0);
        if (widenCmdList.widenedCount != 1) {
            throw new IllegalStateException("InterfaceComponents/MsgDialog.cmdList expected exactly once, found "
                    + widenCmdList.widenedCount + " — refusing to write");
        }
        writeClass(outDir, "InterfaceComponents/MsgDialog.class", mdw.toByteArray());

        // ── 4. Hook netcommand/Cmd_Message.send()V ──────────────────────────
        byte[] cmdBytes = readAll(zf.getInputStream(zf.getEntry("netcommand/Cmd_Message.class")));
        ClassReader cmdr = new ClassReader(cmdBytes);
        ClassWriter cmdw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        SendHook sendHook = new SendHook(cmdw);
        cmdr.accept(sendHook, 0);
        if (sendHook.targetMethodCount != 1) {
            throw new IllegalStateException("netcommand/Cmd_Message.send()V expected exactly 1 method, found "
                    + sendHook.targetMethodCount + " — refusing to write");
        }
        writeClass(outDir, "netcommand/Cmd_Message.class", cmdw.toByteArray());

        // ── 5. Hook Model/Menu2 ─────────────────────────────────────────────
        byte[] menuBytes = readAll(zf.getInputStream(zf.getEntry("Model/Menu2.class")));
        ClassReader menur = new ClassReader(menuBytes);
        ClassWriter menuw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        MenuHook menuHook = new MenuHook(menuw);
        menur.accept(menuHook, 0);
        if (menuHook.localHookCount != 1) {
            throw new IllegalStateException("Model/Menu2.startAt expected exactly 1 method, found "
                    + menuHook.localHookCount + " — refusing to write");
        }
        if (menuHook.serverHookCount != 1) {
            throw new IllegalStateException("Model/Menu2.setinfoDynamic expected exactly 1 method, found "
                    + menuHook.serverHookCount + " — refusing to write");
        }
        writeClass(outDir, "Model/Menu2.class", menuw.toByteArray());

        // ── 6. Paint hook GameScreen/GameScreen.paint(mGraphics)V ────────
        byte[] gsBytes = readAll(zf.getInputStream(zf.getEntry("GameScreen/GameScreen.class")));
        ClassReader gsr = new ClassReader(gsBytes);
        ClassWriter gsw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        PaintHook paintHook = new PaintHook(gsw);
        gsr.accept(paintHook, 0);
        if (paintHook.targetMethodCount != 1 || paintHook.returnCount != 1) {
            throw new IllegalStateException("GameScreen/GameScreen.paint(LCLib/mGraphics;)V: expected exactly 1 method and 1 RETURN, found "
                    + paintHook.targetMethodCount + " methods and " + paintHook.returnCount + " RETURNs — refusing to write");
        }
        writeClass(outDir, "GameScreen/GameScreen.class", gsw.toByteArray());

        // ── 7. LoginScreen.login(String, String)V serverTargetSafe guard ────
        byte[] lsBytes = readAll(zf.getInputStream(zf.getEntry("GameScreen/LoginScreen.class")));
        ClassReader lsr = new ClassReader(lsBytes);
        ClassWriter lsw = new ClassWriter(ClassWriter.COMPUTE_MAXS);
        LoginScreenHook loginHook = new LoginScreenHook(lsw);
        lsr.accept(loginHook, 0);
        if (loginHook.targetMethodCount != 1 || loginHook.guardCount != 1) {
            throw new IllegalStateException("GameScreen/LoginScreen.login(String,String): expected exactly 1 private method, found "
                    + loginHook.targetMethodCount + " methods and " + loginHook.guardCount + " guards — refusing to write");
        }
        writeClass(outDir, "GameScreen/LoginScreen.class", lsw.toByteArray());

        zf.close();
        System.out.println("PatchZeus complete: patched GameCanvas, SelectCharScreen, MsgDialog, Cmd_Message, Menu2, GameScreen, LoginScreen");
    }

    // ── Main/GameCanvas hook: update() tick hook + connect() serverTargetSafe guard ──
    private static final class GameCanvasHook extends ClassAdapter {
        int updateMethodCount = 0;
        int tickReturnCount = 0;
        int connectMethodCount = 0;
        int connectGuardCount = 0;

        GameCanvasHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv != null && "update".equals(name) && "()V".equals(desc)) {
                updateMethodCount++;
                return new MethodAdapter(mv) {
                    public void visitInsn(int opcode) {
                        if (opcode == Opcodes.RETURN) {
                            tickReturnCount++;
                            visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "tick", "()V");
                        }
                        super.visitInsn(opcode);
                    }
                };
            }
            if (mv != null && "connect".equals(name) && "()V".equals(desc)) {
                connectMethodCount++;
                return new MethodAdapter(mv) {
                    public void visitCode() {
                        super.visitCode();
                        connectGuardCount++;
                        Label proceed = new Label();
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "serverTargetSafe", "()Z");
                        visitJumpInsn(Opcodes.IFNE, proceed);
                        visitInsn(Opcodes.RETURN);
                        visitLabel(proceed);
                    }
                };
            }
            return mv;
        }
    }

    // ── GameScreen/LoginScreen hook: login(String, String) serverTargetSafe guard ──
    private static final class LoginScreenHook extends ClassAdapter {
        int targetMethodCount = 0;
        int guardCount = 0;

        LoginScreenHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv != null && "login".equals(name) && "(Ljava/lang/String;Ljava/lang/String;)V".equals(desc)
                    && (access & Opcodes.ACC_PRIVATE) != 0) {
                targetMethodCount++;
                return new MethodAdapter(mv) {
                    public void visitCode() {
                        super.visitCode();
                        guardCount++;
                        Label proceed = new Label();
                        visitMethodInsn(Opcodes.INVOKESTATIC, "Zeus", "serverTargetSafe", "()Z");
                        visitJumpInsn(Opcodes.IFNE, proceed);
                        visitInsn(Opcodes.RETURN);
                        visitLabel(proceed);
                    }
                };
            }
            return mv;
        }
    }

    // ── Field widener (private/package-private -> public) ───────────────
    private static final class FieldWidener extends ClassAdapter {
        private final String targetName;
        private final String targetDesc;
        int widenedCount = 0;

        FieldWidener(ClassWriter cw, String name, String desc) {
            super(cw);
            this.targetName = name;
            this.targetDesc = desc;
        }

        public org.objectweb.asm.FieldVisitor visitField(int access, String name,
                String desc, String signature, Object value) {
            if (targetName.equals(name)) {
                if (!targetDesc.equals(desc)) {
                    throw new IllegalStateException("field " + name + " found with unexpected descriptor: "
                            + desc + " (expected " + targetDesc + ")");
                }
                access = (access & ~(Opcodes.ACC_PRIVATE | Opcodes.ACC_PROTECTED)) | Opcodes.ACC_PUBLIC;
                widenedCount++;
            }
            return super.visitField(access, name, desc, signature, value);
        }
    }

    // ── Send hook: Cmd_Message.send() ───────────────────────────────────
    private static final class SendHook extends ClassAdapter {
        int targetMethodCount = 0;

        SendHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null || !"send".equals(name) || !"()V".equals(desc)) {
                return mv;
            }
            targetMethodCount++;
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
        int targetMethodCount = 0;
        int returnCount = 0;

        PaintHook(ClassWriter cw) {
            super(cw);
        }

        public MethodVisitor visitMethod(int access, String name, String desc,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, desc, signature, exceptions);
            if (mv == null || !"paint".equals(name) || !"(LCLib/mGraphics;)V".equals(desc)) {
                return mv;
            }
            targetMethodCount++;
            return new MethodAdapter(mv) {
                public void visitInsn(int opcode) {
                    if (opcode == Opcodes.RETURN) {
                        returnCount++;
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
        int localHookCount = 0;
        int serverHookCount = 0;

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
                localHookCount++;
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
                serverHookCount++;
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
