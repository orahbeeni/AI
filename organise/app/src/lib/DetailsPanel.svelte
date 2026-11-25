<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { app, ago, dirOf, shortPath } from "./state.svelte";
  import { linkIcon, linkMenu } from "./linkActions";
  import Icon from "./Icon.svelte";

  const f = $derived(app.detailsId ? app.find(app.detailsId) : null);
  const isList = $derived(!!f && !f.parent);
  const col = $derived(f ? app.columnOf(f.node.id) : undefined);
  let more = $state(false);

  // width: drag the left edge; remembered between runs
  const MIN = 300;
  const DEFAULT = 380;
  const maxWidth = () => Math.max(MIN, window.innerWidth - 248 - 360);
  function savedWidth(): number {
    try {
      const w = Number(localStorage.getItem("organise.detailsWidth"));
      return w >= MIN ? w : DEFAULT;
    } catch {
      return DEFAULT;
    }
  }
  let width = $state(savedWidth());
  let resizing = $state(false);
  function setWidth(w: number) {
    width = Math.round(Math.min(maxWidth(), Math.max(MIN, w)));
    try {
      localStorage.setItem("organise.detailsWidth", String(width));
    } catch {
      /* not remembered, still works */
    }
  }
  function startResize(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX;
    const startW = width;
    resizing = true;
    const move = (ev: PointerEvent) => setWidth(startW + (startX - ev.clientX));
    const up = () => {
      resizing = false;
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
  function resizeKey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") setWidth(width + 20);
    else if (e.key === "ArrowRight") setWidth(width - 20);
    else return;
    e.preventDefault();
  }

  const trail = $derived.by(() => {
    if (!f) return [] as string[];
    const out: string[] = [];
    let p = f.parent ? app.find(f.parent.id) : null;
    while (p) {
      out.unshift(p.node.text || "Untitled");
      p = p.parent ? app.find(p.parent.id) : null;
    }
    return out;
  });

  const backlinks = $derived.by(() => {
    if (!f) return [];
    const ids = f.node.links;
    const out: { text: string; list: string; id: string; listId: string }[] = [];
    for (const lid of ids)
      for (const u of app.usersOf(lid))
        if (u.node.id !== f.node.id && !out.some((o) => o.id === u.node.id)) out.push({ text: u.node.text, list: u.list.text, id: u.node.id, listId: u.list.id });
    return out;
  });

  async function addFiles(directory: boolean) {
    if (!f) return;
    const picked = await open({ multiple: !directory, directory });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (!paths.length) return;
    try {
      const links = await app.addPaths(paths);
      app.attach(f.node.id, links.map((l) => l.id));
      app.done(`Linked ${links.length === 1 ? links[0].name : `${links.length} files`}. The file stays where it is.`);
    } catch (e) {
      app.say(String(e), "error");
    }
  }

  function addMenu(e: MouseEvent) {
    const id = f?.node.id;
    if (!id) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    app.openMenu({ clientX: r.right - 200, clientY: r.bottom + 4 }, [
      { label: "File…", icon: "file", run: () => addFiles(false) },
      { label: "Folder…", icon: "folder", run: () => addFiles(true) },
      {
        label: "Web link…",
        icon: "globe",
        run: () =>
          app.ask("Web link", "", async (v) => {
            if (!v.trim()) return;
            const url = /^[a-z]+:/i.test(v.trim()) ? v.trim() : `https://${v.trim()}`;
            const l = await app.addUrl(url);
            app.attach(id, [l.id]);
            app.done(`Added a web link to ${l.name}`);
          }, "https://…", "Add"),
      },
    ]);
  }

  function lastMove(linkId: string) {
    return app.moves.find((m) => m.linkId === linkId);
  }
</script>

{#if f}
  <aside class="panel" style:width="{width}px" class:resizing>
    <button
      type="button"
      class="resizer"
      aria-label="Resize the panel (arrow keys)"
      title="Drag to resize · double-click to reset"
      onpointerdown={startResize}
      ondblclick={() => setWidth(DEFAULT)}
      onkeydown={resizeKey}
    ></button>
    <div class="inner">
    <div class="top">
      <span class="trail faint">{trail.join("  ›  ")}</span>
      <button class="icon-btn" title="Close (Esc)" onclick={() => (app.detailsId = null)}><Icon name="close" size={17} /></button>
    </div>
    <input
      class="title"
      value={f.node.text}
      placeholder={isList ? "Untitled list" : "Untitled item"}
      oninput={(e) => app.setText(f.node.id, e.currentTarget.value)}
    />

    <div class="head">
      <span class="section-label">Links</span>
      <button class="add" onclick={addMenu}><Icon name="plus" size={13} /> Add</button>
    </div>
    {#if !f.node.links.length}
      <p class="faint small">Drag files or folders onto the item, or use Add.</p>
    {/if}
    {#each f.node.links as lid (lid)}
      {@const l = app.links[lid]}
      {#if l}
        {@const mv = lastMove(lid)}
        <div class="link" class:missing={l.status !== "ok"}>
          <span class="ico"><Icon name={l.status === "ok" ? linkIcon(l) : "warning"} size={19} /></span>
          <button class="info" onclick={() => (l.status === "ok" ? app.openLink(lid) : (app.view = "links"))} title={l.status === "ok" ? "Open" : "See what was found"}>
            <span class="name">{l.name}</span>
            <span class="sub">
              {#if l.kind === "url"}
                {l.url}
              {:else if l.status === "searching"}
                Looking for it…
              {:else if l.status === "missing"}
                Can't be found · was in {shortPath(dirOf(l.path), app.home)}
              {:else}
                {shortPath(dirOf(l.path), app.home)}{#if mv}&nbsp;· moved {ago(mv.at)}{/if}
              {/if}
            </span>
          </button>
          <button class="icon-btn" title="More" onclick={(e) => app.openMenu(e, linkMenu(l, f.node.id))}><Icon name="more" size={16} /></button>
        </div>
      {/if}
    {/each}

    <div class="head"><span class="section-label">Notes</span></div>
    <textarea
      class="notes"
      value={f.node.notes}
      placeholder="Add notes…"
      oninput={(e) => app.setNotes(f.node.id, e.currentTarget.value)}
    ></textarea>

    <button class="more" onclick={() => (more = !more)}>
      <span>
        <strong>More details</strong>
        {#if !more}<span class="faint small">{isList ? "Category, board column, delete" : "Type, board column, delete"}</span>{/if}
      </span>
      <Icon name={more ? "chevron-down" : "chevron-right"} size={16} />
    </button>
    {#if more}
      <div class="details">
        {#if isList}
          <label>
            <span class="muted">Category</span>
            <select value={f.node.category ?? ""} onchange={(e) => app.setCategory(f.node.id, e.currentTarget.value || null)}>
              {#each app.doc.categories.filter((c) => !c.archived || c.id === f.node.category) as c (c.id)}<option value={c.id}>{c.name}{c.archived ? " (archived)" : ""}</option>{/each}
              <option value="">None</option>
            </select>
          </label>
        {:else}
          <label>
            <span class="muted">Type</span>
            <select value={f.node.kind} onchange={() => app.toggleKind(f.node.id)}>
              <option value="task">Checkbox</option>
              <option value="bullet">Bullet</option>
            </select>
          </label>
        {/if}
        <label>
          <span class="muted">Board</span>
          <select
            value={col?.id ?? ""}
            onchange={(e) => (e.currentTarget.value ? app.putOnBoard(f.node.id, e.currentTarget.value) : app.takeOffBoard(f.node.id))}
          >
            <option value="">Not on the board</option>
            {#each app.doc.columns as c (c.id)}<option value={c.id}>{c.name}</option>{/each}
          </select>
        </label>
        {#if backlinks.length}
          <div class="backlinks">
            <span class="muted">Same links also on</span>
            {#each backlinks as b (b.id)}
              <button class="bl" onclick={() => app.openList(b.listId, b.id)}>{b.text || "Untitled"} <span class="faint">· {b.list}</span></button>
            {/each}
          </div>
        {/if}
        <button
          class="btn danger"
          onclick={() => {
            const id = f.node.id;
            app.detailsId = null;
            app.deleteNode(id);
          }}><Icon name="trash" size={14} /> Delete {isList ? "list" : "item"}</button
        >
      </div>
    {/if}
    </div>
  </aside>
{/if}

<style>
  .panel {
    position: relative;
    flex: none;
    display: flex;
    border-left: 1px solid var(--line);
    background: var(--bg);
  }
  .inner {
    flex: 1;
    min-width: 0;
    padding: 14px 20px 24px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .resizer {
    position: absolute;
    top: 0;
    bottom: 0;
    left: -5px;
    width: 9px;
    cursor: col-resize;
    z-index: 5;
  }
  .resizer::after {
    content: "";
    position: absolute;
    left: 4px;
    top: 0;
    bottom: 0;
    width: 2px;
    background: transparent;
    transition: background 0.15s;
  }
  .resizer:hover::after,
  .resizer:focus-visible::after,
  .resizing .resizer::after {
    background: var(--accent);
  }
  .resizing,
  .resizing * {
    cursor: col-resize !important;
    user-select: none;
  }
  .top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 30px;
  }
  .trail {
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .title {
    font-size: 21px;
    font-weight: 700;
    border: 0;
    outline: 0;
    background: none;
    padding: 2px 0 8px;
    width: 100%;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 12px;
  }
  .add {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12.5px;
    color: var(--accent);
    padding: 3px 6px;
    border-radius: 6px;
  }
  .add:hover {
    background: var(--hover);
  }
  .small {
    font-size: 12.5px;
    margin: 0;
  }
  .link {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 6px 10px 12px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--chip) 70%, var(--bg));
  }
  .link .ico {
    color: var(--accent);
    display: grid;
  }
  .link.missing {
    background: var(--amber-bg);
  }
  .link.missing .ico {
    color: var(--amber);
  }
  .info {
    flex: 1;
    min-width: 0;
    text-align: left;
    display: flex;
    flex-direction: column;
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: 12px;
    color: var(--text2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .notes {
    flex: 1 1 auto;
    min-height: 140px;
    resize: vertical;
    border: 0;
    border-radius: 10px;
    padding: 10px 12px;
    background: color-mix(in srgb, var(--chip) 70%, var(--bg));
    outline: none;
    line-height: 1.5;
  }
  .more {
    margin-top: 14px;
    padding-top: 12px;
    border-top: 1px solid var(--line);
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    text-align: left;
    color: var(--text2);
  }
  .more > span {
    display: flex;
    flex-direction: column;
    color: var(--text);
  }
  .details {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding-top: 6px;
    align-items: flex-start;
  }
  .details label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    font-size: 12.5px;
  }
  select {
    height: 34px;
    border-radius: 8px;
    border: 1px solid var(--line);
    background: var(--bg);
    padding: 0 8px;
    font-size: 14px;
  }
  .backlinks {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 12.5px;
    width: 100%;
  }
  .bl {
    text-align: left;
    padding: 4px 0;
    color: var(--accent);
    font-size: 13.5px;
  }
</style>
