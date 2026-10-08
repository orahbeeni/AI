# HotDrop - context for Claude

## Intent
HotDrop is the user's own clean-room alternative to JRebel for SAP Commerce (Hybris), because there is no JRebel budget.
Save a `.java` file in IntelliJ -> it is compiled and hot-swapped into the running Hybris server, no restart.
A file that doesn't compile stays pending and is retried on later saves (in any file) until it compiles; dependents of a
broken class are held back. Nothing illegal or derived from JRebel/GPL code: JDK APIs only (`java.lang.instrument`,
Attach API, `javax.tools`). Must be fast (target under 300 ms save-to-live) and **must work on Linux, macOS and Windows**.

Read first: `PLAN.md` (design + verification log of every assumption, with evidence), `README.md` (usage).

## State
Built and passing on Linux (Temurin 17): protocol, agent, daemon (discovery, warm javac, scheduler, polling watcher), CLI,
end-to-end test `it/IntegrationTest.java`. Developed on Linux, so macOS and Windows have NOT been run by hand yet.
Not built: Spring layer, IntelliJ plugin, strict per-extension classpath. Not yet done: the spike on a real Hybris server.

## If you are on macOS (the current task): test it
1. `java -version` must be a JDK 17+ (not a JRE). Note the version and vendor.
2. `java Build.java` - must print `built: ...` with no warnings.
3. `java it/IntegrationTest.java` - default watcher on macOS is polling. All checks must PASS.
4. `HOTDROP_WATCH=native java it/IntegrationTest.java` - the JDK's native macOS watcher. Report whether it passes and the
   three latency numbers at the end; compare with the default run. This tells us if the polling watcher was worth it.
5. `bin/hotdrop doctor` and `bin/hotdrop --help`.
6. Repeat 3 a few times for flakiness (timings on a Mac may need larger timeouts in the test, which is fine to adjust).
7. If a real Hybris tree exists on this machine, run `bin/hotdrop doctor --hybris <hybris dir>` and, with a custom extension
   in `bin/custom`, `bin/hotdrop swap --hybris <dir> <some .java file>`.
Fix macOS-specific bugs you find (keep Linux and Windows working; JDK-only, no new dependencies), re-run everything, and
report: what passed, what you changed, measured latencies. Do not claim Windows works; only the CI run proves that.

## Rules for changes
- Cross-platform: `java.nio.file.Path`, never string path math; no bash-only steps; guard POSIX-only and `/proc` use.
- No third-party dependencies in the agent or daemon (the agent runs inside the customer's Hybris JVM).
- Compile with the same javac Hybris uses (it sets `build.compiler=modern`, i.e. javac); output stays in memory until the
  scheduler approves it; only error-free, non-held classes are written or swapped.
- Hybris option is `tomcat.javaoptions` (not `generaljavaoptions`); the server starts through the Tanuki wrapper.
- A standard JVM cannot swap added methods/fields/lambdas; that must be reported as `[restart required]`, never hidden.
- Build: `java Build.java`. Tests: `java it/IntegrationTest.java` (`HOTDROP_WATCH=poll|native|auto`).
