<script lang="ts">
  // A board card: shown in a pop-up over the board (CardPeek) or in a window of its own.
  // Everything in it can be edited as on the list page; "Open full page" makes it the main window's view.
  import { app, progress } from "./state.svelte";
  import { dnd } from "./dnd.svelte";
  import { setCaret } from "./caret";
  import Row from "./Row.svelte";
  import Icon from "./Icon.svelte";
  import type { MenuItem, Node } from "./types";

  let {
    id,
    mode,
    grab,
    popOut,
  }: {
    id: string;
    mode: "peek" | "window";
    /** pointerdown on the header (the pop-up is dragged by it) */
    grab?: (e: PointerEvent) => void;
    popOut?: () => void;
  } = $props();

  const f = $derived(app.find(id));
  const node = $derived(f?.node ?? null);
  const isSub = $derived(!!f?.parent);
  const cat = $derived(f ? app.category(f.list.category) : undefined);
  const col = $derived(node ? app.columnOf(node.id) : undefined);
  const p = $derived(node ? progress(node) : { done: 0, total: 0 });

  // where a sub-list lives: "List › Parent item"
  const trail = $derived.by(() => {
    if (!f?.parent) return "";
    const out: string[] = [];
    let q = app.find(f.parent.id);
    while (q) {
      out.unshift(q.node.text || "Untitled");
      q = q.parent ? app.find(q.parent.id) : null;
    }
    return out.join(" › ");
  });

  const rows = $derived.by(() => {
    const out: { node: Node; depth: number }[] = [];
    const add = (nodes: Node[], depth: number) => {
      for (const n of nodes) {
        out.push({ node: n, depth });
        if (!app.collapsed[n.id]) add(n.children, depth + 1);
      }
    };
    if (node) add(node.children, 0);
    return out;
  });

  let titleEl: HTMLDivElement | undefined = $state();

  $effect(() => {
    const t = node?.text ?? "";
    if (titleEl && titleEl.textContent !== t) titleEl.textContent = t;
  });
  $effect(() => {
    const fc = app.focus;
    if (fc && node && fc.id === node.id && titleEl) {
      titleEl.focus();
      setCaret(titleEl, fc.caret === "start" ? 0 : (titleEl.textContent?.length ?? 0));
      app.focus = null;
    }
  });

  function titleKey(e: KeyboardEvent) {
    if (!node) return;
    if (e.key === "Enter") {
      e.preventDefault();
      const first = node.children[0];
      if (first) app.focus = { id: first.id, caret: "end" };
      else newItem();
    } else if (e.key === "Escape") titleEl?.blur();
  }

  function newItem() {
    if (!node) return;
    const it = app.append(node.id);
    if (it) app.focus = { id: it.id, caret: "start" };
  }

  const below = (e: MouseEvent, alignRight = 0) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return { clientX: alignRight ? r.right - alignRight : r.left, clientY: r.bottom + 4 };
  };

  function columnMenu(e: MouseEvent) {
    if (!node) return;
    const id = node.id;
    const items: MenuItem[] = app.doc.columns.map((c) => ({
      label: c.name,
      color: c.color ?? undefined,
      checked: col?.id === c.id,
      run: () => (col ? col.id !== c.id && app.moveCard(id, c.id, c.cards.length, true) : app.putOnBoard(id, c.id)),
    }));
    if (col) items.push({ separator: true, label: "" }, { label: "Take off the board", icon: "close", run: () => app.takeOffBoard(id) });
    app.openMenu(below(e), items);
  }

  function catMenu(e: MouseEvent) {
    if (!f || isSub) return;
    const l = f.list;
    app.openMenu(below(e), [
      ...app.activeCategories.map((c) => ({ label: c.name, color: c.color, checked: l.category === c.id, run: () => app.setCategory(l.id, c.id) })),
      { label: "None", checked: !l.category, run: () => app.setCategory(l.id, null) },
    ]);
  }

  function moreMenu(e: MouseEvent) {
    if (!node) return;
    const n = node;
    app.openMenu(below(e, 220), [
      { label: "Details and notes", icon: "info", run: () => (app.detailsId = n.id) },
      { label: "Rename", icon: "edit", run: () => (app.focus = { id: n.id, caret: "end" }) },
      {
        label: "Open cards as full pages",
        icon: "settings",
        hint: "Change in Settings › Board",
        run: () => {
          app.change(() => (app.doc.settings.cardOpens = "page"));
          app.done("Clicking a card now opens it as a full page");
        },
      },
      { separator: true, label: "" },
      {
        label: isSub ? "Delete" : "Delete list",
        icon: "trash",
        danger: true,
        run: () =>
          (app.confirm = {
            title: `Delete “${n.text || "Untitled"}”?`,
            body: isSub ? "It and everything in it are removed. Undo works." : "Its file moves to the .trash folder inside your data folder. Undo works too.",
            okLabel: "Delete",
            danger: true,
            ok: () => app.deleteNode(n.id),
          }),
      },
    ]);
  }

  const endTarget = $derived(
    (dnd.s?.target?.kind === "end" && dnd.s.target.id === node?.id) || (app.fileDrop?.kind === "end" && app.fileDrop.id === node?.id),
  );
</script>

{#if f && node}
  <div class="card {mode}" data-peek={node.id}>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <header onpointerdown={grab} title={mode === "peek" ? "Drag to move · drag outside the window to open it in its own window" : undefined}>
        {#if isSub}
          <span class="chip static" title={trail}><span class="dot" style:background={cat?.color ?? "var(--text3)"}></span><span class="ell">in {trail}</span></span>
        {:else}
          <button class="chip" onclick={catMenu}>
            <span class="dot" style:background={cat?.color ?? "var(--text3)"}></span>{cat?.name ?? "No category"}<Icon name="chevron-down" size={12} />
          </button>
        {/if}
        <button class="chip" title="Board column" onclick={columnMenu}>
          <Icon name="board" size={13} />{col?.name ?? "Not on the board"}<Icon name="chevron-down" size={12} />
        </button>
        <span class="grow"></span>
        {#if mode === "peek"}
          <button class="icon-btn" title="Open in its own window" onclick={popOut}><Icon name="popout" size={17} /></button>
        {:else}
          <button class="icon-btn" title="Back into the main window" onclick={() => app.cardToMain("dock")}><Icon name="dock" size={17} /></button>
        {/if}
        <button class="full" title="Make this the main window's view" onclick={() => (mode === "peek" ? app.openFull(node.id) : app.cardToMain("full"))}>
          <Icon name="expand" size={15} /> Open full page
        </button>
        <button class="icon-btn" title="More" onclick={moreMenu}><Icon name="more" size={18} /></button>
        {#if mode === "peek"}
          <button class="icon-btn" title="Close (Esc)" onclick={() => app.closePeek()}><Icon name="close" size={17} /></button>
        {/if}
      </header>

      <div class="body">
        <div
          class="title"
          bind:this={titleEl}
          role="textbox"
          aria-label="Title"
          tabindex="0"
          contenteditable="plaintext-only"
          data-placeholder="Untitled"
          spellcheck="true"
          oninput={() => app.setText(node.id, titleEl?.textContent ?? "")}
          onkeydown={titleKey}
        ></div>
        <div class="meta">
          {#if p.total}
            <span>{p.done} of {p.total} done</span>
            <span class="bar"><span style:width="{(100 * p.done) / p.total}%" style:background={cat?.color ?? "var(--accent)"}></span></span>
          {:else}
            <span class="faint">Enter adds an item · Tab nests it · drop files on any item</span>
          {/if}
        </div>

        <div class="rows" role="list">
          {#each rows as r, i (r.node.id)}
            <Row node={r.node} depth={r.depth} prevId={rows[i - 1]?.node.id ?? null} nextId={rows[i + 1]?.node.id ?? null} />
          {/each}
        </div>
        <div class="end" data-list-end={node.id} class:target={endTarget}>
          <button class="add" onclick={newItem}><Icon name="plus" size={15} /> New item</button>
          {#if endTarget && app.fileDrop}<span class="drop-hint">Drop to add an item for each file</span>{/if}
        </div>
      </div>
  </div>
{/if}

<style>
  .card {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  header {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 12px 12px 4px 20px;
    min-width: 0;
  }
  .peek header {
    cursor: grab;
    touch-action: none;
  }
  .peek header :global(button) {
    cursor: pointer;
  }
  .window header {
    padding: 10px 10px 4px 18px;
    border-bottom: 1px solid transparent;
  }
  .grow {
    flex: 1;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 9px;
    border-radius: 7px;
    font-size: 12.5px;
    color: var(--text2);
    background: var(--chip);
    min-width: 0;
    max-width: 280px;
  }
  .chip:not(.static):hover {
    background: var(--chip-hover);
    color: var(--text);
  }
  .chip.static {
    background: none;
    padding-left: 0;
  }
  .ell {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .full {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 30px;
    padding: 0 11px;
    border-radius: 8px;
    font-size: 13px;
    font-weight: 550;
    color: var(--accent);
    white-space: nowrap;
  }
  .full:hover {
    background: var(--sel);
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 28px 8px 48px;
    display: flex;
    flex-direction: column;
  }
  .title {
    font-size: 24px;
    line-height: 1.25;
    font-weight: 700;
    margin: 4px 0 6px;
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
    margin-bottom: 16px;
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
    border-radius: 5px;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .end {
    min-height: 64px;
    padding: 4px 0 16px;
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
</style>
