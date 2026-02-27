package orionis;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.time.Instant;
import java.util.HashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.stream.Collectors;

/**
 * Orionis Java Agent — Lightweight runtime tracing for JVM.
 */
public class Orionis {
    private static String engineUrl = "http://localhost:7700";
    private static final ThreadLocal<String> traceId = new ThreadLocal<>();
    private static final HttpClient httpClient = HttpClient.newBuilder().build();
    private static final ConcurrentLinkedQueue<Map<String, Object>> batch = new ConcurrentLinkedQueue<>();
    private static final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "orionis-sender");
        t.setDaemon(true);
        return t;
    });

    static {
        scheduler.scheduleAtFixedRate(Orionis::flush, 100, 100, TimeUnit.MILLISECONDS);
    }

    public static void start(String url) {
        if (url != null) engineUrl = url;
        resetTrace();
        System.out.println("[Orionis] Java Agent started: " + engineUrl);
    }

    public static void stop() {
        flush();
        scheduler.shutdown();
    }

    public static void resetTrace() {
        traceId.set(UUID.randomUUID().toString());
    }

    public static String getTraceId() {
        if (traceId.get() == null) resetTrace();
        return traceId.get();
    }

    public static void injectTraceHeaders(Map<String, String> headers) {
        String tid = getTraceId().replace("-", "");
        headers.put("traceparent", "00-" + tid + "-0000000000000000-01");
    }

    public static void extractTraceHeaders(Map<String, String> headers) {
        String tp = headers.get("traceparent");
        if (tp != null && tp.length() >= 55) {
            String tidRaw = tp.substring(3, 35);
            String tid = String.format("%s-%s-%s-%s-%s",
                tidRaw.substring(0, 8), tidRaw.substring(8, 12),
                tidRaw.substring(12, 16), tidRaw.substring(16, 20),
                tidRaw.substring(20, 32));
            traceId.set(tid);
        }
    }

    public static void emit(String type, String func, String file, int line, String spanId, String parentId, Long durUs) {
        Map<String, Object> ev = new HashMap<>();
        ev.put("trace_id", getTraceId());
        ev.put("span_id", spanId);
        ev.put("parent_span_id", parentId);
        ev.put("timestamp_ms", System.currentTimeMillis());
        ev.put("event_type", type);
        ev.put("function_name", func);
        ev.put("module", "java_app");
        ev.put("file", file);
        ev.put("line", line);
        ev.put("language", "java");
        ev.put("duration_us", durUs);
        ev.put("thread_id", Thread.currentThread().getName());
        batch.add(ev);
    }

    /**
     * Report an uncaught exception (called by the default uncaught exception handler installed
     * by OrionisAgent). Emits an exception event with the throwable details.
     */
    public static void reportException(Throwable t, String threadName) {
        String spanId = UUID.randomUUID().toString();
        Map<String, Object> ev = new HashMap<>();
        ev.put("trace_id", getTraceId());
        ev.put("span_id", spanId);
        ev.put("parent_span_id", null);
        ev.put("timestamp_ms", System.currentTimeMillis());
        ev.put("event_type", "exception");
        ev.put("function_name", t.getClass().getName());
        ev.put("module", "java_app");
        ev.put("file", "uncaught");
        ev.put("line", 0);
        ev.put("language", "java");
        ev.put("duration_us", null);
        ev.put("thread_id", threadName);
        ev.put("exception_message", t.getMessage() != null ? t.getMessage() : "");
        ev.put("exception_type", t.getClass().getName());
        batch.add(ev);
        // Immediately flush so the event isn't lost on JVM shutdown
        flush();
    }

    private static void flush() {
        if (batch.isEmpty()) return;
        
        java.util.List<Map<String, Object>> events = new java.util.ArrayList<>();
        while (!batch.isEmpty()) {
            events.add(batch.poll());
        }

        try {
            String json = "[" + events.stream()
                .map(Orionis::toJson)
                .collect(Collectors.joining(",")) + "]";

            HttpRequest request = HttpRequest.newBuilder()
                .uri(URI.create(engineUrl + "/api/ingest"))
                .header("Content-Type", "application/json")
                .POST(HttpRequest.BodyPublishers.ofString(json))
                .build();

            httpClient.sendAsync(request, HttpResponse.BodyHandlers.discarding());
        } catch (Exception e) {
            // Ignore ingestion errors to keep app stable
        }
    }

    private static String toJson(Map<String, Object> map) {
        return "{" + map.entrySet().stream()
            .map(e -> "\"" + e.getKey() + "\":" + (e.getValue() instanceof String ? "\"" + escape(e.getValue().toString()) + "\"" : e.getValue()))
            .collect(Collectors.joining(",")) + "}";
    }

    private static String escape(String s) {
        return s.replace("\\", "\\\\").replace("\"", "\\\"");
    }

    /**
     * RAII-style Span for try-with-resources.
     */
    public static class Span implements AutoCloseable {
        private final String spanId;
        private final String func;
        private final String file;
        private final int line;
        private final long startNs;

        public Span(String func, String file, int line) {
            this.spanId = UUID.randomUUID().toString();
            this.func = func;
            this.file = file;
            this.line = line;
            this.startNs = System.nanoTime();
            emit("function_enter", func, file, line, spanId, null, null);
        }

        @Override
        public void close() {
            long durUs = (System.nanoTime() - startNs) / 1000;
            emit("function_exit", func, file, line, UUID.randomUUID().toString(), spanId, durUs);
        }
    }
}
