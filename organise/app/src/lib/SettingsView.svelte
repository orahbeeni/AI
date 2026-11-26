<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open, save } from "@tauri-apps/plugin-dialog";
  import { app, PALETTE, shortPath } from "./state.svelte";
  import Icon from "./Icon.svelte";
  import { categoryMenu } from "./categoryMenu";
  import type { Category, Column, LoadResult, TrackerStats } from "./types";

  let advanced = $state(false);
  let stats = $state<TrackerStats | null>(null);

  $effect(() => {
    if (advanced) invoke<TrackerStats>("tracker_stats").then((s) => (stats = s));
  });

  async function exportBackup() {
    const d = new Date().toISOString().slice(0, 10);
    const dest = await save({ defaultPath: `${app.home}/Organise-backup-${d}.zip`, filters: [{ name: "Zip", extensions: ["zip"] }] });
    if (!dest) return;
    await app.flush();
    try {
      const n = await invoke<number>("export_backup", { dest });
      app.say(`Backup saved (${n} files)`);
    } catch (e) {
      app.say(String(e), "error");
    }
  }

  async function restoreBackup() {
    const src = await open({ multiple: false, filters: [{ name: "Zip", extensions: ["zip"] }] });
    if (typeof src !== "string") return;
    app.confirm = {
      title: "Restore this backup?",
      body: "Your lists, board and settings are replaced by the backup's. What's there now is kept in .history inside the data folder.",
      okLabel: "Restore",
      danger: true,
      ok: async () => {
        await app.flush();
        try {
          const r = await invoke<LoadResult>("restore_backup", { src });
          app.apply(r);
          app.say("Backup restored");
        } catch (e) {
          app.say(String(e), "error");
        }
      },
    };
  }

  function catMenu(e: MouseEvent, c: Category) {
    app.openMenu(e, categoryMenu(c));
  }

  function columnMenu(e: MouseEvent, c: Column) {
    const n = c.cards.length;
    app.openMenu(e, [
      { label: "Rename", icon: "edit", run: () => app.ask("Rename column", c.name, (v) => app.renameColumn(c.id, v), "", "Rename") },
      { separator: true, label: "" },
      { label: "Delete column", icon: "trash", danger: true, disabled: n > 0, hint: n ? `Move its ${n} ${n === 1 ? "card" : "cards"} out first` : undefined, run: () => app.deleteColumn(c.id) },
    ]);
  }

  const theme = $derived(app.doc.settings.theme ?? "system");
  function setTheme(v: "system" | "light" | "dark") {
    if (v !== theme) app.change(() => (app.doc.settings.theme = v));
  }

  const cardOpens = $derived(app.doc.settings.cardOpens ?? "popup");
  function setCardOpens(v: "popup" | "page") {
    if (v === cardOpens) return;
    app.change(() => (app.doc.settings.cardOpens = v));
    app.done(v === "popup" ? "Clicking a card now opens it in a pop-up" : "Clicking a card now opens it as a full page");
  }

  async function addWatched() {
    const dir = await open({ directory: true, multiple: false });
    if (typeof dir !== "string") return;
    const p = shortPath(dir, app.home);
    if (app.doc.settings.watched.includes(p)) return;
    app.change(() => app.doc.settings.watched.push(p));
    app.done(`Now watching ${p}. Moves into it are seen instantly.`);
    setTimeout(() => invoke<TrackerStats>("tracker_stats").then((s) => (stats = s)), 1500);
  }
</script>

<div class="view">
  <div class="content">
    <h1>Settings</h1>

    <div class="section-label">Your data</div>
    <div class="row">
      <div class="label">
        <span>Data folder</span>
        <span class="muted small">{shortPath(app.dataDir, app.home)} · plain text: Markdown + YAML</span>
      </div>
      <button class="btn" onclick={() => invoke("open_data_dir").catch((e) => app.say(String(e), "error"))}><Icon name="folder" size={14} /> Open</button>
    </div>
    <div class="row">
      <div class="label">
        <span>Backup</span>
        <span class="muted small">A .zip of the data folder. Restoring keeps a copy of what's there now.</span>
      </div>
      <button class="btn primary" onclick={exportBackup}><Icon name="download" size={14} /> Export</button>
      <button class="btn" onclick={restoreBackup}><Icon name="upload" size={14} /> Restore…</button>
    </div>

    <div class="section-label">Appearance</div>
    <div class="row">
      <div class="label">
        <span>Colour mode</span>
        <span class="muted small">{theme === "system" ? "Light or dark, as your computer is set." : theme === "dark" ? "Always dark." : "Always light."}</span>
      </div>
      <div class="seg" role="radiogroup" aria-label="Colour mode">
        <button role="radio" aria-checked={theme === "system"} class:on={theme === "system"} onclick={() => setTheme("system")}>System</button>
        <button role="radio" aria-checked={theme === "light"} class:on={theme === "light"} onclick={() => setTheme("light")}>Light</button>
        <button role="radio" aria-checked={theme === "dark"} class:on={theme === "dark"} onclick={() => setTheme("dark")}>Dark</button>
      </div>
    </div>

    <div class="section-label">Categories</div>
    <div class="pills">
      {#each app.activeCategories as c (c.id)}
        <button class="pill" onclick={(e) => catMenu(e, c)} oncontextmenu={(e) => catMenu(e, c)}>
          <span class="dot" style:background={c.color}></span>{c.name}
        </button>
      {/each}
      <button class="pill add" onclick={() => app.ask("New category", "", (v) => app.addCategory(v), "e.g. Side projects", "Add")}>
        <Icon name="plus" size={13} /> Add
      </button>
    </div>

    {#if app.doc.categories.some((c) => c.archived)}
      <div class="archived-row">
        <span class="faint small">Archived</span>
        {#each app.doc.categories.filter((c) => c.archived) as c (c.id)}
          <button class="pill archived" title="Click for Unarchive" onclick={(e) => catMenu(e, c)} oncontextmenu={(e) => catMenu(e, c)}>
            <span class="dot" style:background={c.color}></span>{c.name}
          </button>
        {/each}
      </div>
    {/if}

    <div class="section-label">Board</div>
    <div class="row">
      <div class="label">
        <span>Clicking a card</span>
        <span class="muted small">
          {cardOpens === "popup"
            ? "Opens it in a pop-up over the board. Open full page in the pop-up makes it the main view."
            : "Opens its list as the main view. Right-click a card for Open in a pop-up."}
        </span>
      </div>
      <div class="seg" role="radiogroup" aria-label="Clicking a card">
        <button role="radio" aria-checked={cardOpens === "popup"} class:on={cardOpens === "popup"} onclick={() => setCardOpens("popup")}>Pop-up</button>
        <button role="radio" aria-checked={cardOpens === "page"} class:on={cardOpens === "page"} onclick={() => setCardOpens("page")}>Full page</button>
      </div>
    </div>
    <div class="sub-label faint small">Columns</div>
    <div class="pills">
      {#each app.doc.columns as c (c.id)}
        <button class="pill" onclick={(e) => columnMenu(e, c)} oncontextmenu={(e) => columnMenu(e, c)}>
          {#if c.color}<span class="dot" style:background={c.color}></span>{/if}{c.name}
          {#if c.cards.length}<span class="faint">{c.cards.length}</span>{/if}
        </button>
      {/each}
      <button class="pill add" onclick={() => app.ask("New column", "", (v) => app.addColumn(v), "e.g. Waiting", "Add")}>
        <Icon name="plus" size={13} /> Add
      </button>
    </div>

    <div class="section-label">Email</div>
    <div class="row">
      <Icon name="mail" size={18} />
      <div class="label">
        <span>Outlook emails</span>
        <span class="muted small">Dragging Outlook emails in comes with the Windows version. Web links work everywhere: paste a URL into any item.</span>
      </div>
    </div>

    <button class="advanced" onclick={() => (advanced = !advanced)}>
      <span>
        <strong>Advanced</strong>
        {#if !advanced}<span class="faint small">Watched folders, file tracking, file format</span>{/if}
      </span>
      <Icon name={advanced ? "chevron-down" : "chevron-right"} size={18} />
    </button>

    {#if advanced}
      <div class="adv">
        <div class="label">
          <span>Watched folders</span>
          <span class="muted small">
            Folders that hold linked files are always watched. Add folders where you often move files to (Downloads, a projects folder)
            and moves into them are seen instantly. Anything else is found by searching from where the file was.
          </span>
        </div>
        {#each app.doc.settings.watched as w, i (w)}
          <div class="watched">
            <Icon name="folder" size={16} />
            <span>{w}</span>
            <button class="icon-btn" title="Stop watching" onclick={() => {
                app.change(() => app.doc.settings.watched.splice(i, 1));
                app.done(`Stopped watching ${w}`);
              }}><Icon name="close" size={14} /></button>
          </div>
        {/each}
        <button class="link-btn" onclick={addWatched}><Icon name="plus" size={14} /> Add folder</button>
        {#if stats}
          <p class="faint small">
            {stats.error ?? `Watching ${stats.watches.toLocaleString()} folders (limit ${stats.budget.toLocaleString()})`}{stats.limited
              ? ". Some sub-folders aren't watched because there are too many; moves there are still found by searching."
              : "."}
          </p>
        {/if}
        <label class="check">
          <input type="checkbox" checked={app.doc.settings.skipHidden} onchange={(e) => app.change(() => (app.doc.settings.skipHidden = e.currentTarget.checked))} />
          Skip hidden folders, node_modules and build caches
        </label>
        <label class="check">
          <input type="checkbox" checked={app.doc.settings.autoSearch} onchange={(e) => app.change(() => (app.doc.settings.autoSearch = e.currentTarget.checked))} />
          Search for a linked file automatically when it goes missing
        </label>
        <div class="row">
          <div class="label">
            <span>File format</span>
            <span class="muted small">One Markdown file per list in lists/, readable in any editor (and Obsidian). Edits made there are read when Organise starts.</span>
          </div>
          <button class="btn" onclick={() => invoke("open_data_dir")}>Show files</button>
        </div>
        <p class="faint small">Organise 0.1 · this computer: {app.host}</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .view {
    height: 100%;
    overflow-y: auto;
  }
  .content {
    max-width: 780px;
    margin: 0 auto;
    padding: 40px 48px 60px;
  }
  h1 {
    font-size: 28px;
    margin: 0 0 26px;
  }
  .section-label {
    margin: 26px 0 10px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 0;
    border-bottom: 1px solid var(--line);
  }
  .label {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .small {
    font-size: 12.5px;
  }
  .seg {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    border-radius: 9px;
    background: var(--chip);
    flex: none;
  }
  .seg button {
    height: 28px;
    padding: 0 12px;
    border-radius: 7px;
    font-size: 13px;
    color: var(--text2);
  }
  .seg button:hover {
    color: var(--text);
  }
  .seg button.on {
    background: var(--bg);
    color: var(--text);
    font-weight: 600;
    box-shadow: 0 1px 3px rgba(15, 23, 42, 0.12);
  }
  .sub-label {
    margin: 14px 0 8px;
  }
  .pills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 30px;
    padding: 0 12px;
    border-radius: 8px;
    background: var(--chip);
    font-size: 13px;
  }
  .pill:hover {
    background: var(--chip-hover);
  }
  .pill.archived {
    background: none;
    border: 1px dashed var(--line-strong);
    color: var(--text2);
  }
  .archived-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-top: 10px;
  }
  .pill.add {
    background: none;
    color: var(--text2);
  }
  .pill.add:hover {
    background: var(--hover);
  }
  .advanced {
    width: 100%;
    margin-top: 30px;
    padding: 14px 0;
    border-top: 1px solid var(--line);
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    text-align: left;
    color: var(--text2);
  }
  .advanced > span {
    display: flex;
    flex-direction: column;
    color: var(--text);
  }
  .adv {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .watched {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 6px 6px 12px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--chip) 70%, var(--bg));
    color: var(--text2);
  }
  .watched span {
    flex: 1;
    color: var(--text);
  }
  .link-btn {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--accent);
    font-size: 13px;
    padding: 4px 0;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .check input {
    accent-color: var(--accent);
    width: 16px;
    height: 16px;
  }
</style>
