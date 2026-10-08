# HotDrop

Save a `.java` file, and the running SAP Commerce (Hybris) server picks up the new code without a restart.
Pure JDK, no dependencies, same on **Linux, macOS and Windows**. See [PLAN.md](PLAN.md) for the design and the verification log.

## What it does
- Watches the `src` and `web/src` folders of your custom extensions.
- Compiles changed files with javac (the compiler Hybris itself uses), warm in memory.
- Swaps the new classes into the running JVM through a tiny agent (`redefineClasses`).
- **Broken code is held back.** A file that doesn't compile stays pending and is retried on every later save, in any
  file. A file that depends on a broken class is held too. When everything compiles, the batch goes live together.
- Writes the compiled classes to the extension's `classes/` folder, so a restart without `ant build` still has them.

## Build
Needs a **JDK 17+** (not a JRE). The same command on every OS:

```
java Build.java
```
Produces `build/hotdrop-agent.jar` and `build/hotdrop-daemon.jar`.

## Use with Hybris
### Easiest: one command, or one-time setup
- **`bin/hotdrop up`** - finds the running Hybris server by itself, runs on the server's own JDK (no `JAVA_HOME` needed),
  loads the agent if the server has none (also after a server restart) and starts watching. Builds HotDrop on first use.
- **`bin/hotdrop install`** - one-time: adds the `-javaagent` line to `local.properties` (it shows the change and asks first;
  `--remove` undoes it). Run `ant server` once afterwards. From then on the agent starts the watcher together with the
  server (log: `~/.hotdrop/daemon.log`) and stops it with the server. `hotdrop up` still works: if the watcher is
  already running it shows its log instead of starting a second one. Opt out with `-Dhotdrop.autostart=false`.
  Keep `hotdrop-daemon.jar` next to `hotdrop-agent.jar`. Debug start (`ystartDebug`) uses `tomcat.debugjavaoptions`
  instead of `tomcat.javaoptions`; `install` handles both.

### By hand
1. Add the agent to `hybris/config/local.properties` (use forward slashes, also on Windows):
   ```properties
   # Linux / macOS
   tomcat.javaoptions=-javaagent:/opt/hotdrop/hotdrop-agent.jar
   # Windows
   # tomcat.javaoptions=-javaagent:C:/hotdrop/hotdrop-agent.jar
   ```
   Then regenerate the server config with `ant server` and start Hybris as usual.
   (`hybrisserver debug` uses `tomcat.debugjavaoptions` instead.)
   No restart wanted yet? Start the server as usual and run `hotdrop attach --hybris <dir>` (see limits below).
2. Run the daemon with **the same JDK that builds Hybris**:
   ```
   bin/hotdrop start --hybris /path/to/hybris          # Linux, macOS
   bin\hotdrop.cmd start --hybris C:\path\to\hybris    # Windows
   ```
3. Edit and save. The console shows each cycle:
   ```
   17:04:34.463 [ok] swapped Service  (compile 75ms, send 39ms, total 120ms)
   17:04:34.547 ! [broken] Service.java - 1 error(s), kept pending until it compiles
   17:04:35.130 ! [held] Service.java - waiting on demo.Extra
   17:04:35.939 ! [restart required] demo.Service - UnsupportedOperationException: ... attempted to add a method
   ```

### Other commands
`swap <files>` compile and swap once, then exit - `attach` - `status` - `flush [file]` - `pause` / `resume` - `rescan` - `stop` - `doctor`.
`doctor` checks the JDK, the Hybris settings, whether the agent is in `wrapper.conf`, and the connected server.

## In the server log
The agent prints one line per class to the server console (so it lands in the Hybris log, like JRebel does):
```
[HotDrop] Reloaded class 'za.co.shoprite.secfacade.product.converters.populator.SecProductBasicPopulator'
[HotDrop] Compiled class 'x.NewClass' (not loaded yet, used when first needed)
[HotDrop] Could not reload class 'x.Y' - restart required: a method, constructor or lambda was added (...)
[HotDrop] Not reloaded: Y.java does not compile (1 error(s)), first: Y.java:28:9: cannot find symbol ... - kept pending until it compiles
[HotDrop] Held back: Z.java - waiting on x.Y
```
Silence them with `-Dhotdrop.log=false` or the agent option `quiet=true`. `-Dhotdrop.debug=true` adds a timing line per swap.

## What can be swapped
| Change | Standard JVM | JetBrains Runtime with `-XX:+AllowEnhancedClassRedefinition` |
|---|---|---|
| Method body, new class | yes | yes |
| New/removed method, field or **lambda**, changed supertypes | no, reported as `[restart required]` | yes (opt-in, not yet tested on a real Hybris) |

## Platforms
| | Linux | macOS | Windows |
|---|---|---|---|
| File watching | native (inotify) | stat polling every 100 ms (the JDK's own watcher polls slowly there) | native (ReadDirectoryChangesW) |
| Locked files | - | - | reads and class writes retry briefly (editors, antivirus) |
| Launcher | `bin/hotdrop` | `bin/hotdrop` | `bin\hotdrop.cmd` |

`--watch auto|native|poll` overrides the choice. If the OS refuses native watches (for example the Linux inotify limit),
HotDrop adds the polling watcher automatically.
Only Linux has been run by hand so far; the CI workflow in `../.github/workflows/hotdrop.yml` runs the full
end-to-end test on Linux, macOS and Windows with Java 17 and 21, and both watcher modes.

## Tests
```
java Build.java
java it/IntegrationTest.java          # HOTDROP_WATCH=poll to test the macOS watcher
```
The test starts a JVM with the agent that loads classes through a `URLClassLoader` (as Hybris does), starts the daemon,
edits files, and checks what the running JVM does: body change, pending file, held dependent, release together, constant
ripple, an unswappable change reported clearly, and the CLI.

## Known limits (today)
- Not yet run against a real Hybris server (planned first milestone, see PLAN.md). Verified against the real 2205.6 platform
  jars for compiling and against a stand-in server JVM for swapping.
- Spring-aware reloading (new fields, controller mappings, `*-spring.xml`) and the IntelliJ plugin are not built yet.
- `hotdrop attach` can't register Spring contexts created before it attached (not relevant until the Spring layer exists).
- The compile classpath comes from the server's classloaders (or a scan of `hybris/bin` without one); the per-extension
  strict classpath that would cut compile time further is not implemented yet. Measured: ~230 ms per save on the full
  1,089-jar classpath, ~125 ms expected with it.
