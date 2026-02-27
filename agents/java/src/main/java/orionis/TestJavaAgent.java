package orionis;

/**
 * Test: demonstrates Orionis Java agent usage via try-with-resources Span
 * and verifies that auto-instrumentation (via -javaagent) and manual spans both work.
 *
 * Run manually (without agent):
 *   javac -cp . TestJavaAgent.java && java TestJavaAgent
 *
 * Run with auto-instrumentation:
 *   java -javaagent:target/orionis-agent-0.1.0.jar=url=http://localhost:7700,packages=orionis \
 *        -jar target/orionis-agent-0.1.0.jar
 */
public class TestJavaAgent {

    public static void main(String[] args) throws Exception {
        // Manual usage (no -javaagent needed for this path)
        Orionis.start("http://localhost:7700");
        Orionis.resetTrace();

        System.out.println("[Test] Trace ID: " + Orionis.getTraceId());

        // Test manual span
        try (Orionis.Span s = new Orionis.Span("TestJavaAgent.main", "TestJavaAgent.java", 22)) {
            doWork();
            doMoreWork();
        }

        // Test exception capture
        try (Orionis.Span s = new Orionis.Span("TestJavaAgent.crashTest", "TestJavaAgent.java", 30)) {
            crashTest();
        } catch (Exception e) {
            System.out.println("[Test] Caught expected exception: " + e.getMessage());
        }

        // Let the batch flush
        Thread.sleep(300);
        Orionis.stop();
        System.out.println("[Test] Done. Check Orionis dashboard at http://localhost:7700");
    }

    static void doWork() {
        try (Orionis.Span s = new Orionis.Span("TestJavaAgent.doWork", "TestJavaAgent.java", 41)) {
            System.out.println("[Test] doWork() executing");
            // Simulate some work
            long sum = 0;
            for (int i = 0; i < 100_000; i++) sum += i;
            System.out.println("[Test] doWork() sum=" + sum);
        }
    }

    static void doMoreWork() {
        try (Orionis.Span s = new Orionis.Span("TestJavaAgent.doMoreWork", "TestJavaAgent.java", 51)) {
            System.out.println("[Test] doMoreWork() executing");
        }
    }

    static void crashTest() throws Exception {
        try (Orionis.Span s = new Orionis.Span("TestJavaAgent.crashTest", "TestJavaAgent.java", 58)) {
            throw new RuntimeException("Simulated failure for Orionis trace capture");
        }
    }
}
