# HotDrop — Implementation Plan

> Save a `.java` file in IntelliJ → HotDrop compiles it → the running SAP Commerce (Hybris) server picks up the new code in roughly a quarter of a second. No restart, no debugger session, no `ant build`.

**Revision 2 (2026-10-08).** Every assumption in revision 1 has been checked against a real Hybris installation, small experiments on JDK 17, and public sources. See **§0 Verification log** for what was confirmed, what was wrong and has been fixed, and what is still open.

---

## 0. Verification log

Evidence came from four places:
- a local SAP Commerce **2205.6** installation (`~/work/HybrisBackups/...`),
- the `core-customize` project's manifest (**2211.46**),
- experiments run on **Temurin 17.0.18**,
- web sources (listed at the end).

### ✅ Confirmed
| # | Assumption | Evidence |
|---|---|---|
| V1 | `Instrumentation.redefineClasses` swaps a **method-body** change in a running JVM | Experiment: `greet()` returned the new value straight after the redefine |
| V2 | A batch redefine is **atomic** | Experiment: the batch held a valid change to `Other` and an invalid change to `Target`. The JVM rejected it, and **neither** class changed |
| V3 | A standard HotSpot JVM **rejects a newly added lambda**, because a lambda adds a synthetic method | Experiment: `UnsupportedOperationException: class redefinition failed: attempted to add a method` |
| V4 | Extension `classes/` folders are on the runtime classpath, **ahead of** `bin/<ext>server.jar` | `PlatformInPlaceClassLoader.addExtensionURLs` (read with `javap`) adds, per extension: `resources/`, `lib/*.jar`, `classes/` (if it exists), then `bin/<ext>server.jar`. Class files HotDrop writes to `classes/` therefore win for new classes |
| V5 | The platform classloader is a `URLClassLoader` | `PlatformInPlaceClassLoader extends YURLClassLoader extends java.net.URLClassLoader` |
| V6 | Web extensions have their own Tomcat webapp loader | `de.hybris.tomcat.HybrisWebappLoader$HybrisWebappClassLoader extends org.apache.catalina.loader.WebappClassLoader` (`URLClassLoader` underneath) |
| V7 | A brand-new class can be defined into a foreign `URLClassLoader` | Experiment: `MethodHandles.privateLookupIn(neighbour, lookup()).defineClass(bytes)` put the class in the target loader, and `loadClass` then found it |
| V8 | Spring contexts can be reached through Hybris APIs | `de.hybris.platform.core.Registry` has `getCoreApplicationContext()` and `getGlobalApplicationContext()` (from `coreserver.jar`) |
| V9 | JetBrains Runtime supports `-XX:+AllowEnhancedClassRedefinition` | JBR README. Active lines are JBR 17, 21 and 25. Since JBR 21, HotswapAgent is **no longer bundled**, which doesn't affect HotDrop |
| V10 | The JDK's `WatchService` **polls** on macOS | OpenJDK mailing list (JDK-8293067). Native FSEvents support is not in mainline, so a native watcher library is still needed |

### ❌ Wrong in revision 1, now fixed
| # | Rev-1 claim | Reality | Fix in this revision |
|---|---|---|---|
| F1 | "Hybris builds with ECJ" | `advanced.properties` sets **`build.compiler=modern`**, which means **javac**. (`ecj-3.15.1.jar` is only on the `yjavac` compiler classpath and is not used.) | **javac via `javax.tools` is now the only compiler.** ECJ is dropped. ECJ names lambdas differently (`lambda$0` and not `lambda$greet$0`), so on a standard JVM its output would be rejected for almost every class that uses streams |
| F2 | "Single-file compile 30–100 ms" | Measured warm javac against the real Hybris classpath:<br>• **~205 ms** with all 1,089 jars<br>• **~190 ms** with all 1,089 jars and HotDrop's package-listing cache<br>• **~125 ms** with the 348 platform jars and the cache | Latency budget revised (§8). Compile against each extension's **strict classpath** (Hybris's own `build.strict.compilation.mode=true`) and cache package listings |
| F3 | Agent option goes in `tomcat.generaljavaoptions` | That property **doesn't exist**. Hybris has `tomcat.generaloptions` (a long default value you'd have to copy), plus `tomcat.javaoptions` and `tomcat.debugjavaoptions`, which are appended to it | Use `tomcat.javaoptions` (and `tomcat.debugjavaoptions`), then regenerate `wrapper.conf` with `ant server` or `ant all` |
| F4 | Attach finds the JVM by main class `org.apache.catalina.startup.Bootstrap` | Hybris starts through the Tanuki wrapper. The main class is `org.tanukisoftware.wrapper.WrapperSimpleApp`, and `Bootstrap` is passed as an argument | Find the JVM by its system properties (`PLATFORM_HOME`, `catalina.home`) through `VirtualMachine.getSystemProperties()` |
| F5 | `build.development.mode` controls jars versus class folders | It only controls items.xml deployment checks | Removed. V4 shows class folders are always on the classpath |
| F6 | Targets "2211 on Java 17/21", agent `--release 11` | Two lines exist today:<br>• **2211.x** (your project: 2211.46): Java 17, Spring 5.3, `javax.*`<br>• **2211-jdk21.x** (since Sept 2025): Java 21, **Spring 6, `jakarta.*`**<br>SAP blocks Java 17 builds after **2026-08-31**, so you will move to jdk21 soon | Agent and Spring plugin are built with `--release 17` and must work on **both Spring 5.3 and Spring 6**. They use only APIs present in both, and reflection for anything that differs. Integration tests run against both |
| F7 | Implicitly: "a compile with errors produces no output" | Experiment: javac **still wrote `DefaultAcmeService.class`** while `Broken.java` in the same compile had an error | javac writes to an **in-memory** file manager. The scheduler alone decides what reaches disk or the JVM |

### ⚠️ Still open — must be answered in the M0 spike on your real 2211 server
| # | Open question | Why it matters |
|---|---|---|
| O1 | An end-to-end swap inside a **running Hybris server** (not just a test JVM) | Everything above was checked piece by piece. The full chain hasn't run yet |
| O2 | The exact Tomcat version and webapp classloader class on **2211-jdk21** (expected Tomcat 10.1) | The agent reads URLs through `URLClassLoader.getURLs()`, so the version shouldn't matter. It still needs checking |
| O3 | JBR enhanced-redefinition **pause time** on a real Hybris heap (2–6 GB) | Enhanced redefinition walks the heap. There are also public reports of JVM crashes during enhanced redefinition, so it stays **opt-in** |
| O4 | Whether Hybris runs **cleanly on JBR** (SAP officially supports SapMachine) | JBR is OpenJDK-based, so it's expected to work. It's for local development only |
| O5 | Whether your custom extensions use **annotation processors** (for example Lombok) | They would have to run in HotDrop's compile too, and that costs time |
| O6 | Whether javac from the daemon's JDK produces **byte-identical** synthetic member names to the JDK that ran `ant` | The rule is that the daemon uses the **same JDK** as `ant`, and `doctor` enforces it |

---

## 0.5 Platform requirement: Linux, macOS and Windows

Added after revision 2. Every part must work on all three, with no per-OS install steps beyond the `-javaagent` path.

| Concern | Decision |
|---|---|
| Build | `java Build.java` (plain Java), not a shell script |
| Launchers | `bin/hotdrop` (sh) and `bin/hotdrop.cmd` |
| File watching | Linux/Windows: JDK `WatchService`. **macOS: own stat-polling watcher (100 ms)**, because the JDK's one polls with multi-second latency (V10). Polling is also the automatic fallback if the OS refuses native watches. This replaces the planned `directory-watcher` dependency, keeping HotDrop dependency-free |
| Locked files (Windows) | Retry source reads and class-file replacement with a short back-off |
| Symlinks (macOS `/var`, CCv2 checkouts) | Roots are matched by real path as a fallback; the classpath scan follows links |
| Permissions | POSIX `0600` on the token files where supported, skipped elsewhere (user profile ACLs apply) |
| Paths | Only `java.nio.file.Path`/URIs, no string path math; `-javaagent:` path uses forward slashes on Windows |
| Procfs / OS tools | Never required; `/proc` is only read by `doctor`, and guarded |
| CI | GitHub Actions matrix: {Ubuntu, macOS, Windows} x {Java 17, 21} x {native, poll watcher} runs the end-to-end test |

Status: all of the above is implemented; only Linux has been exercised by hand so far.

---

## 0.6 Build status

| Milestone | State |
|---|---|
| Wire protocol, agent (inventory, atomic redefine with per-class fallback, token + loopback) | built, tested |
| Daemon: discovery of custom extensions, build settings from `advanced.properties`, warm javac, in-memory output | built, tested (also against the real 2205.6 platform jars) |
| Scheduler: broken / held / restart-required, ABI-driven ripple, constant ripple, new classes, queued delivery for a server that connects later | built, tested end to end |
| CLI: start, swap, attach, status, flush, pause, resume, rescan, stop, doctor | built |
| M0 spike on a real Hybris server (O1-O6) | O1, O2, O5, O6 answered 2026-10-08 (live swap + auto-start on a real 2211-jdk21 server, 55 roots; Tomcat 10.1.57 / Spring 6.2.19; no annotation processors; javac output matches ant); O3/O4 (JBR) open |
| Strict per-extension classpath (speed), web/backoffice roots on a real Tomcat | not yet |
| Spring layer (M4): `*-spring.xml` add / property / util-collection changes, cache clearing, MVC mapping refresh | built, tested on Spring 5.3.19 and 6.2.19 with a stand-in context; not yet on a real server (how `Registry` yields contexts, parent of web contexts) |
| IntelliJ plugin, JBR tier verification, field re-injection | not yet |

Measured so far: ~120-190 ms from save to live in the end-to-end test (standard JVM, small project); ~230 ms warm compile
on the full Hybris 2205.6 classpath of 1,089 jars.

---

## 1. Goals and non-goals

### Goals
1. **Compile on save.** Watch the source folders of custom extensions and compile changed `.java` files right away.
2. **Hot swap into the running server.** Redefine loaded classes inside the live Hybris JVM without restarting it.
3. **Handle broken code.** If a file doesn't compile, keep it *pending* and retry it on every later change, in any file, until it compiles. Then swap it.
4. **Speed.** For a single-file method-body change, save to live in **under 300 ms (p50)** on a standard JVM (see §8).
5. **Clean room.** Use only public JDK APIs (`java.lang.instrument`, the Attach API, `javax.tools`) and permissively licensed libraries. No JRebel code, binaries, or decompiled material is used.
6. **Independent of the IDE.** The core works with any editor. IntelliJ gets a plugin for status display and to skip the file watcher's latency.

### Non-goals (v1)
- Production use. This is a local developer tool, and it opens a code-injection channel into the JVM.
- Reloading `items.xml` (needs model generation, `ant build`, and an update of the running system) or `beans.xml` (generated DTOs). HotDrop **detects** these changes and tells you a restart is needed.
- Full JRebel parity across every framework. The target is plain Java, Spring, and Spring MVC as used in Hybris.

---

## 2. Prior art

| Option | What it gives you | Why HotDrop still matters |
|---|---|---|
| IntelliJ debugger "HotSwap" | Method-body swaps after a manual recompile | Needs a JDWP debug session and a manual step, gives no help with broken code, and does nothing for Spring |
| **JetBrains Runtime (JBR) + `-XX:+AllowEnhancedClassRedefinition`** | Free (GPLv2+CPE) JVM that allows adding and removing methods, fields, and lambdas | It's a JVM capability, not a workflow. **HotDrop builds on it as an optional tier** |
| HotswapAgent (GPLv2) | Framework plugins on top of JBR | It doesn't compile on save, doesn't handle broken code, and isn't Hybris-aware. **We do not copy or derive from its code** |

**Honest note for the business case:** JBR plus IntelliJ's debugger HotSwap is free and covers part of the need. HotDrop adds automatic compile on save, tolerance for broken code, swaps without a debugger, and knowledge of Hybris and Spring.

---

## 3. Language and technology choice

**Java throughout.** The daemon runs on the same JDK as your `ant` build. The agent and Spring plugin are built with `--release 17`. The IntelliJ plugin is Java. Gradle (Kotlin DSL) is the build tool.

- The in-server part **must** be a Java agent. Only the agent gets `Instrumentation`.
- The compiler is the JDK's own javac (`javax.tools.JavaCompiler`), running in a warm, long-lived JVM. It is the **same compiler Hybris uses** (F1), so the swapped bytecode has the same shape as what's loaded.

| Library | Licence | Purpose |
|---|---|---|
| JDK `javax.tools` + `com.sun.source` (Compiler Tree API) | part of the JDK | Compiling, diagnostics, and collecting dependency data through a `TaskListener` |
| ASM | BSD-3 | Class-file reading, ABI hashing, and the agent's bytecode rewriting (shaded and relocated) |
| *(none)* | - | File watching: JDK `WatchService` on Linux/Windows plus an own polling watcher for macOS (see section 0.5). No third-party dependencies at all |

No ECJ, no JSON library, no web framework, no file-watching library. The wire protocol is hand-written binary.

---

## 4. Architecture

```
┌──────────────────────── Developer machine ───────────────────────────┐
│  IntelliJ ──save──▶ filesystem ◀── native watch ──┐                  │
│     │ (plugin: FLUSH-on-save,                     ▼                  │
│     │  status, errors)            ┌─────────────────────────────────┐│
│     └──── local socket ──────────▶│ HotDrop Daemon (same JDK as ant)││
│                                   │ • watcher + debouncer           ││
│                                   │ • warm javac, in-memory output  ││
│                                   │ • pending/broken scheduler      ││
│                                   │ • dependency graph (Tree API)   ││
│                                   │ • classpath model (from agent)  ││
│                                   └───────────────┬─────────────────┘│
│                                     127.0.0.1 + token, binary frames │
│  ┌─────────────── Hybris JVM (Tanuki wrapper → Tomcat) ─▼──────────┐ │
│  │ HotDrop Agent  (-javaagent via tomcat.javaoptions, or attach)   │ │
│  │ • redefineClasses (one atomic batch)                            │ │
│  │ • classloader inventory → classpath for the daemon              │ │
│  │ • defineClass for brand-new classes when needed                 │ │
│  │ • post-swap hooks: Spring plugin, cache clearing                │ │
│  └─────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────────────┘
```

The daemon runs **outside** the server JVM for three reasons:
- Compiler memory use and any crash stay out of Hybris.
- The daemon stays warm across server restarts. The first compile in a cold JVM took about 1.2–2 s in the benchmarks.
- The IntelliJ plugin and CLI need a stable endpoint.

A loopback round trip costs under 1 ms.

### 4.1 `hotdrop-agent` (inside Hybris)
- One small jar. The manifest sets `Premain-Class`, `Agent-Class`, `Can-Redefine-Classes: true` and `Can-Retransform-Classes: true`. ASM is shaded and relocated to `hotdrop.shaded.asm`. Nothing else is bundled.
- **Startup install** (recommended; Spring context registration needs it). In `hybris/config/local.properties`:
  ```properties
  tomcat.javaoptions=-javaagent:/opt/hotdrop/hotdrop-agent.jar
  tomcat.debugjavaoptions=${tomcat.debugjavaoptions} -javaagent:/opt/hotdrop/hotdrop-agent.jar
  ```
  Then run `ant server` (or `ant all`) to regenerate `wrapper.conf` (F3). Adjust `tomcat.debugjavaoptions` if you've already customised it.
- **Dynamic attach** (`hotdrop attach`): find the JVM by its `PLATFORM_HOME` system property (F4) and load the agent with no restart. On Java 21 this prints the JEP 451 dynamic-agent warning, which you can silence with `-XX:+EnableDynamicAgentLoading`. In this mode Spring support is partial (§7.1).
- On start-up the agent listens on `127.0.0.1` on an ephemeral port. It writes `~/.hotdrop/agents/<pid>.json` (port plus a 256-bit token, mode `0600`).
- Responsibilities:
  1. **Inventory.** Report every `URLClassLoader` that holds application classes: loader ID, parent, `getURLs()`, and its kind (platform `PlatformInPlaceClassLoader`, or a webapp `HybrisWebappClassLoader` with its webapp name). V5 and V6 confirm both are `URLClassLoader`s. Tomcat-specific classes are never referenced directly.
  2. **Redefine.** For each class in a batch, find every loaded `Class` with that name whose `CodeSource` matches the source root's output directory or `bin/<ext>server.jar`. A web class can be loaded once per webapp. Then call **one** `redefineClasses` for the batch. V2 confirms it's all or nothing.
  3. **New classes.** If the loader isn't asking for the class yet, writing it to `<ext>/classes` is enough, because that directory comes first (V4). If a stale copy might shadow it, or it's needed right away, use `privateLookupIn(neighbour).defineClass` (V7).
  4. **Hooks.** Run the post-swap plugins (§7), each inside an error boundary.
  5. **Report back** per class: `SWAPPED`, `REJECTED(message)`, or `NOT_LOADED`.

### 4.2 `hotdrop-daemon`
- Started with `hotdrop start --hybris ~/work/cloud/core-customize/hybris`.
- **Discovery.**
  1. Read `config/localextensions.xml` and each extension's `extensioninfo.xml` (for `requires-extension`, which gives the strict classpath).
  2. Watch extensions under `bin/custom/` by default.
  3. Find each extension's source roots:

  | Source root | Output dir | Runtime loader |
  |---|---|---|
  | `<ext>/src` | `<ext>/classes` | `PlatformInPlaceClassLoader` |
  | `<ext>/web/src` | `<ext>/web/webroot/WEB-INF/classes` | that webapp's `HybrisWebappClassLoader` |
  | `<ext>/backoffice/src` | backoffice classes / `_bof.jar` | backoffice widget loader (M5) |
  | `<ext>/testsrc` | `<ext>/testclasses` | off by default |
  | `<ext>/gensrc` | — | **never watched** |

- **Classpath model.** The authoritative source is the agent's inventory. Each source root is matched to the loader whose URLs contain its output directory. When no server is running, fall back to rebuilding the Hybris layout (`platform/bootstrap/bin`, extension `lib/`, `classes/`, `bin/*server.jar`, `WEB-INF/lib`, and Tomcat `lib/`, mirroring `yjavac`'s classpath in `resources/ant/util.xml`). Use the **strict** per-extension classpath (F2). The model is cached on disk.
- **Compiler.**
  - One `StandardJavaFileManager` per classpath, kept for the daemon's lifetime and wrapped in a `ForwardingJavaFileManager` that **caches `list()` results** for classpath and platform locations. This was measured in the F2 benchmark.
  - `CLASS_OUTPUT` goes to memory (F7).
  - Options match `advanced.properties`: `--release <build.target>`, `-encoding UTF8`, `-g` (`build.debug=on`), `-nowarn`, `-implicit:none`. Use `-proc:none` unless annotation processors are found on the classpath (O5).
  - The source path covers the watched roots, so a file compiles against the **current source** of its siblings.
- **Writes `.class` files** to the output directory after the scheduler approves them, so a restart without `ant build` still runs the new code. Its own writes are ignored (a hash set of what it wrote).
- **Warm-up.** At start-up, compile a few real units once (about 1.2–2 s), so the JIT and file-manager caches are hot before your first save.

### 4.3 `hotdrop-cli`
`hotdrop start | stop | status [--timings] | attach | doctor | swap-now | pause | log`.

`doctor` checks the following:
- the daemon's JDK is the same JDK as `ant`/`JAVA_HOME` (O6),
- redefinition support and the enhanced tier,
- the agent is present in the generated `wrapper.conf`,
- annotation processors on the classpath,
- the inotify watch limit,
- IntelliJ "safe write".

### 4.4 `hotdrop-intellij`
- Status widget: 🟢 live / 🟡 pending (N broken, M held) / 🔴 restart required / ⚪ no server.
- Tool window: broken files with clickable javac errors, held files and what each is waiting on, recent swaps with timings, and rejected classes with the reason.
- When a document is saved, the plugin sends `FLUSH(path)` to the daemon. This skips the debounce window and the watcher, and handles IntelliJ's safe-write rename.
- Actions: Swap now, Pause (for big refactors or `git checkout`), and notifications when a restart is required.

---

## 5. Pending and broken handling (the scheduler)

```
CLEAN ──edit──▶ DIRTY ──compiles──▶ READY ──swap ok──▶ CLEAN
                  │                     └─rejected──▶ RESTART_REQUIRED
                  ├─ errors ─────────▶ BROKEN  (retried every cycle)
                  └─ uses a BROKEN type ▶ HELD (retried every cycle)
```

```
on file events(paths):
    dirty += javaUnits(paths)              // deletes count as changes
    restart debounce (40 ms); a FLUSH from IntelliJ skips it

cycle():
    candidates = dirty ∪ broken ∪ held
    loop:
        result = javac(candidates)  → in-memory class files + diagnostics + reference data
        abiChanged = units whose API hash or constant values changed
        dependents = graph.referencesTo(abiChanged) − candidates
        if dependents is empty: break
        candidates += dependents           // ripple, e.g. inlined static final constants

    broken = units with ERROR diagnostics
    held   = error-free units that reference a type declared in a broken unit
    ready  = candidates − broken − held    // F7: javac may have produced bytes for held units; they're discarded

    batch = [class files of `ready` whose hash ≠ lastSwappedHash]
    write batch to output dirs; send to agent → one atomic redefineClasses
    update states, graph, plugin and CLI
```

- **Reference data** comes from a javac `TaskListener` on `ANALYZE` events. It walks each unit's tree with `Trees` and records every type and **constant field** that unit uses. That data is needed because javac inlines `static final` constants, so class files don't show those dependencies.
- **The "write it before it exists" case.** `Foo` calls `bar.newMethod()` before you've written it, so `Foo` is BROKEN. When you save `Bar`, the cycle retries `Foo`, both compile, and both are swapped in one atomic batch.
- **The "half-written dependency" case.** `Foo` compiles but uses `Baz`, which is broken. `Foo` is HELD, so it never goes live against a half-written dependency.
- **Retries are cheap.** The sets are small, javac is warm, and unchanged bytes are never sent.
- **Policy option.** `hold=strict` (default) or `lenient`. Lenient swaps any unit that compiled cleanly.

---

## 6. What can be swapped (capability tiers)

| Change | Standard HotSpot (SapMachine/Temurin) | JBR + `-XX:+AllowEnhancedClassRedefinition` |
|---|---|---|
| Method body | ✅ (V1) | ✅ |
| Add or remove a method or constructor | ❌ restart | ✅ |
| **Add a lambda** | ❌ restart (V3), a common surprise | ✅ |
| Add or remove a field | ❌ restart | ✅ (new fields start at defaults; the Spring plugin fills injected ones) |
| Change superclass or interfaces | ❌ | ✅ (mostly) |
| Brand-new class | ✅ (V4/V7) | ✅ |
| Annotation change | ✅ bytes; frameworks cache metadata, so the plugin handles it | ✅ + plugin |
| Static initialiser | not re-run | not re-run |

- **Default:** your normal JDK. HotDrop gives a precise message, such as *"restart required: `Foo.greet()` added a lambda — enable the JBR tier to swap this live"*.
- **Opt-in:** JBR 17 (2211.x) or JBR 21 (2211-jdk21) with `-XX:+AllowEnhancedClassRedefinition` in `tomcat.javaoptions`. It stays opt-in until the spike settles O3 and O4.

---

## 7. Framework layer

### 7.1 Finding ApplicationContexts
- **Startup agent:** a `ClassFileTransformer` hooks `AbstractApplicationContext.refresh()` and keeps a weak registry of every context: global, tenant core, and each web extension.
- **Late attach:** `Registry.getCoreApplicationContext()` and `getGlobalApplicationContext()` (V8). Web contexts are found where reachable through servlet context attributes. HotDrop reports this mode as "partial".
- The Spring plugin is loaded in a **child classloader of the loader that defined Spring**, so it links against the server's own Spring, whether 5.3 or 6 (F6).

### 7.2 Spring plugin (M4)
The plugin only uses APIs present in both Spring 5.3 and 6:
1. **Clear caches** after each swap: `ReflectionUtils.clearCache()`, `AnnotationUtils.clearCache()`, `CachedIntrospectionResults.clearClassLoader(...)`.
2. **Re-inject** singletons whose class gained fields (JBR tier) by calling `autowireBeanProperties(target, AUTOWIRE_NO, false)` on the proxy's **target**, and re-apply `<property>` values from the bean definition for new properties. Existing singleton instances are never replaced.
3. **Refresh MVC mappings** when a `@Controller` changes: `unregisterMapping` then `registerMapping` on `RequestMappingHandlerMapping`.
4. **Watch `*-spring.xml` (experimental):**
   - property value changed → set it on the live bean,
   - new bean → register it,
   - class or scope changed, or bean removed → restart required.

### 7.3 Hybris extras (M5)
- `items.xml` or `beans.xml` change → notification: restart + `ant build` needed (plus an update of the running system for `items.xml`).
- Optional reload of localization properties, and optional impex-on-save (opt-in).
- JSP: `tomcat.development.mode=true` is already the default (seen in `project.properties`). `doctor` confirms it.
- Backoffice widget classes.

---

## 8. Performance plan (revised from measurements)

| Step | Budget | Basis |
|---|---|---|
| FS event (inotify/FSEvents), or FLUSH from IntelliJ | 1–10 ms (0 with FLUSH) | — |
| Debounce | 40 ms (0 with FLUSH) | — |
| **javac, 1 file, warm, strict classpath + list cache** | **~125 ms** (≈190 ms on the full classpath) | **Measured** (F2) |
| Dependency and ABI analysis | < 10 ms | — |
| Write and send over loopback | < 3 ms | — |
| `redefineClasses`, standard JVM | 5–50 ms | — |
| `redefineClasses`, JBR enhanced | unknown, possibly hundreds of ms on big heaps | O3 |
| Spring post-hooks | < 20 ms | — |
| **Total p50 target** | **< 300 ms standard** (~200 ms with FLUSH) | — |

For comparison, a Hybris restart takes minutes, so even the slow path is far faster.

Further speed-ups, applied in order and **each measured** before the next:
1. **Strict per-extension classpath plus the cached `list()`.** Already measured at about 125 ms.
2. Fewer files per cycle: only dirty, broken, held, and ripple units are compiled.
3. Skip unchanged bytes.
4. Pre-warm the compile at daemon start, and re-warm after a classpath change.
5. *If still needed:* a symbol-cache experiment. javac re-reads class files on every task, so reuse a single `JavacTask` context across compiles, the way IDEs and Gradle do. This is **research**, not a promise.

Every cycle logs per-stage timings, and `hotdrop status --timings` shows p50/p95.

---

## 9. Risks

| # | Risk | Mitigation |
|---|---|---|
| R1 | A standard JVM rejects anything beyond method bodies, **including new lambdas** (V3) | Clear per-class "restart required" messages. Optional JBR tier |
| R2 | Synthetic-name differences between the daemon's javac and the javac used by `ant` | Same compiler (F1). `doctor` enforces the same JDK (O6). A golden test compiles a fixture both ways and diffs the members |
| R3 | A stale class in `bin/<ext>server.jar` | Low risk: `classes/` comes first (V4). `defineClass` is the fallback (V7) |
| R4 | The same class loaded in several webapp loaders | Redefine every matching `Class` (§4.1) |
| R5 | Classpath drift: new jars in `lib/`, or an `ant` build while the daemon runs | Watch `lib/` and `extensioninfo.xml`. Re-index incrementally. Refresh the inventory on reconnect |
| R6 | `git checkout` touches hundreds of files | Burst detection switches to batch mode. Pause toggle |
| R7 | Security: the agent is effectively remote code execution | Loopback only, a token in a `0600` file, refuses to start on a production `system.mode`, warning banner |
| R8 | Spring proxies, `@Transactional`, AOP | Act on proxy targets. Never replace singletons. Anything ambiguous means restart required |
| R9 | JBR pauses or crashes during enhanced redefinition (O3) | Opt-in tier. Batch redefines. Easy to switch back to the standard JVM |
| R10 | **Spring 5.3 / `javax` versus Spring 6 / `jakarta`** (F6) | Compile against the common API surface and use reflection for the rest. Integration tests on both lines |
| R11 | Annotation processors (Lombok and others) slow down compiles or are missed (O5) | Detect processors and run them only for the files that need them. `doctor` reports them |

---

## 10. Repository layout

```
HotDrop/
├── PLAN.md
├── settings.gradle.kts / build.gradle.kts
├── hotdrop-protocol/       # framed binary messages, --release 17, no deps
├── hotdrop-agent/          # javaagent, --release 17, ASM shaded+relocated
├── hotdrop-agent-spring/   # compileOnly Spring 5.3 API subset; tested on 5.3 and 6
├── hotdrop-daemon/         # watcher, javac driver, scheduler, dep graph, classpath model, CLI
├── hotdrop-intellij/       # IntelliJ Platform Gradle plugin
├── hotdrop-it/             # integration tests (fake Hybris layout, Spring 5.3 + 6 apps, Temurin + JBR)
└── docs/
```

Wire protocol: `[u32 len][u8 type][payload]`.
- `HELLO{token, agentVersion, jvm, enhancedRedefine}`
- `INVENTORY{loaders[]}`
- `REDEFINE{batchId, classes[]}` → `RESULT{perClass[]}`
- `DEFINE{neighbourClass, bytes}`
- `EVENT{...}`

---

## 11. Milestones

### M0 — Spike on your real server (2–3 days)
1. Agent installed through `tomcat.javaoptions` + `ant server` on **2211.46**.
2. A standalone `hotdrop-swap <File.java>` command compiles with javac and the strict classpath, then swaps.
3. A method-body change to a custom `Default…Service` shows up in the storefront or HAC.
4. **Answer O1–O6.** Measure the real end-to-end p50.
5. If JBR 17 is available, measure redefinition pauses and stability.

**Exit criterion:** end-to-end under 400 ms on a real server, with every open question answered. *If something fundamental fails here, we stop or re-plan before writing the rest.*

### M1 — MVP (1–2 weeks)
- Watcher, debounce, warm javac with in-memory output.
- Scheduler (§5).
- Atomic batches.
- CLI with timings.

### M2 — Robustness (1–2 weeks)
- Dependency graph and constant ripple.
- New classes.
- Web extensions.
- JBR tier.
- `doctor`, git bursts, reconnect after a server restart.

### M3 — IntelliJ plugin (1 week)

### M4 — Spring layer (2 weeks), tested on Spring 5.3 and 6

### M5 — Hybris extras (ongoing)

---

## 12. Testing strategy
- **Unit tests:** scheduler state machine (table-driven edit sequences), ABI hashing, protocol.
- **Golden test:** compile fixtures with HotDrop's javac setup and with Hybris's `yjavac` settings, then diff the class files (R2).
- **Integration tests:** a forked JVM with the agent and a small Spring app. Script the edits, then assert the behaviour over HTTP. The matrix is {Temurin 17, Temurin 21, JBR 17, JBR 21} × {Spring 5.3, Spring 6}.
- **Performance regression test:** a scripted single-file edit loop. The build fails if p50 regresses by more than 20%.
- **Manual acceptance checklist on real Hybris:** service, facade, populator, controller, new class, new lambda (expect a restart message on the standard JVM, a live swap on JBR), broken-then-fixed across two files, `git checkout`.

---

## 13. Clean-room statement
- Built only from public JDK specifications and APIs (`java.lang.instrument`, Attach API, `javax.tools`, Compiler Tree API, the JVMS class-file format) and the documentation of the libraries listed above.
- No JRebel binaries, configuration formats, non-public documentation, or decompiled code are used. No code is copied from GPL projects (DCEVM/HotswapAgent). JBR is only used as a separately installed runtime.
- SAP classes were inspected only with `javap`, to learn signatures and classpath order for interoperability. No SAP code is copied.
- All dependencies are Apache-2.0 or BSD.

---

## 14. Next steps
1. Approve this revision.
2. M0 spike on `core-customize` (2211.46). I build the agent plus `hotdrop-swap`, and we prove a live swap and answer O1–O6.
3. Decide on the JBR tier based on the spike's numbers.
4. M1 onward.

---

### Sources
- JetBrains Runtime README: https://github.com/JetBrains/JetBrainsRuntime
- HotswapAgent (JBR 21/25 notes): https://github.com/HotswapProjects/HotswapAgent
- JDK-8293067, macOS WatchService discussion: https://mail.openjdk.org/pipermail/nio-dev/2022-November/012741.html
- SAP Commerce 2211-jdk21 / Spring 6 / Java 17 cutoff: https://www.akkodis.com/blog/articles/sap-commerce-cloud-jdk21-upgrade-guide
- Local evidence: `hybris/bin/platform/resources/advanced.properties`, `project.properties`, `resources/ant/util.xml`, `bootstrap/bin/ybootstrap.jar`, `bootstrap/bin/ytomcat.jar`, `ext/core/bin/coreserver.jar` (2205.6); `core-customize/manifest.json` (2211.46)
