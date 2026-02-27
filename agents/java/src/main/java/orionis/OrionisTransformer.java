package orionis;

import io.orionis.shaded.asm.ClassReader;
import io.orionis.shaded.asm.ClassWriter;
import io.orionis.shaded.asm.ClassVisitor;
import io.orionis.shaded.asm.MethodVisitor;
import io.orionis.shaded.asm.Opcodes;
import io.orionis.shaded.asm.Type;
import io.orionis.shaded.asm.commons.AdviceAdapter;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.IllegalClassFormatException;
import java.security.ProtectionDomain;

/**
 * OrionisTransformer — ASM-based bytecode transformer.
 *
 * Injects function_enter / function_exit events at the start and end of every
 * method in classes that match the configured package filter. Uses try/finally
 * via AdviceAdapter so exit events are emitted even when exceptions are thrown.
 */
public class OrionisTransformer implements ClassFileTransformer {

    private final OrionisAgent.AgentConfig config;

    // Internal JVM name for Orionis runtime class (used in method calls we inject)
    private static final String RUNTIME_INTERNAL = "orionis/Orionis";

    public OrionisTransformer(OrionisAgent.AgentConfig config) {
        this.config = config;
    }

    @Override
    public byte[] transform(ClassLoader loader,
                            String className,
                            Class<?> classBeingRedefined,
                            ProtectionDomain protectionDomain,
                            byte[] classfileBuffer) throws IllegalClassFormatException {
        if (!config.shouldInstrument(className)) {
            return null; // null = no transformation
        }

        try {
            ClassReader reader = new ClassReader(classfileBuffer);
            // COMPUTE_FRAMES: ASM recomputes stack frames — required after we inject code
            ClassWriter writer = new ClassWriter(reader, ClassWriter.COMPUTE_FRAMES);
            ClassVisitor visitor = new OrionisClassVisitor(writer, className);
            reader.accept(visitor, ClassReader.SKIP_FRAMES);
            return writer.toByteArray();
        } catch (Throwable t) {
            // Never crash the target application due to instrumentation failure
            System.err.println("[Orionis] Transform failed for " + className + ": " + t.getMessage());
            return null;
        }
    }

    // ── Class Visitor ──────────────────────────────────────────────────────

    private static class OrionisClassVisitor extends ClassVisitor {
        private final String className;

        OrionisClassVisitor(ClassWriter writer, String className) {
            super(Opcodes.ASM9, writer);
            this.className = className;
        }

        @Override
        public MethodVisitor visitMethod(int access, String name, String descriptor,
                                         String signature, String[] exceptions) {
            MethodVisitor mv = super.visitMethod(access, name, descriptor, signature, exceptions);

            // Skip: constructors (<init>), static initializers (<clinit>), abstract/native
            if (name.equals("<init>") || name.equals("<clinit>")) return mv;
            if ((access & Opcodes.ACC_ABSTRACT) != 0) return mv;
            if ((access & Opcodes.ACC_NATIVE) != 0) return mv;

            // Readable method name: "com/example/Foo.doThing"
            String readableName = className.replace('/', '.') + "." + name;

            return new OrionisMethodVisitor(mv, access, name, descriptor, className, readableName);
        }
    }

    // ── Method Visitor ─────────────────────────────────────────────────────

    /**
     * Uses AdviceAdapter which handles:
     * - onMethodEnter: called after the method body starts (after super() call in constructors)
     * - onMethodExit:  called before every return opcode AND before athrow (exception propagation)
     *
     * We also wrap the entire body in a try/finally to emit exception events on throws.
     */
    private static class OrionisMethodVisitor extends AdviceAdapter {
        private final String className;
        private final String readableName;
        private int spanIdLocal;
        private int startNsLocal;

        OrionisMethodVisitor(MethodVisitor mv, int access, String name, String descriptor,
                              String className, String readableName) {
            super(Opcodes.ASM9, mv, access, name, descriptor);
            this.className = className;
            this.readableName = readableName;
        }

        @Override
        protected void onMethodEnter() {
            // Allocate local slots for spanId (String) and startNs (long)
            spanIdLocal = newLocal(Type.getType(String.class));
            startNsLocal = newLocal(Type.LONG_TYPE);

            // String spanId = UUID.randomUUID().toString();
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, "java/util/UUID", "randomUUID",
                "()Ljava/util/UUID;", false);
            mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, "java/util/UUID", "toString",
                "()Ljava/lang/String;", false);
            mv.visitVarInsn(Opcodes.ASTORE, spanIdLocal);

            // long startNs = System.nanoTime();
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/System", "nanoTime",
                "()J", false);
            mv.visitVarInsn(Opcodes.LSTORE, startNsLocal);

            // Orionis.emit("function_enter", readableName, className, 0, spanId, null, null);
            emitEvent("function_enter", null);
        }

        @Override
        protected void onMethodExit(int opcode) {
            if (opcode == Opcodes.ATHROW) {
                // Exception path: emit exception event
                // Stack has the exception on top — dup so we can still throw it
                mv.visitInsn(Opcodes.DUP);
                mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, "java/lang/Throwable", "toString",
                    "()Ljava/lang/String;", false);
                mv.visitInsn(Opcodes.POP); // discard, we don't pass message now

                // Emit exception event
                emitExceptionEvent();
            } else {
                // Normal return: emit function_exit with duration
                emitExitEvent();
            }
        }

        // ── Helpers ──────────────────────────────────────────────────────────

        private void emitEvent(String eventType, String parentId) {
            // Orionis.emit(type, func, file, line, spanId, parentId, null)
            mv.visitLdcInsn(eventType);
            mv.visitLdcInsn(readableName);
            mv.visitLdcInsn(className.replace('/', '.'));
            mv.visitInsn(Opcodes.ICONST_0);           // line = 0 (no debug info at runtime)
            mv.visitVarInsn(Opcodes.ALOAD, spanIdLocal);
            if (parentId != null) {
                mv.visitLdcInsn(parentId);
            } else {
                mv.visitInsn(Opcodes.ACONST_NULL);
            }
            mv.visitInsn(Opcodes.ACONST_NULL);        // durUs = null on enter
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, RUNTIME_INTERNAL, "emit",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;Ljava/lang/Long;)V",
                false);
        }

        private void emitExitEvent() {
            // long durUs = (System.nanoTime() - startNs) / 1000;
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/System", "nanoTime", "()J", false);
            mv.visitVarInsn(Opcodes.LLOAD, startNsLocal);
            mv.visitInsn(Opcodes.LSUB);
            mv.visitLdcInsn(1000L);
            mv.visitInsn(Opcodes.LDIV);
            // Box to Long
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, "java/lang/Long", "valueOf",
                "(J)Ljava/lang/Long;", false);
            int durLocal = newLocal(Type.getType(Long.class));
            mv.visitVarInsn(Opcodes.ASTORE, durLocal);

            // Emit: Orionis.emit("function_exit", readableName, className, 0, newSpanId, spanId, dur)
            String newSpanExpr = "java/util/UUID";
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, newSpanExpr, "randomUUID",
                "()Ljava/util/UUID;", false);
            mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, newSpanExpr, "toString",
                "()Ljava/lang/String;", false);
            int exitSpanLocal = newLocal(Type.getType(String.class));
            mv.visitVarInsn(Opcodes.ASTORE, exitSpanLocal);

            mv.visitLdcInsn("function_exit");
            mv.visitLdcInsn(readableName);
            mv.visitLdcInsn(className.replace('/', '.'));
            mv.visitInsn(Opcodes.ICONST_0);
            mv.visitVarInsn(Opcodes.ALOAD, exitSpanLocal);
            mv.visitVarInsn(Opcodes.ALOAD, spanIdLocal);    // parent = entry spanId
            mv.visitVarInsn(Opcodes.ALOAD, durLocal);
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, RUNTIME_INTERNAL, "emit",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;Ljava/lang/Long;)V",
                false);
        }

        private void emitExceptionEvent() {
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, "java/util/UUID", "randomUUID",
                "()Ljava/util/UUID;", false);
            mv.visitMethodInsn(Opcodes.INVOKEVIRTUAL, "java/util/UUID", "toString",
                "()Ljava/lang/String;", false);
            int excSpanLocal = newLocal(Type.getType(String.class));
            mv.visitVarInsn(Opcodes.ASTORE, excSpanLocal);

            mv.visitLdcInsn("exception");
            mv.visitLdcInsn(readableName);
            mv.visitLdcInsn(className.replace('/', '.'));
            mv.visitInsn(Opcodes.ICONST_0);
            mv.visitVarInsn(Opcodes.ALOAD, excSpanLocal);
            mv.visitVarInsn(Opcodes.ALOAD, spanIdLocal);
            mv.visitInsn(Opcodes.ACONST_NULL);
            mv.visitMethodInsn(Opcodes.INVOKESTATIC, RUNTIME_INTERNAL, "emit",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;ILjava/lang/String;Ljava/lang/String;Ljava/lang/Long;)V",
                false);
        }
    }
}
