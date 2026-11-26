<script lang="ts">
  import { app, progress } from "./state.svelte";
  import { dnd } from "./dnd.svelte";
  import { setCaret } from "./caret";
  import Row from "./Row.svelte";
  import Icon from "./Icon.svelte";
  import type { MenuItem, Node } from "./types";

  const list = $derived(app.list);
  const cat = $derived(list ? app.category(list.category) : undefined);
  const col = $derived(list ? app.columnOf(list.id) : undefined);
  const p = $derived(list ? progress(list) : { done: 0, total: 0 });

  const rows = $derived.by(() => {
    const out: { node: Node; depth: number }[] = [];
    const add = (nodes: Node[], depth: number) => {
      for (const n of nodes) {
        out.push({ node: n, depth });
        if (!app.collapsed[n.id]) add(n.children, depth + 1);
      }
    };
    if (list) add(list.children, 0);
    return out;
  });

  let titleEl: HTMLDivElement | undefined = $state();
  $effect(() => {
    const t = list?.text ?? "";
    if (titleEl && titleEl.textContent !== t) titleEl.textContent = t;
  });
  $effect(() => {
    const f = app.focus;
    if (f && list && f.id === list.id && titleEl) {
      titleEl.focus();
      setCaret(titleEl, f.caret === "start" ? 0 : (titleEl.textContent?.length ?? 0));
      app.focus = null;
    }
  });

  function titleKey(e: KeyboardEvent) {
    if (!list) return;
    if (e.key === "Enter") {
      e.preventDefault();
      const first = list.children[0];
      if (first) app.focus = { id: first.id, caret: "end" };
      else newItem();
    } else if (e.key === "Escape") titleEl?.blur();
  }

  function newItem() {
    if (!list) return;
    const it = app.append(list.id);
    if (it) app.focus = { id: it.id, caret: "start" };
  }

  function catMenu(e: MouseEvent) {
    if (!list) return;
    const items: MenuItem[] = [
      ...(cat?.archived ? [{ label: `Unarchive ${cat.name}`, icon: "upload", run: () => app.archiveCategory(cat.id, false) }, { separator: true, label: "" }] : []),
      ...app.activeCategories.map((c) => ({ label: c.name, color: c.color, checked: list.category === c.id, run: () => app.setCategory(list.id, c.id) })),
      { label: "None", checked: !list.category, run: () => app.setCategory(list.id, null) },
      { separator: true, label: "" },
      {
        label: "New category…",
        icon: "plus",
        run: () =>
          app.ask("New category", "", (v) => {
            const c = app.addCategory(v);
            if (c) app.setCategory(list.id, c.id);
          }, "e.g. Side projects", "Add"),
      },
    ];
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    app.openMenu({ clientX: r.left, clientY: r.bottom + 4 }, items);
  }

  function listMenu(e: MouseEvent) {
    if (!list) return;
    const l = list;
    const items: MenuItem[] = [
      col
        ? { label: "Take off the board", icon: "board", run: () => app.takeOffBoard(l.id) }
        : { label: "Put on the board", icon: "board", run: () => app.putOnBoard(l.id) },
      {
        label: "Board column",
        icon: "grid",
        submenu: app.doc.columns.map((c) => ({ label: c.name, checked: col?.id === c.id, run: () => app.putOnBoard(l.id, c.id) })),
      },
      {
        label: "Move into another list",
        icon: "arrow-right",
        submenu: app.doc.lists.filter((x) => x.id !== l.id).map((x) => ({ label: x.text || "Untitled", color: app.category(x.category)?.color, run: () => app.move(l.id, x.id, "inside") })),
      },
      { label: "Rename", icon: "edit", run: () => (app.focus = { id: l.id, caret: "end" }) },
      { label: "Details and notes", icon: "info", run: () => (app.detailsId = l.id) },
      { separator: true, label: "" },
      {
        label: "Delete list",
        icon: "trash",
        danger: true,
        run: () =>
          (app.confirm = {
            title: `Delete “${l.text || "Untitled"}”?`,
            body: "Its file moves to the .trash folder inside your data folder. Undo works too.",
            okLabel: "Delete",
            danger: true,
            ok: () => app.deleteNode(l.id),
          }),
      },
    ];
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    app.openMenu({ clientX: r.right - 220, clientY: r.bottom + 4 }, items);
  }

  const endTarget = $derived(
    (dnd.s?.target?.kind === "end" && dnd.s.target.id === list?.id) || (app.fileDrop?.kind === "end" && app.fileDrop.id === list?.id),
  );
</script>

{#if !list}
  <div class="empty">
    <h2>No lists yet</h2>
    <p class="muted">Lists hold your tasks, notes and links to files.</p>
    <button class="btn primary" onclick={() => app.addList()}><Icon name="plus" size={15} /> New list</button>
  </div>
{:else}
  <div class="view">
    <div class="toolbar">
      <button class="icon-btn" title="Details and notes" onclick={() => (app.detailsId = list.id)}><Icon name="info" size={18} /></button>
      <button class="icon-btn" title="More" onclick={listMenu}><Icon name="more" size={18} /></button>
    </div>
    <div class="content">
      <button class="cat" onclick={catMenu}>
        <span class="dot" style:background={cat?.color ?? "var(--text3)"}></span>
        {cat?.name ?? "No category"}
        {#if cat?.archived}<span class="archived">Archived</span>{/if}
        <Icon name="chevron-down" size={12} />
      </button>
      <div
        class="title"
        bind:this={titleEl}
        role="textbox"
        aria-label="List title"
        tabindex="0"
        contenteditable="plaintext-only"
        data-placeholder="Untitled list"
        spellcheck="true"
        oninput={() => app.setText(list.id, titleEl?.textContent ?? "")}
        onkeydown={titleKey}
      ></div>
      <div class="meta">
        {#if col}<span class="board-tag" style:--col={col.color ?? "var(--accent)"} title="On the board in {col.name}"><Icon name="board" size={12} />{col.name}</span>{/if}
        {#if p.total}
          <span>{p.done} of {p.total} done</span>
          <span class="bar"><span style:width="{(100 * p.done) / p.total}%"></span></span>
        {:else}
          <span class="faint">Enter adds an item · Tab nests it · drop files on any item</span>
        {/if}
      </div>

      <div class="rows" role="list">
        {#each rows as r, i (r.node.id)}
          <Row node={r.node} depth={r.depth} prevId={rows[i - 1]?.node.id ?? null} nextId={rows[i + 1]?.node.id ?? null} />
        {/each}
      </div>
      <div class="end" data-list-end={list.id} class:target={endTarget}>
        <button class="add" onclick={newItem}><Icon name="plus" size={15} /> New item</button>
        {#if endTarget && app.fileDrop}
          <span class="drop-hint">Drop to add an item for each file</span>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .view {
    height: 100%;
    overflow-y: auto;
    position: relative;
  }
  .toolbar {
    position: sticky;
    top: 0;
    display: flex;
    justify-content: flex-end;
    gap: 2px;
    padding: 10px 12px 0;
    height: 0;
    overflow: visible;
    z-index: 2;
  }
  .content {
    max-width: 860px;
    margin: 0 auto;
    padding: 44px 48px 0 64px;
    min-height: 100%;
    display: flex;
    flex-direction: column;
  }
  @container main (max-width: 640px) {
    .content {
      padding: 32px 16px 0 40px;
    }
  }
  .cat {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12.5px;
    color: var(--text2);
    padding: 3px 8px;
    margin-left: -8px;
    border-radius: 6px;
  }
  .cat:hover {
    background: var(--hover);
  }
  .archived {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 5px;
    background: var(--chip);
    color: var(--text2);
  }
  .title {
    font-size: 30px;
    line-height: 1.2;
    font-weight: 700;
    margin: 6px 0 8px;
    outline: none;
    letter-spacing: -0.01em;
  }
  .title:empty::before {
    content: attr(data-placeholder);
    color: var(--text3);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 13px;
    color: var(--text2);
    margin-bottom: 22px;
  }
  .board-tag {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 550;
    color: var(--col);
    background: color-mix(in srgb, var(--col) 14%, transparent);
    margin-right: 4px;
  }
  .bar {
    width: 110px;
    height: 5px;
    border-radius: 5px;
    background: var(--chip);
    overflow: hidden;
    margin-left: 6px;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    border-radius: 5px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .end {
    flex: 1;
    min-height: 160px;
    padding: 6px 0 40px;
    border-radius: 8px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }
  .end.target {
    background: var(--sel);
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .add {
    display: inline-flex;
    align-items: center;
    gap: 9px;
    color: var(--text3);
    padding: 6px 8px 6px 20px;
    border-radius: 7px;
    font-size: 14.5px;
  }
  .add:hover {
    color: var(--text);
    background: var(--hover);
  }
  .drop-hint {
    padding-left: 24px;
    color: var(--accent);
    font-size: 13px;
  }
  .empty {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }
  .empty h2 {
    margin: 0;
  }
  .empty p {
    margin: 0 0 12px;
  }
</style>
