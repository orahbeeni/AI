package hotdrop.agent;

import java.lang.instrument.Instrumentation;
import java.util.HashMap;
import java.util.Map;

/**
 * Entry point of the in-server agent. Loaded with -javaagent (premain) or through the Attach API (agentmain).
 * Agent options are comma separated key=value pairs: dir=&lt;agents dir&gt;, quiet=true, autostart=false.
 * Must never throw: a failing dev tool must not stop the server from starting.
 */
public final class HotDropAgent {
    private static boolean started;

    private HotDropAgent() {}

    public static void premain(String args, Instrumentation inst) {
        start(args, inst, true);
    }

    public static void agentmain(String args, Instrumentation inst) {
        start(args, inst, false);
    }

    /** autostart: only for -javaagent at server start; an attach comes from a watcher that is already running. */
    private static synchronized void start(String args, Instrumentation inst, boolean autostart) {
        if (started) {
            return;
        }
        started = true;
        try {
            Map<String, String> opts = new HashMap<>();
            if (args != null) {
                for (String kv : args.split(",")) {
                    int i = kv.indexOf('=');
                    if (i > 0) opts.put(kv.substring(0, i).trim(), kv.substring(i + 1).trim());
                }
            }
            new AgentServer(inst, opts).start(autostart);
        } catch (Throwable t) {
            System.err.println("[HotDrop] agent failed to start: " + t);
        }
    }
}
