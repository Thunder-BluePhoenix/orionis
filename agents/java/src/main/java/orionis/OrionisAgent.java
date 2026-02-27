package orionis;

import java.lang.instrument.ClassFileTransformer;
import java.lang.instrument.Instrumentation;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Set;

/**
 * OrionisAgent — JVM -javaagent entry point.
 *
 * Usage:
 *   java -javaagent:orionis-agent-0.1.0.jar=packages=com.myapp,url=http://localhost:7700 -jar app.jar
 *
 * Agent arguments (comma-separated key=value):
 *   url=http://host:port   — Orionis engine URL (default: http://localhost:7700)
 *   packages=com.foo,...   — Comma-separated package prefixes to instrument (default: all non-JDK)
 *   exclude=com.bar,...    — Package prefixes to skip
 *   mode=dev|safe|error    — Tracing mode (default: dev)
 */
public class OrionisAgent {

    // ── System class prefixes we never instrument ──────────────────────────
    private static final Set<String> SYSTEM_PREFIXES = new HashSet<>(Arrays.asList(
        "java/", "javax/", "sun/", "com/sun/", "jdk/",
        "orionis/",            // never self-instrument
        "org/objectweb/asm/",  // ASM itself
        "io/orionis/shaded/"   // shaded ASM copy
    ));

    /**
     * Called by JVM when agent is loaded at startup via -javaagent.
     */
    public static void premain(String agentArgs, Instrumentation instrumentation) {
        agentMain(agentArgs, instrumentation);
    }

    /**
     * Called by JVM when agent is attached dynamically at runtime (Java 9+).
     */
    public static void agentMain(String agentArgs, Instrumentation instrumentation) {
        AgentConfig config = parseArgs(agentArgs);

        // Start the Orionis event sender
        Orionis.start(config.url);

        // Register uncaught exception handler so crashes are reported
        Thread.setDefaultUncaughtExceptionHandler((thread, throwable) -> {
            Orionis.reportException(throwable, thread.getName());
            // Don't swallow — let original handler run too (if any was set)
        });

        // Register shutdown hook to flush pending events
        Runtime.getRuntime().addShutdownHook(new Thread(() -> {
            Orionis.stop();
        }, "orionis-shutdown"));

        // Install the class file transformer
        ClassFileTransformer transformer = new OrionisTransformer(config);
        instrumentation.addTransformer(transformer, true);

        System.out.println("[Orionis] Agent installed. Mode=" + config.mode
            + " | Packages=" + (config.packages.isEmpty() ? "all" : config.packages)
            + " | URL=" + config.url);
    }

    // ── Argument parsing ──────────────────────────────────────────────────

    static AgentConfig parseArgs(String rawArgs) {
        AgentConfig cfg = new AgentConfig();
        if (rawArgs == null || rawArgs.trim().isEmpty()) return cfg;

        for (String part : rawArgs.split(",")) {
            String p = part.trim();
            if (p.startsWith("url=")) {
                cfg.url = p.substring(4);
            } else if (p.startsWith("packages=")) {
                for (String pkg : p.substring(9).split(";")) {
                    cfg.packages.add(pkg.replace('.', '/'));
                }
            } else if (p.startsWith("exclude=")) {
                for (String pkg : p.substring(8).split(";")) {
                    cfg.exclude.add(pkg.replace('.', '/'));
                }
            } else if (p.startsWith("mode=")) {
                cfg.mode = p.substring(5).toLowerCase();
            }
        }
        return cfg;
    }

    // ── Config ─────────────────────────────────────────────────────────────

    static class AgentConfig {
        String url = "http://localhost:7700";
        Set<String> packages = new HashSet<>();
        Set<String> exclude = new HashSet<>();
        String mode = "dev"; // dev | safe | error

        /**
         * Whether a class (internal JVM name like "com/foo/Bar") should be instrumented.
         */
        boolean shouldInstrument(String className) {
            if (className == null) return false;

            // Never instrument system classes
            for (String sys : SYSTEM_PREFIXES) {
                if (className.startsWith(sys)) return false;
            }

            // Never instrument excluded packages
            for (String excl : exclude) {
                if (className.startsWith(excl)) return false;
            }

            // If package filter specified, only instrument matching
            if (!packages.isEmpty()) {
                for (String pkg : packages) {
                    if (className.startsWith(pkg)) return true;
                }
                return false;
            }

            // No filter — instrument everything non-system
            return true;
        }
    }
}
