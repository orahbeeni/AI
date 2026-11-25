<script lang="ts">
  import { onDestroy } from "svelte";
  import { app, linkCount, progress, PALETTE } from "./state.svelte";
  import { dnd, startDrag } from "./dnd.svelte";
  import Icon from "./Icon.svelte";
  import CardPeek from "./CardPeek.svelte";

  // leaving the board (sidebar, Ctrl+1…) closes a card pop-up
  onDestroy(() => (app.peekId = null));
  import type { Column, MenuItem, Node } from "./types";

  const notOnBoard = $derived(app.doc.lists.filter((l) => !app.columnOf(l.id) && !app.isArchived(l.id)));

  function cardInfo(id: string) {
    const f = app.find(id);
    if (!f) return null;
    const cat = app.category(f.list.category);
    return { node: f.node, list: f.list, cat, p: progress(f.node), links: linkCount(f.node), sub: f.parent ? f.list.text : null };
  }

  function visible(id: string) {
    if (app.isArchived(id)) return false; // archived categories stay off the board until unarchived
    if (!app.boardFilter) return true;
    return app.categoryOfNode(id)?.id === app.boardFilter;
  }

  function columnMenu(e: MouseEvent, c: Column) {
    const i = app.doc.columns.indexOf(c);
    const n = c.cards.length;
    const hidden = c.cards.filter((id) => app.isArchived(id)).length;
    const items: MenuItem[] = [
      { label: "Rename", icon: "edit", run: () => app.ask("Rename column", c.name, (v) => app.renameColumn(c.id, v), "", "Rename") },
      { label: "Move left", icon: "arrow-left", disabled: i === 0, run: () => app.moveColumn(c.id, -1) },
      { label: "Move right", icon: "arrow-right", disabled: i === app.doc.columns.length - 1, run: () => app.moveColumn(c.id, 1) },
      {
        label: "More…",
        icon: "more",
        submenu: [
          { label: "No colour", checked: !c.color, run: () => app.setColumnColor(c.id, null) },
          ...PALETTE.map((p) => ({ label: "Colour", color: p, checked: c.color === p, run: () => app.setColumnColor(c.id, p) })),
        ],
      },
      { separator: true, label: "" },
      {
        label: "Delete column",
        icon: "trash",
        danger: true,
        disabled: n > 0,
        hint: n > 0 ? `Move its ${n} ${n === 1 ? "card" : "cards"} out first${hidden ? ` (${hidden} in archived categories)` : ""}` : undefined,
        run: () => app.deleteColumn(c.id),
      },
    ];
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    app.openMenu({ clientX: r.left, clientY: r.bottom + 4 }, items);
  }

  function addListMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuItem[] = notOnBoard.length
      ? notOnBoard.map((l: Node) => ({ label: l.text || "Untitled", color: app.category(l.category)?.color, run: () => app.putOnBoard(l.id) }))
      : [{ label: "Every list is on the board", disabled: true }];
    app.openMenu({ clientX: r.right - 240, clientY: r.bottom + 4 }, items);
  }

  function cardMenu(e: MouseEvent, id: string) {
    const popup = (app.doc.settings.cardOpens ?? "popup") === "popup";
    app.openMenu(e, [
      { label: "Open", icon: "external", run: () => app.openCard(id) },
      popup
        ? { label: "Open full page", icon: "expand", run: () => app.openCard(id, "page") }
        : { label: "Open in a pop-up", icon: "expand", run: () => app.openCard(id, "popup") },
      { label: "Open in its own window", icon: "popout", run: () => app.popOut(id) },
      {
        label: "Move to",
        icon: "arrow-right",
        submenu: app.doc.columns.map((c) => ({ label: c.name, checked: c.cards.includes(id), run: () => app.moveCard(id, c.id, c.cards.length, true) })),
      },
      { separator: true, label: "" },
      { label: "Take off the board", icon: "close", run: () => app.takeOffBoard(id) },
    ]);
  }

  const colTarget = (c: Column) => dnd.s && dnd.s.target?.kind === "column" && dnd.s.target.id === c.id ? dnd.s.target.index : -1;
</script>

<div class="board">
  <header>
    <h1>Board</h1>
    <div class="filters">
      <button class:on={!app.boardFilter} onclick={() => (app.boardFilter = null)}>All</button>
      {#each app.activeCategories as c (c.id)}
        <button class:on={app.boardFilter === c.id} onclick={() => (app.boardFilter = c.id)}>{c.name}</button>
      {/each}
    </div>
    <div class="actions">
      <button class="btn" onclick={addListMenu}><Icon name="plus" size={15} /> Add list</button>
    </div>
  </header>

  <div class="columns">
    {#each app.doc.columns as c (c.id)}
      {@const ti = colTarget(c)}
      <section class="column" data-col={c.id} class:target={ti >= 0}>
        <div class="col-head">
          <span class="dot" style:background={c.color ?? "var(--text3)"}></span>
          <span class="col-name">{c.name}</span>
          <span class="faint">{c.cards.filter(visible).length}</span>
          <button class="icon-btn col-menu" title="Column options" onclick={(e) => columnMenu(e, c)}><Icon name="more" size={16} /></button>
        </div>
        <div class="cards">
          {#each c.cards as id, i (id)}
            {@const ci = cardInfo(id)}
            {#if ti === i}<div class="slot"></div>{/if}
            {#if ci && visible(id)}
              <div
                class="card"
                data-card={id}
                class:dragging={dnd.s?.kind === "card" && dnd.s.id === id}
                role="button"
                tabindex="0"
                onpointerdown={(e) => startDrag(e, "card", id, ci.node.text || "Untitled", () => app.openCard(id))}
                onkeydown={(e) => e.key === "Enter" && app.openCard(id)}
                oncontextmenu={(e) => cardMenu(e, id)}
              >
                <div class="card-title">{ci.node.text || "Untitled"}</div>
                {#if ci.sub}<div class="card-sub faint">in {ci.sub}</div>{/if}
                <div class="card-meta">
                  <span class="dot" style:background={ci.cat?.color ?? "var(--text3)"}></span>
                  <span>{ci.cat?.name ?? "No category"}</span>
                  {#if ci.links}<span class="links"><Icon name="link" size={13} />{ci.links}</span>{/if}
                </div>
                {#if ci.p.total}
                  <div class="progress">
                    <span class="bar"><span style:width="{(100 * ci.p.done) / ci.p.total}%" style:background={ci.cat?.color ?? "var(--accent)"}></span></span>
                    <span class="faint">{ci.p.done}/{ci.p.total}</span>
                  </div>
                {/if}
              </div>
            {/if}
          {/each}
          {#if ti === c.cards.length}<div class="slot"></div>{/if}
          {#if !c.cards.length && ti < 0}<div class="empty-col faint">Drag lists here</div>{/if}
          {#if c.cards.some((id) => app.isArchived(id))}
            {@const h = c.cards.filter((id) => app.isArchived(id)).length}
            <div class="hidden-note faint">{h} {h === 1 ? "card" : "cards"} in archived categories</div>
          {/if}
        </div>
      </section>
    {/each}
    <button class="add-col" title="Add a column" onclick={() => app.ask("New column", "", (v) => app.addColumn(v), "e.g. Waiting", "Add")}>
      <Icon name="plus" size={18} />
    </button>
  </div>
</div>

{#if app.peekId}<CardPeek />{/if}

<style>
  .board {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 20px;
    padding: 24px 28px 16px;
  }
  h1 {
    font-size: 28px;
    margin: 0;
    letter-spacing: -0.01em;
  }
  .filters {
    display: flex;
    gap: 4px;
  }
  .filters button {
    padding: 5px 11px;
    border-radius: 7px;
    color: var(--text2);
    font-size: 13px;
  }
  .filters button:hover {
    background: var(--hover);
  }
  .filters button.on {
    background: var(--sidebar-sel);
    color: var(--text);
    font-weight: 600;
  }
  .actions {
    margin-left: auto;
  }
  .columns {
    flex: 1;
    display: flex;
    gap: 14px;
    padding: 0 28px 24px;
    overflow-x: auto;
    min-height: 0;
    align-items: stretch;
  }
  .column {
    flex: 1 1 0;
    min-width: 168px;
    max-width: 320px;
    background: var(--column);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .column.target {
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .col-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 10px 8px 14px;
    font-size: 13.5px;
  }
  .col-name {
    font-weight: 650;
  }
  .col-menu {
    margin-left: auto;
    opacity: 0;
    width: 26px;
    height: 26px;
  }
  .column:hover .col-menu,
  .col-menu:focus-visible {
    opacity: 1;
  }
  .cards {
    flex: 1;
    overflow-y: auto;
    padding: 2px 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .card {
    flex: none;
    background: var(--card);
    border: 1px solid var(--line);
    border-radius: 10px;
    padding: 11px 13px;
    cursor: pointer;
    user-select: none;
    touch-action: none;
  }
  .card:hover {
    border-color: var(--line-strong);
    box-shadow: 0 2px 8px rgba(15, 23, 42, 0.06);
  }
  .card.dragging {
    opacity: 0.4;
  }
  .card-title {
    font-weight: 600;
    font-size: 14px;
  }
  .card-sub {
    font-size: 12px;
  }
  .card-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text2);
    margin-top: 6px;
  }
  .links {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--text3);
  }
  .progress {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 9px;
    font-size: 11.5px;
  }
  .bar {
    flex: 1;
    height: 4px;
    border-radius: 4px;
    background: var(--chip);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    border-radius: 4px;
  }
  .slot {
    height: 52px;
    border-radius: 10px;
    background: var(--sel);
    border: 1.5px dashed color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .hidden-note {
    font-size: 11.5px;
    text-align: center;
    padding: 4px;
  }
  .empty-col {
    font-size: 12.5px;
    padding: 16px 4px;
    text-align: center;
  }
  .add-col {
    width: 40px;
    height: 40px;
    flex: none;
    border-radius: 10px;
    background: var(--column);
    color: var(--text3);
    display: grid;
    place-items: center;
  }
  .add-col:hover {
    color: var(--text);
    background: var(--hover);
  }
</style>
