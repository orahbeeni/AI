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
Spring layer (M4: `*-spring.xml` beans, util collections, cache clearing, MVC mappings; `SpringBridge` in the agent is reflection-only) is built and tested on Spring 5.3.19 and 6.2.19 with `it/SpringIntegrationTest.java`, but not against a real Hybris server. M5 extras (items/beans notices, message bundle cache clearing, doctor development-mode line) are built; impex-on-save and the backoffice widget loader are not. Not built: IntelliJ plugin, strict per-extension classpath, JBR field re-injection. Not yet done: the spike on a real Hybris server.

## Done on macOS (2026-10-08)
macOS tested: build and integration test pass (polling watcher; native fails because the JDK polls slowly there).
Verified on a real Hybris 2211-jdk21 (Microsoft JDK 21.0.12, CCv2 project, 55 source roots): attach, live swap of a
populator, `hotdrop up`, and the full auto-start path (`-javaagent` in config/local-config/99-local.properties ->
`ant server` -> agent starts the watcher -> live swap seen on the PDP). So O1 is answered: it works end to end.
Added: `up`, `install`, agent auto-start + supervision (`DaemonLauncher`), PLATFORM_HOME fallback (HYBRIS_BIN_DIR /
catalina.home). Real-project bug found and fixed: one javac file manager per root held a full jar index each and ran
the daemon out of memory (3 GB+); now one shared file manager (`RootCompiler.Shared`), ~230 MB.
Done later same day: `install` now prefers config/local-config/99-local.properties; Hybris-log messages ([HotDrop] Reloaded
class ..., Not reloaded ..., via Wire.NOTICE); acceptance on the real server (body change, new class, added lambda -> restart
required, broken then fixed, web controller); real latency: warm swaps 233-245 ms total (compile ~135, server ~100). O2: Tomcat
10.1.57, Spring 6.2.19. O5: no annotation processors in custom code. O6: 426 seccore classes compiled by HotDrop vs ant are
identical under `javap -p -s -c`. Not verified: JBR tier (no JBR installed here), strict per-extension classpath (not needed
for the 300 ms target so far). Note debug start (`ystartDebug`, JDWP) may slow redefine; untested without it.
BUG FOUND AND FIXED: HotDrop did not pass `-parameters` (Hybris: build.parameter-metadata=true), so swapped Spring MVC
controllers lost parameter names -> "Name for argument ... not specified" on @PathVariable. Always compare class BYTES with
the ant build (Golden check: 440/440 identical with -parameters), not just javap -p -s -c.
Review for leaks/robustness (same day): memory stable (watcher ~280 MB heap, 19 MB metaspace after 30 min). Fixed:
polling watcher used 22% CPU idle on macOS (55 roots) -> 3.8% (hot files + rotating sweep; first save of an untouched
file can take up to ~1 s, later saves one tick); poller/native-watcher/worker threads no longer die silently on an
exception; `up`/`attach` only attach to Tomcat-like JVMs (display name), not the IDE or Gradle daemons; daemon.log
rotates at 10 MB; one-watcher check verifies the command line (pid reuse).
Notes: debug start uses tomcat.debugjavaoptions INSTEAD of tomcat.javaoptions. Remaining
PLAN open questions: O2-O6, strict per-extension classpath, JBR tier. Never run the real server's items.xml build issues
past HotDrop - it does not touch items.xml.

## (historical) If you are on macOS (the current task): test it
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
