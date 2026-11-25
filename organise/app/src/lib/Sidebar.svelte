<script lang="ts">
  import { app, walk } from "./state.svelte";
  import { dnd, startDrag } from "./dnd.svelte";
  import Icon from "./Icon.svelte";
  import { categoryMenu } from "./categoryMenu";
  import type { Category, MenuItem, Node } from "./types";

  let searchEl: HTMLInputElement | undefined = $state();

  type Group = { id: string; name: string; color: string; cat: Category | null; lists: Node[] };
  const groups = $derived.by(() => {
    const out: Group[] = app.doc.categories.map((c) => ({ id: c.id, name: c.name, color: c.color, cat: c, lists: [] as Node[] }));
    const other: Node[] = [];
    for (const l of app.doc.lists) {
      const g = out.find((g) => g.id === l.category);
      if (g) g.lists.push(l);
      else other.push(l);
    }
    if (other.length) out.push({ id: "-", name: "Other", color: "var(--text3)", cat: null, lists: other });
    return out;
  });
  const activeGroups = $derived(groups.filter((g) => !g.cat?.archived));
  const archivedGroups = $derived(groups.filter((g) => g.cat?.archived));
  let showArchived = $state(false);

  const results = $derived.by(() => {
    const q = app.query.trim().toLowerCase();
    if (!q) return [];
    const out: { node: Node; list: Node; via?: string }[] = [];
    for (const list of app.doc.lists) {
      walk(list, (n) => {
        if (out.length >= 40) return;
        if (n.text.toLowerCase().includes(q)) out.push({ node: n, list });
        else {
          const l = n.links.map((id) => app.links[id]).find((l) => l?.name.toLowerCase().includes(q));
          if (l) out.push({ node: n, list, via: l.name });
        }
      });
    }
    return out;
  });

  const missing = $derived(app.missingLinks().length);
  const onBoard = $derived(app.doc.columns.reduce((k, c) => k + c.cards.filter((id) => !app.isArchived(id)).length, 0));

  function listMenu(e: MouseEvent, l: Node) {
    const col = app.columnOf(l.id);
    const items: MenuItem[] = [
      { label: "Move up", icon: "arrow-up", shortcut: "Alt ↑", disabled: !app.canMoveList(l.id, -1), run: () => app.moveList(l.id, -1) },
      { label: "Move down", icon: "arrow-down", shortcut: "Alt ↓", disabled: !app.canMoveList(l.id, 1), run: () => app.moveList(l.id, 1) },
      { separator: true, label: "" },
      { label: "Rename", icon: "edit", run: () => { app.openList(l.id); app.focus = { id: l.id, caret: "end" }; } },
      {
        label: "Category",
        icon: "tag",
        submenu: [
          ...app.activeCategories.map((c) => ({ label: c.name, color: c.color, checked: l.category === c.id, run: () => app.setCategory(l.id, c.id) })),
          { label: "None", checked: !l.category, run: () => app.setCategory(l.id, null) },
        ],
      },
      col
        ? { label: "Take off the board", icon: "board", run: () => app.takeOffBoard(l.id) }
        : { label: "Put on the board", icon: "board", run: () => app.putOnBoard(l.id) },
      {
        label: "Move into another list",
        icon: "arrow-right",
        hint: "Its board card stays where it is",
        submenu: app.doc.lists.filter((x) => x.id !== l.id).map((x) => ({ label: x.text || "Untitled", color: app.category(x.category)?.color, run: () => app.move(l.id, x.id, "inside") })),
      },
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
    app.openMenu(e, items);
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      app.query = "";
      searchEl?.blur();
    } else if (e.key === "Enter" && results[0]) {
      app.openList(results[0].list.id, results[0].node.id === results[0].list.id ? undefined : results[0].node.id);
      app.query = "";
    }
  }

  export function focusSearch() {
    searchEl?.focus();
    searchEl?.select();
  }

  function isDropTarget(id: string) {
    const t = dnd.s?.target;
    return (t?.kind === "list" && t.id === id && t.pos === "into") || (app.fileDrop?.kind === "list" && app.fileDrop.id === id);
  }
</script>

<aside class="sidebar">
  <div class="brand">
    <span class="logo"><Icon name="check" size={15} /></span>
    <span>Organise</span>
  </div>

  <label class="search">
    <Icon name="search" size={15} />
    <input bind:this={searchEl} bind:value={app.query} placeholder="Search" onkeydown={keydown} spellcheck="false" />
    {#if app.query}
      <button class="clear" onclick={() => (app.query = "")} aria-label="Clear search"><Icon name="close" size={13} /></button>
    {:else}
      <kbd>Ctrl K</kbd>
    {/if}
  </label>

  {#if app.query}
    <div class="results">
      {#if !results.length}
        <p class="faint empty">No matches</p>
      {/if}
      {#each results as r (r.node.id)}
        <button
          class="result"
          onclick={() => {
            app.openList(r.list.id, r.node.id === r.list.id ? undefined : r.node.id);
            app.query = "";
          }}
        >
          <span class="r-text">{r.node.text || "Untitled"}</span>
          <span class="r-sub faint">{r.node.id === r.list.id ? "List" : r.list.text}{r.via ? ` · ${r.via}` : ""}{app.isArchived(r.list.id) ? " · archived" : ""}</span>
        </button>
      {/each}
    </div>
  {:else}
    <nav class="nav">
      <button class="nav-item" class:active={app.view === "board"} onclick={() => (app.view = "board")}>
        <Icon name="grid" size={17} /> Board <span class="count">{onBoard || ""}</span>
      </button>
    </nav>

    <div class="lists">
      {#snippet listButton(l: Node, dim: boolean)}
        <button
          class="list-item"
          class:dim
          data-list={l.id}
          class:active={app.view === "list" && app.listId === l.id}
          class:drop-into={isDropTarget(l.id)}
          class:drop-before={dnd.s?.target?.kind === "list" && dnd.s.target.id === l.id && dnd.s.target.pos === "before"}
          class:drop-after={dnd.s?.target?.kind === "list" && dnd.s.target.id === l.id && dnd.s.target.pos === "after"}
          onpointerdown={(e) => startDrag(e, "list", l.id, l.text || "Untitled")}
          onclick={() => app.openList(l.id)}
          oncontextmenu={(e) => listMenu(e, l)}
          onkeydown={(e) => {
            if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
              e.preventDefault();
              app.moveList(l.id, e.key === "ArrowUp" ? -1 : 1);
              const id = l.id;
              requestAnimationFrame(() => document.querySelector<HTMLElement>(`[data-list="${id}"]`)?.focus());
            }
          }}
        >
          <span class="grip"><Icon name="grip" size={13} /></span>
          {l.text || "Untitled"}
        </button>
      {/snippet}

      {#each activeGroups as g (g.id)}
        {@const ct = dnd.s?.kind === "cat" && dnd.s.target?.kind === "catblock" && dnd.s.target.id === g.id ? dnd.s.target.pos : null}
        {@const folded = !!app.collapsedCats[g.id]}
        <div class="block" data-cat-block={g.id} class:cat-before={ct === "before"} class:cat-after={ct === "after"} class:lifted={dnd.s?.kind === "cat" && dnd.s.id === g.id}>
          <div
            class="group"
            class:draggable={!!g.cat}
            data-cat={g.id}
            class:cat-target={dnd.s?.target?.kind === "cat" && dnd.s.target.id === g.id}
            role="presentation"
            class:folded
            onpointerdown={(e) =>
              !(e.target as HTMLElement).closest("button") && startDrag(e, "cat", g.id, g.name, () => app.toggleCategory(g.id), !g.cat)}
            oncontextmenu={(e) => g.cat && app.openMenu(e, categoryMenu(g.cat))}
          >
            <button
              class="chev"
              aria-expanded={!folded}
              aria-label="{folded ? 'Expand' : 'Collapse'} {g.name}"
              onclick={() => app.toggleCategory(g.id)}
            >
              <Icon name={folded ? "chevron-right" : "chevron-down"} size={12} />
            </button>
            <span class="dot" style:background={g.color}></span>
            <span class="section-label">{g.name}</span>
            {#if folded}<span class="n faint">{g.lists.length}</span>{/if}
            {#if g.cat}
              <button class="icon-btn more" title="{g.name}: move, archive, rename, colour…" onclick={(e) => g.cat && app.openMenu(e, categoryMenu(g.cat))}>
                <Icon name="more" size={14} />
              </button>
            {/if}
            <button class="icon-btn plus" class:first={!g.cat} title="New list in {g.name}" onclick={() => app.addList(g.id === "-" ? null : g.id)}>
              <Icon name="plus" size={14} />
            </button>
          </div>
          {#if !folded}
            {#each g.lists as l (l.id)}
              {@render listButton(l, false)}
            {:else}
              <div class="empty-cat faint">No lists yet</div>
            {/each}
          {/if}
        </div>
      {/each}
      <button class="new-list" onclick={() => app.newCategory()}><Icon name="plus" size={14} /> New category</button>

      {#if archivedGroups.length}
        <button class="archived-toggle" onclick={() => (showArchived = !showArchived)} aria-expanded={showArchived}>
          <Icon name={showArchived ? "chevron-down" : "chevron-right"} size={13} />
          Archived
          <span class="count">{archivedGroups.length}</span>
        </button>
        {#if showArchived}
          {#each archivedGroups as g (g.id)}
            <div class="group dim" data-cat={g.id} role="presentation" oncontextmenu={(e) => g.cat && app.openMenu(e, categoryMenu(g.cat))}>
              <span class="dot" style:background={g.color}></span>
              <span class="section-label">{g.name}</span>
              <button class="unarchive" onclick={() => g.cat && app.archiveCategory(g.cat.id, false)}>Unarchive</button>
            </div>
            {#each g.lists as l (l.id)}
              {@render listButton(l, true)}
            {:else}
              <div class="empty-cat faint">No lists</div>
            {/each}
          {/each}
        {/if}
      {/if}
    </div>
  {/if}

  <div class="bottom">
    {#if app.searching}
      <div class="status faint"><span class="spinner"></span> Looking for moved files…</div>
    {/if}
    {#if missing}
      <button class="attention" onclick={() => (app.view = "links")}>
        <Icon name="warning" size={14} />
        {missing} {missing === 1 ? "link needs" : "links need"} attention
      </button>
    {/if}
    <button class="nav-item" class:active={app.view === "settings"} onclick={() => (app.view = "settings")}>
      <Icon name="settings" size={17} /> Settings
    </button>
  </div>
</aside>

<style>
  .sidebar {
    width: 248px;
    flex: none;
    background: var(--sidebar);
    display: flex;
    flex-direction: column;
    padding: 14px 10px 10px;
    gap: 4px;
    min-height: 0;
    user-select: none;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 9px;
    font-weight: 700;
    font-size: 16px;
    padding: 2px 8px 10px;
  }
  .logo {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: var(--accent);
    color: #fff;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 34px;
    padding: 0 10px;
    border-radius: 8px;
    background: color-mix(in srgb, var(--text) 6%, transparent);
    color: var(--text3);
    margin-bottom: 8px;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
  }
  .search input::placeholder {
    color: var(--text3);
  }
  kbd {
    font: inherit;
    font-size: 11px;
    color: var(--text3);
  }
  .clear {
    color: var(--text3);
    display: grid;
  }
  .nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 14px;
  }
  .nav-item {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 32px;
    padding: 0 10px;
    border-radius: 7px;
    color: var(--text);
    text-align: left;
    width: 100%;
  }
  .nav-item :global(.icon) {
    color: var(--text2);
  }
  .nav-item:hover {
    background: var(--hover);
  }
  .nav-item.active {
    background: var(--sidebar-sel);
    font-weight: 600;
  }
  .count {
    margin-left: auto;
    font-size: 12px;
    color: var(--text3);
    font-weight: 400;
  }
  .lists {
    padding-top: 4px;
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .group {
    flex: none;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 12px 10px 4px;
    border-radius: 7px;
  }

  .group .icon-btn {
    width: 22px;
    height: 22px;
  }
  .group .more {
    margin-left: auto;
    opacity: 0;
  }
  .group .plus.first {
    margin-left: auto;
  }
  .group:hover .more,
  .group .more:focus-visible {
    opacity: 1;
  }
  .group .plus {
    color: var(--text3);
  }
  .group .plus:hover {
    color: var(--text);
  }
  .group.draggable {
    cursor: grab;
  }
  .group {
    cursor: pointer;
  }
  .group:hover .section-label {
    color: var(--text2);
  }
  .chev {
    width: 14px;
    height: 18px;
    margin-left: -6px;
    margin-right: -3px;
    display: grid;
    place-items: center;
    color: var(--text3);
    flex: none;
    border-radius: 4px;
  }
  .chev:hover {
    color: var(--text);
  }
  .n {
    font-size: 11px;
  }
  .block {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 1px;
    border-radius: 8px;
  }
  .block {
    position: relative;
  }
  .block.cat-before::before,
  .block.cat-after::after {
    content: "";
    position: absolute;
    left: 6px;
    right: 6px;
    height: 2px;
    border-radius: 2px;
    background: var(--accent);
  }
  .block.cat-before::before {
    top: -4px;
  }
  .block.cat-after::after {
    bottom: -4px;
  }
  .block.lifted {
    opacity: 0.45;
  }
  .block:first-child .group {
    padding-top: 0;
  }
  .cat-target {
    background: var(--sel);
  }
  .list-item {
    flex: none;
    text-align: left;
    padding: 6px 10px 6px 25px;
    border-radius: 7px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    position: relative;
  }
  .list-item:hover {
    background: var(--hover);
  }
  .grip {
    position: absolute;
    left: 7px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text3);
    opacity: 0;
    cursor: grab;
  }
  .list-item:hover .grip,
  .list-item:focus-visible .grip {
    opacity: 1;
  }
  .list-item.active {
    background: var(--sidebar-sel);
    font-weight: 600;
  }
  .list-item.drop-into {
    background: var(--sel);
    box-shadow: inset 0 0 0 1.5px var(--accent);
  }
  .list-item.drop-before {
    box-shadow: inset 0 2px 0 var(--accent);
  }
  .list-item.drop-after {
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .new-list {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text3);
    padding: 10px 10px;
    border-radius: 7px;
    margin-top: 6px;
    text-align: left;
  }
  .archived-toggle {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 10px;
    padding: 6px 10px;
    border-radius: 7px;
    color: var(--text3);
    font-size: 12.5px;
    text-align: left;
  }
  .archived-toggle:hover {
    background: var(--hover);
    color: var(--text2);
  }
  .archived-toggle .count {
    margin-left: 2px;
  }
  .group.dim .section-label,
  .group.dim .dot {
    opacity: 0.6;
  }
  .unarchive {
    margin-left: auto;
    font-size: 11.5px;
    color: var(--accent);
    padding: 1px 6px;
    border-radius: 5px;
    opacity: 0;
  }
  .group:hover .unarchive,
  .unarchive:focus-visible {
    opacity: 1;
  }
  .list-item.dim {
    color: var(--text2);
  }
  .empty-cat {
    font-size: 12px;
    padding: 2px 10px 6px 25px;
  }
  .new-list:hover {
    color: var(--text);
    background: var(--hover);
  }
  .bottom {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 8px;
  }
  .attention {
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 6px 10px;
    border-radius: 7px;
    background: var(--amber-bg);
    color: var(--amber);
    font-size: 12.5px;
    text-align: left;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    padding: 0 10px;
  }
  .results {
    flex: 1;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .result {
    flex: none;
    text-align: left;
    padding: 7px 10px;
    border-radius: 7px;
    display: flex;
    flex-direction: column;
  }
  .result:hover {
    background: var(--hover);
  }
  .r-text {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .r-sub {
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .empty {
    padding: 8px 10px;
    margin: 0;
  }
</style>
