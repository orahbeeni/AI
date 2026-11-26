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

  // ---- the "Not on the board" tray under the columns; its height is dragged by the bar above it ----
  const tray = $derived(app.notOnBoard(app.boardFilter));
  const trayTarget = $derived(dnd.s?.target?.kind === "tray");
  /** a card from the board is being dragged: show the "Take off the board" drop icon */
  const offboard = $derived(dnd.s?.kind === "card" && !!app.columnOf(dnd.s.id));
  const overIcon = $derived(trayTarget && !!dnd.s && document.elementFromPoint(dnd.s.x, dnd.s.y)?.closest("[data-offboard]"));
  const TRAY_KEY = "organise.trayHeight";
  const TRAY_DEFAULT = 170;
  const TRAY_MIN = 46; // just its heading
  function savedTray(): number {
    try {
      const h = Number(localStorage.getItem(TRAY_KEY));
      return h >= TRAY_MIN ? h : TRAY_DEFAULT;
    } catch {
      return TRAY_DEFAULT;
    }
  }
  let trayH = $state(savedTray());
  let boardEl: HTMLDivElement | undefined = $state();
  const trayMax = () => Math.max(TRAY_MIN, (boardEl?.clientHeight ?? 600) - 220);
  function setTray(h: number) {
    trayH = Math.round(Math.min(trayMax(), Math.max(TRAY_MIN, h)));
    try {
      localStorage.setItem(TRAY_KEY, String(trayH));
    } catch {
      /* not remembered */
    }
  }
  function resizeTray(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const y0 = e.clientY;
    const h0 = trayH;
    document.body.classList.add("resizing-rows");
    const move = (ev: PointerEvent) => setTray(h0 + (y0 - ev.clientY));
    const up = () => {
      document.body.classList.remove("resizing-rows");
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
  function trayKey(e: KeyboardEvent) {
    if (e.key === "ArrowUp") setTray(trayH + 20);
    else if (e.key === "ArrowDown") setTray(trayH - 20);
    else return;
    e.preventDefault();
  }

  const colTarget = (c: Column) => dnd.s && dnd.s.target?.kind === "column" && dnd.s.target.id === c.id ? dnd.s.target.index : -1;
</script>

<div class="board" bind:this={boardEl}>
  {#if offboard}
    <div class="offboard" class:hot={overIcon} data-offboard>
      <Icon name="board-off" size={18} /> Take off the board
    </div>
  {/if}
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

  <button
    class="splitter"
    title="Drag to resize · double-click to reset"
    aria-label="Resize the Not on the board area"
    onpointerdown={resizeTray}
    ondblclick={() => setTray(TRAY_DEFAULT)}
    onkeydown={trayKey}
  ></button>
  <section class="tray" data-tray class:target={trayTarget} style:height="{trayH}px">
    <div class="tray-head">
      <span class="tray-title">Not on the board</span>
      <span class="faint">{tray.length}</span>
      <span class="faint hint">{trayTarget ? "Drop to take it off the board" : "Drag onto a column to add it · click to open"}</span>
    </div>
    <div class="tray-items">
      {#each tray as t (t.node.id)}
        {@const cat = app.category(t.list.category)}
        {@const p = progress(t.node)}
        <div
          class="tray-card"
          data-tray-card={t.node.id}
          class:dragging={dnd.s?.kind === "card" && dnd.s.id === t.node.id}
          role="button"
          tabindex="0"
          title={t.sub ? `In ${t.list.text || "Untitled"}` : undefined}
          onpointerdown={(e) => startDrag(e, "card", t.node.id, t.node.text || "Untitled", () => app.openCard(t.node.id, "popup"))}
          onkeydown={(e) => e.key === "Enter" && app.openCard(t.node.id, "popup")}
          oncontextmenu={(e) =>
            app.openMenu(e, [
              { label: "Open", icon: "external", run: () => app.openCard(t.node.id, "popup") },
              { label: "Put on the board", icon: "board", submenu: app.doc.columns.map((c) => ({ label: c.name, run: () => app.putOnBoard(t.node.id, c.id) })) },
            ])}
        >
          <span class="dot" style:background={cat?.color ?? "var(--text3)"}></span>
          <span class="tc-text">
            <span class="tc-title">{t.node.text || "Untitled"}</span>
            {#if t.sub}<span class="tc-sub faint">in {t.list.text || "Untitled"}</span>{/if}
          </span>
          {#if p.total}<span class="faint tc-n">{p.done}/{p.total}</span>{/if}
        </div>
      {:else}
        <div class="faint tray-empty">Everything is on the board</div>
      {/each}
    </div>
  </section>
</div>

{#if app.peekId}<CardPeek />{/if}

<style>
  .board {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-width: 0;
    position: relative;
  }
  .offboard {
    position: absolute;
    top: 18px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 15;
    display: flex;
    align-items: center;
    gap: 9px;
    height: 44px;
    padding: 0 20px;
    border-radius: 22px;
    background: var(--bg);
    border: 1.5px dashed var(--line-strong);
    color: var(--text2);
    font-size: 13.5px;
    font-weight: 600;
    box-shadow: var(--shadow);
    animation: drop-in 0.15s ease-out;
  }
  .offboard.hot {
    border-style: solid;
    border-color: var(--red);
    color: var(--red);
    background: color-mix(in srgb, var(--red) 12%, var(--bg));
    transform: translateX(-50%) scale(1.06);
  }
  @keyframes drop-in {
    from {
      opacity: 0;
      transform: translate(-50%, -10px);
    }
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
    padding: 0 28px 10px;
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
  .splitter {
    flex: none;
    height: 9px;
    margin: 0 28px;
    cursor: row-resize;
    position: relative;
  }
  .splitter::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    top: 4px;
    height: 1px;
    background: var(--line);
    transition: background 0.15s;
  }
  .splitter:hover::after,
  .splitter:focus-visible::after,
  :global(body.resizing-rows) .splitter::after {
    height: 2px;
    background: var(--accent);
  }
  :global(body.resizing-rows),
  :global(body.resizing-rows *) {
    cursor: row-resize !important;
    user-select: none !important;
  }
  .tray {
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    margin: 0 28px 14px;
    border-radius: 12px;
  }
  .tray.target {
    background: var(--sel);
    box-shadow: inset 0 0 0 1.5px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .tray-head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 4px 8px;
    font-size: 13.5px;
  }
  .tray-title {
    font-weight: 650;
  }
  .hint {
    margin-left: auto;
    font-size: 12px;
  }
  .tray-items {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-wrap: wrap;
    align-content: flex-start;
    gap: 8px;
    padding: 2px 2px 8px;
  }
  .tray-card {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 260px;
    padding: 7px 11px;
    border-radius: 9px;
    background: var(--card);
    border: 1px solid var(--line);
    cursor: pointer;
    user-select: none;
    touch-action: none;
    font-size: 13px;
  }
  .tray-card:hover {
    border-color: var(--line-strong);
  }
  .tray-card.dragging {
    opacity: 0.4;
  }
  .tc-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .tc-title,
  .tc-sub {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tc-title {
    font-weight: 600;
  }
  .tc-sub {
    font-size: 11.5px;
  }
  .tc-n {
    font-size: 11.5px;
    margin-left: 2px;
  }
  .tray-empty {
    font-size: 12.5px;
    padding: 6px 4px;
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
