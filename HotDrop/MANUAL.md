# HotDrop User Manual

HotDrop reloads your code and configuration into a running SAP Commerce (Hybris) server when you save a file, so you do
not restart the server to see a change. This manual is for developers using it every day. For how it works inside, see
[PLAN.md](PLAN.md); for a short overview, [README.md](README.md).

**How much to trust it today.** Everything here is covered by automated tests that use a stand-in server (a small JVM that
loads classes the way Hybris does, with real Spring 5.3 and 6.2 contexts). It has been run on Linux. It has **not** yet been
run against a real Hybris server, and not on macOS or Windows. Sections marked *(experimental)* depend on guesses about
Hybris that no test could check. Expect to report what differs.

---

## 1. Contents
1. [What you get](#2-what-you-get)
2. [Requirements](#3-requirements)
3. [Install](#4-install)
4. [Start it](#5-start-it)
5. [Your daily workflow](#6-your-daily-workflow)
6. [What happens when you save](#7-what-happens-when-you-save)
7. [Reading the messages](#8-reading-the-messages)
8. [Command reference](#9-command-reference)
9. [Options reference](#10-options-reference)
10. [Settings in the server](#11-settings-in-the-server)
11. [Troubleshooting](#12-troubleshooting)
12. [Limits and things to know](#13-limits-and-things-to-know)
13. [Questions](#14-questions)

---

## 2. What you get

| You change | What HotDrop does | Restart? |
|---|---|---|
| Java method body (in `src`, `web/src`, addon, Backoffice) | compiles it and swaps it into the running JVM | no |
| A new Java class | compiles it, writes it, loads it when first used | no |
| A constant another class uses | recompiles the classes that inlined it, in the same batch | no |
| A Java file with a compile error | keeps it pending, retries on every later save in any file | no |
| Add or remove a method, field or lambda | tells you `[restart required]` and why | yes |
| `*-spring.xml`: a new bean | registers and creates it in the running context | no |
| `*-spring.xml`: a `<property>` value | sets it on the live bean | no |
| `*-spring.xml`: a `<util:list/set/map>` | refills the live collection | no |
| `*-spring.xml`: a bean's class, scope, constructor, parent; removing a bean | tells you `[restart required]` | yes |
| `@RequestMapping` path on a controller | rebuilds that controller's MVC mappings | no |
| Message bundle `.properties` | clears the message-source caches | no |
| `*-items.xml` | tells you to run `ant build`, restart and update the system | yes |
| `*-beans.xml` | tells you to run `ant build` and restart | yes |
| Backoffice `*-backoffice-config.xml` / `-widgets.xml` | tells you Backoffice caches its config *(a note)* | depends |
| `*.impex` you have marked | imports it through HAC *(experimental)* | no |

JSP and tag files are not handled by HotDrop; Tomcat recompiles them itself when `tomcat.development.mode` is on.

HotDrop never hides a change it cannot apply. If it does not say "ok", it says why.

## 3. Requirements
- **A JDK 17 or newer** (not a JRE) to build and run HotDrop. The daemon should run on **the same JDK that builds your
  Hybris** (`hotdrop up` picks the server's own JDK for you). Different JDKs can name lambdas differently, and the swap is
  then rejected.
- Hybris 2205 or 2211 (Spring 5.3) / 2211-jdk21 (Spring 6). Custom extensions under `hybris/bin/custom`.
- Linux, macOS or Windows. Nothing else to install: HotDrop uses only the JDK.

## 4. Install

### 4.1 Build once
From the HotDrop folder, on any operating system:
```
java Build.java
```
This produces `build/hotdrop-agent.jar` and `build/hotdrop-daemon.jar`. Keep the two jars together.

### 4.2 Choose how it starts
**Option A, nothing to configure: `hotdrop up`.** It finds the running Hybris server, runs on the server's JDK, loads the
agent into the server if it has none, and starts watching. You run it each time you start Hybris. Spring support and bean
lookups work best when the agent was there from the start (Option B), because contexts that finished starting before the
agent arrived can be missed.

**Option B, one-time setup: `hotdrop install`.** It adds the agent line to `hybris/config/local.properties` (for a CCv2-style
project that has `config/local-config/`, it writes `config/local-config/99-local.properties` instead), shows you the change
and asks first. Then run `ant server` once. `--hybris` is optional if a server is running and can be found. From then on, HotDrop starts and stops together with the server.
```
bin/hotdrop install --hybris /path/to/hybris        # Linux, macOS
bin\hotdrop.cmd install --hybris C:\path\to\hybris   # Windows
```
`hotdrop install --remove` takes the line out again. The server's debug start (`ystartDebug`) reads
`tomcat.debugjavaoptions`; `install` handles both.

**Option C, by hand.** Put this in `local.properties` (forward slashes, also on Windows), then run `ant server`:
```
tomcat.javaoptions=-javaagent:/opt/hotdrop/hotdrop-agent.jar
```
Start the watcher yourself with `hotdrop start --hybris /path/to/hybris`.

Check the result with `hotdrop doctor --hybris /path/to/hybris` (see [Troubleshooting](#12-troubleshooting)).

## 5. Start it
With Option B, start Hybris as usual; the watcher starts with it (its log is `~/.hotdrop/daemon.log`).

With Option A or C, start Hybris, then:
```
bin/hotdrop up                      # finds the server itself
bin/hotdrop start --hybris <dir>    # when you installed the agent yourself
```
The console prints that it is watching your extensions, the number of source roots, and `save a .java file to compile and
swap it`. After a short indexing pass (a few seconds) you are ready. If you restart Hybris, HotDrop reconnects by itself.

## 6. Your daily workflow
1. Start Hybris and HotDrop once.
2. Edit code, Spring XML or messages in IntelliJ and save.
3. Watch the HotDrop console (or the Hybris log) for the line that tells you what happened. Usually one line, 100 to 300 ms
   after the save.
4. Reload the page or call the service. The new code is running.
5. If it says `[restart required]`, finish the change you are working on, then restart. Nothing was lost: the compiled
   classes were also written to the extension's `classes` folder.

Tips:
- Save often. A file that does not compile does no harm; it waits.
- Several files edited together (a rename, a new class and its user) go live together once all of them compile.
- `hotdrop flush` skips the short wait after a save if you want it now. `hotdrop pause` collects saves without compiling
  (useful during a big git operation); `hotdrop resume` continues.
- After a git branch switch or a pull that changed many files, run `hotdrop rescan`.

## 7. What happens when you save

### 7.1 Java files
HotDrop compiles the saved file with javac (the compiler Hybris uses) and checks it against the files that depend on it.

- **Compiles:** the class is swapped into the running JVM. If the same class is loaded by several web applications, all
  copies are updated.
- **Does not compile:** nothing changes in the server. The file is `[broken]` and stays pending. Fix it and save any file; it
  is retried.
- **Depends on a broken class:** the file is `[held]` until the class it needs compiles. Then both go live together.
- **Changed a constant:** classes that copied the old value (Java inlines `static final` constants) are recompiled for you.
- **Added or removed a method, field or lambda:** the JVM refuses these on a standard JDK. HotDrop reports
  `[restart required]` and which change caused it. The running code is left untouched. (A JetBrains Runtime with
  `-XX:+AllowEnhancedClassRedefinition` allows these; this tier is optional and untested on a real Hybris.)
- Classes are also written to the extension's `classes` (or `WEB-INF/classes`) folder, so a restart without `ant build`
  still has your changes. Code that Backoffice loads from a packed `_bof.jar` is swapped in memory only; run `ant build` before
  you restart.

### 7.2 Spring XML
HotDrop keeps the version of each `*-spring.xml` that the server loaded and compares your saved file with it.

- **New bean:** created in the Spring context that loaded the file. `${placeholders}` are resolved and `@Autowired` fields are
  filled. If the bean cannot be created (for example its class does not exist yet), it is removed again and reported, and
  retried on your next save of that file.
- **Changed property value:** set on the live bean and in its definition. Works through Spring AOP proxies.
- **Changed `<util:list>`, `<util:set>`, `<util:map>`:** the live collection is refilled, so beans that already hold it see the
  change.
- **Anything else** (class, scope, constructor arguments, parent, `init-method`, a removed bean or property, an alias, an import,
  `context:` or other namespace elements): `[restart required]` with the bean's name and reason.
- Beans applied successfully are not applied again on the next save; only the ones that failed are retried.

A new bean is **not** injected into beans that already exist. If another bean collects "all beans of a type" (a list of
converters, for example), it will not see the new bean until a restart.

### 7.3 Controllers
When a swapped class is an MVC controller, HotDrop re-registers its `@RequestMapping` entries, so changing a path or
`produces` takes effect. It does not add a brand-new handler method (that is an added method).

### 7.4 Messages
A `.properties` file in `resources/localization` or in a `WEB-INF/messages` folder clears the message-source caches, so the
next lookup reads the new text. This works for Spring's `ReloadableResourceBundleMessageSource` and
`ResourceBundleMessageSource`. If your storefront uses a different message source, restart.

### 7.5 Model files
`*-items.xml` and `*-beans.xml` cannot be applied live. HotDrop only notices a real content change (saving an unchanged
file is silent) and tells you the next step: run `ant build`, restart, and for `items.xml` update the running system
(HAC, Platform, Update).

### 7.6 Backoffice
Widget controller classes in `backoffice/src` are swapped like any other class. Backoffice keeps its configuration cached,
so a change to `*-backoffice-config.xml` or `*-backoffice-widgets.xml` gets a `[note]`: reload from Backoffice or restart.

### 7.7 ImpEx on save *(experimental)*
You decide per file whether it runs on save, and you turn the feature on once.

1. Start with `--impex` (`hotdrop start --hybris <dir> --impex`).
2. Put a comment line near the top of each ImpEx file you want to run:
   ```
   # hotdrop-on-save
   INSERT_UPDATE Title;code[unique=true];name[lang=en]
   ;dr;Doctor
   ```
3. Save. HotDrop logs into HAC, posts the file to the ImpEx import page (strict validation) and logs the result or HAC's error
   message, here and in the Hybris log.

Rules:
- Files without the marker are never run. (The log says so once per file.)
- The **whole file** runs on every save. Use re-runnable `INSERT_UPDATE` scripts only. A script with `INSERT` or `REMOVE`
  would repeat.
- It uses HAC at `https://localhost:9002/hac` as `admin`. Change them with `--hac`, `--hac-user` and `--hac-password` (or
  the `HOTDROP_HAC_PASSWORD` environment variable; the default password is the development one).
- On your own machine, `http` or a self-signed `https` certificate is accepted. HAC on another machine must be `https` with a
  normal certificate; plain `http` to another machine is refused so the password never travels in clear text.
- Only `resources/impex/**` and `resources/<extension>/import/**` of your custom extensions are watched. Add other folders
  with `--impex-dir <folder>`.
- Saves that arrive while an import is waiting are folded into one run. Imports never delay a class swap.
- HotDrop waits until the file has stopped changing before it reads it.

---

## 8. Reading the messages

**In the HotDrop console (and `~/.hotdrop/daemon.log`):**

| Message | Meaning | What to do |
|---|---|---|
| `[ok] swapped X (compile 75ms, send 39ms, total 120ms)` | class swapped | nothing |
| `[ok] written (not loaded yet) X` | new class is on disk; loaded when first used | nothing |
| `[ok] nothing to swap, class bytes unchanged` | you saved without changing the compiled result | nothing |
| `[broken] X.java - N error(s), kept pending` | does not compile (first errors are listed below) | fix and save |
| `[held] X.java - waiting on ...` | depends on a broken class | fix the other class |
| `[restart required] ...` | a change the running server cannot take | continue, restart when ready |
| `[spring] file: bean 'id' added / updated` | Spring change applied | nothing |
| `[not applied] file: bean 'id': reason` | a bean could not be created or set; stays pending | fix and save that file |
| `[spring] ... no running Spring context has loaded this file` | the server has not loaded the file (not started, or a context was missed) | check the agent is in the server |
| `[note] ...` | information, for example Backoffice config | read it |
| `[impex] file imported in N ms: ...` / `[impex failed] file: ...` | ImpEx result | read the reason |

**In the Hybris server log**, one line per event, starting with `[HotDrop]`:
```
[HotDrop] Reloaded class 'com.acme.facade.DefaultProductFacade'
[HotDrop] Not reloaded: DefaultProductFacade.java does not compile (1 error(s)), first: ...
[HotDrop] Spring bean 'acmeGreeter': greeting updated
[HotDrop] Could not reload class 'x.Y' - restart required: a method, constructor or lambda was added
```

---

## 9. Command reference
Run as `bin/hotdrop <command>` (Linux, macOS) or `bin\hotdrop.cmd <command>` (Windows).

| Command | What it does |
|---|---|
| `up` | The easy way: find the running Hybris, load the agent if needed, watch. |
| `start` | Watch sources and swap on save (use with the agent installed in the server). |
| `install` | Add the agent line to `local.properties`. `--remove` undoes it, `--yes` skips the question. |
| `attach` | Load the agent into an already running server without a restart (partial Spring support). |
| `swap <files>` | Compile and swap the given files once, then exit. |
| `status` | Show the server connection, file states, queue and timings. |
| `flush [file]` | Compile now, without waiting for more saves. |
| `pause` / `resume` | Collect saves without compiling / continue. |
| `rescan` | Re-read the source tree (after a big git operation). |
| `stop` | Stop the watcher. |
| `doctor` | Check the setup and explain what is missing. |

## 10. Options reference
Source options (`start`, `swap`, `doctor`):

| Option | Meaning |
|---|---|
| `--hybris <dir>` | The `hybris` folder (contains `bin/` and `config/`). Extensions, Spring files, messages and ImpEx folders are found from it. |
| `--root <src>=<out>` | A plain source folder and its class output folder (repeatable), for projects that are not a Hybris tree. |
| `--cp <paths>` | Extra classpath when no server is connected. |
| `--spring-dir <dir>` | A folder with `*-spring.xml` files (repeatable). Found automatically with `--hybris`. |
| `--messages-dir <dir>` | A folder with message bundles (repeatable). |
| `--no-spring` | Do not watch Spring XML, model XML, Backoffice config or message bundles. |
| `--impex` | Turn on ImpEx on save (still needs the marker line per file). |
| `--impex-dir <dir>` | A folder with ImpEx files (repeatable). |
| `--hac <url>` | HAC address for ImpEx (default `https://localhost:9002/hac`). |
| `--hac-user <name>` | HAC user (default `admin`). |
| `--hac-password <pw>` | HAC password; prefer the `HOTDROP_HAC_PASSWORD` environment variable (a command-line argument is visible to other users). |

Other options:

| Option | Meaning |
|---|---|
| `--home <dir>` | State folder (default `~/.hotdrop`). |
| `--debounce <ms>` | Quiet time after the last save before compiling (default 40). |
| `--no-index` | Skip the dependency scan at start (faster start, less precise ripple of changes). |
| `--watch auto\|native\|poll` | How files are watched. `auto` uses the operating system's events, and polling on macOS where those are slow. |
| `--poll-ms <ms>` | Polling interval (default 100). |
| `--pid <pid>` | For `attach`: the server JVM to attach to. |
| `--debug` | More detail in the console. |

## 11. Settings in the server
These go to the JVM that runs Hybris (`tomcat.javaoptions`).

Agent options, after the jar path, comma separated: `-javaagent:/opt/hotdrop/hotdrop-agent.jar=quiet=true,spring=false`

| Agent option | Meaning |
|---|---|
| `quiet=true` | Do not print `[HotDrop]` lines to the server console. |
| `spring=false` | Do not look for Spring contexts (disables Spring XML, message and MVC support). |
| `autostart=false` | Do not start the watcher together with the server. |
| `dir=<folder>` | Where the agent leaves its connection file (default `~/.hotdrop/agents`). |

System properties: `-Dhotdrop.log=false` silences the server console lines, `-Dhotdrop.debug=true` adds a timing line per
swap, `-Dhotdrop.autostart=false` stops the watcher starting with the server.

---

## 12. Troubleshooting
Start with `hotdrop doctor --hybris <dir>`. It prints the JDK, the build settings, how many folders are watched, whether
`wrapper.conf` contains the agent, whether `tomcat.development.mode` is on, and whether a server is connected.

| Symptom | Likely cause and fix |
|---|---|
| Nothing happens when I save | `hotdrop status`: is a server connected? If "not connected", the agent is not in the server (run `up`, or `install` and `ant server`). Is your extension in `localextensions.xml` and under `bin/custom`? |
| `no agent connected` | The server runs without the agent, or was started after the watcher with a different home folder. Check `--home` is the same for both. |
| Every swap is rejected with a name mismatch | The daemon runs on a different JDK than the one that built Hybris. Use `hotdrop up` or start HotDrop with the Hybris JDK. |
| `[broken]` but my file looks fine | The error list below the message shows the first errors. A missing class from another extension usually means that extension is not under `bin/custom` or not in `localextensions.xml`. |
| `cannot watch` / events missing on Linux | The inotify limit is too low. Raise `fs.inotify.max_user_watches` (doctor shows the current value). HotDrop adds polling for what it could not watch. |
| Slow saves on macOS | The JDK's native file watcher polls every few seconds there, so HotDrop polls itself (`--poll-ms`). Raise it if the CPU use bothers you. |
| `[spring] ... no running Spring context has loaded this file` | The server has not loaded that file, or the agent missed the context. Start the agent together with the server (Option B). |
| A new bean does not appear in a list that collects beans | Existing beans are not re-wired. Restart. |
| ImpEx file never runs | Both `--impex` and the `# hotdrop-on-save` line are needed; the file must be under a watched folder. |
| `[impex failed] ... HAC login failed` | Wrong `--hac-user` / password, or the address is not HAC. |
| Message change not visible | Your storefront's message source may not be Spring's reloadable one. Restart, or check the cache settings of that bean. |
| After a restart my change is gone | Run `ant build` if the class is loaded from a jar (Backoffice `_bof.jar`); otherwise check that `classes` folder is writable. |

## 13. Limits and things to know
- **Not proven on a real Hybris yet.** See the note at the top. The first run on your project is also a test of the tool.
- Adding or removing methods, fields and lambdas needs a restart on a standard JDK.
- A swap changes code, not state: objects keep their field values, and static initializers do not run again.
- Spring: new beans are not injected into existing beans; post-processor beans only take effect after a restart; contexts
  that finished starting before the agent loaded can be missed.
- `items.xml`/`beans.xml` need `ant build` and a restart. They are detected, not applied.
- ImpEx on save is experimental and runs the whole file each time.
- HotDrop watches your custom extensions only (`bin/custom`), not the platform's own.
- It is a development tool. Do not load the agent into a production server.

## 14. Questions
**Does it replace `ant build`?** No. It writes the same classes `ant build` would, so a restart still works without a build
for Java changes, but model files and anything packed into a jar still need `ant build`.

**Will it change my files?** Only the class output folders. `hotdrop install` edits `local.properties` (or the CCv2
`local-config/99-local.properties`) after showing you the change and asking. Nothing else in your source tree is modified.

**What if I stop HotDrop?** The server keeps running with whatever was swapped. Stopping the watcher does not undo
anything; a restart returns to the code on disk (which already contains your compiled changes).

**Is it safe to leave on?** The agent listens only on `127.0.0.1` and requires a random token that sits in a file readable
only by you. Do not use it on a shared production server.

**Which Spring versions?** 5.3 and 6.2 are covered by tests. The Spring code uses reflection only, so it adapts to whichever
Spring the server has.

**Where are the logs?** The console of `hotdrop start`, `~/.hotdrop/daemon.log` when the agent started the watcher, and
the Hybris log for the `[HotDrop]` lines.

**How do I run the tests?** `java Build.java`, then `java it/IntegrationTest.java`, `java it/SpringIntegrationTest.java`
(needs Spring jars, see the README) and `java it/ResourcesIntegrationTest.java`.
