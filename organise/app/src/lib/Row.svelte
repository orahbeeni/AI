<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { app, isUrl } from "./state.svelte";
  import { dnd, startDrag } from "./dnd.svelte";
  import { caretOffset, caretOnEdge, setCaret } from "./caret";
  import LinkChip from "./LinkChip.svelte";
  import Icon from "./Icon.svelte";
  import type { MenuItem, Node } from "./types";

  let { node, depth, prevId, nextId }: { node: Node; depth: number; prevId: string | null; nextId: string | null } = $props();
  let el: HTMLDivElement | undefined = $state();

  // keep the editable text in step with the document (typing updates the document, undo updates the text)
  $effect(() => {
    const t = node.text;
    if (el && el.textContent !== t) el.textContent = t;
  });

  $effect(() => {
    const f = app.focus;
    if (f && f.id === node.id && el) {
      el.focus();
      setCaret(el, f.caret === "start" ? 0 : f.caret === "end" ? (el.textContent?.length ?? 0) : f.caret);
      el.scrollIntoView({ block: "nearest" });
      app.focus = null;
    }
  });

  const t = $derived(dnd.s?.target?.kind === "row" && dnd.s.target.id === node.id ? dnd.s.target.pos : null);
  const fileTarget = $derived(app.fileDrop?.kind === "row" && app.fileDrop.id === node.id);
  const hiddenKids = $derived(app.collapsed[node.id] ? node.children.length : 0);

  function sync() {
    if (el) app.setText(node.id, el.textContent ?? "");
  }

  function keydown(e: KeyboardEvent) {
    if (e.isComposing || !el) return;
    const ctrl = e.ctrlKey || e.metaKey;
    if (e.key === "Enter" && ctrl) {
      e.preventDefault();
      app.toggleDone(node.id);
    } else if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sync();
      app.split(node.id, caretOffset(el));
    } else if (e.key === "Tab") {
      e.preventDefault();
      sync();
      const c = caretOffset(el);
      if (e.shiftKey) app.outdent(node.id);
      else app.indent(node.id);
      app.focus = { id: node.id, caret: c };
    } else if (e.key === "Backspace" && !ctrl && !e.altKey && !e.shiftKey && window.getSelection()?.isCollapsed && caretOffset(el) === 0) {
      // at the start of the item: join it to the item above (the first item of a list has none, so stays)
      e.preventDefault();
      if (!prevId) return;
      sync();
      app.joinUp(node.id, prevId);
    } else if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      e.preventDefault();
      const c = caretOffset(el);
      app.shift(node.id, e.key === "ArrowUp" ? -1 : 1);
      app.focus = { id: node.id, caret: c };
    } else if (e.key === "ArrowUp" && !e.shiftKey && prevId && caretOnEdge(el, "first")) {
      e.preventDefault();
      app.focus = { id: prevId, caret: "end" };
    } else if (e.key === "ArrowDown" && !e.shiftKey && nextId && caretOnEdge(el, "last")) {
      e.preventDefault();
      app.focus = { id: nextId, caret: "end" };
    } else if (ctrl && e.key === ".") {
      e.preventDefault();
      app.toggleKind(node.id);
    } else if (ctrl && (e.key === "i" || e.key === "I")) {
      e.preventDefault();
      app.detailsId = node.id;
    } else if (e.key === "Escape") {
      el.blur();
    }
  }

  async function paste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData("text/plain") ?? "";
    if (isUrl(text)) {
      e.preventDefault();
      const l = await app.addUrl(text.trim());
      app.attach(node.id, [l.id]);
      app.say("Web link added");
      return;
    }
    if (/\r?\n/.test(text.trim()) && el) {
      e.preventDefault();
      const lines = text
        .split(/\r?\n/)
        .map((s) => s.replace(/^\s*(?:[-*+]\s+)?(?:\[[ xX]\]\s+)?/, "").trim())
        .filter(Boolean);
      const c = caretOffset(el);
      const f = app.find(node.id);
      if (!f || !lines.length) return;
      const items = lines.slice(1).map((s) => app.newItem(s, node.kind));
      app.change(() => {
        f.node.text = f.node.text.slice(0, c) + lines[0] + f.node.text.slice(c);
        f.siblings.splice(f.index + 1, 0, ...items);
      });
      app.say(`Pasted ${lines.length} items`, "info", { label: "Undo", run: () => app.undo() });
    }
  }

  async function addFile(directory: boolean) {
    const picked = await open({ multiple: !directory, directory });
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    if (!paths.length) return;
    try {
      const links = await app.addPaths(paths);
      app.attach(node.id, links.map((l) => l.id));
      app.done(`Linked ${links.length === 1 ? links[0].name : `${links.length} files`}. The file stays where it is.`);
    } catch (e) {
      app.say(String(e), "error");
    }
  }

  function menu(e: MouseEvent) {
    const col = app.columnOf(node.id);
    const items: MenuItem[] = [
      { label: "Link a file…", icon: "file", run: () => addFile(false) },
      { label: "Link a folder…", icon: "folder", run: () => addFile(true) },
      {
        label: "Add a web link…",
        icon: "globe",
        run: () =>
          app.ask("Web link", "", async (v) => {
            if (!v.trim()) return;
            const url = /^[a-z]+:/i.test(v.trim()) ? v.trim() : `https://${v.trim()}`;
            const l = await app.addUrl(url);
            app.attach(node.id, [l.id]);
            app.done(`Added a web link to ${l.name}`);
          }, "https://…", "Add"),
      },
      { separator: true, label: "" },
      { label: node.kind === "task" ? "Make it a bullet" : "Make it a checkbox", icon: node.kind === "task" ? "bullet" : "check", shortcut: "Ctrl .", run: () => app.toggleKind(node.id) },
      { label: "Details", icon: "info", shortcut: "Ctrl I", run: () => (app.detailsId = node.id) },
      {
        label: "More…",
        icon: "more",
        submenu: [
          col
            ? { label: "Take off the board", icon: "board", run: () => app.takeOffBoard(node.id) }
            : { label: "Put on the board as a card", icon: "board", run: () => app.putOnBoard(node.id) },
          { label: "Turn into a list", icon: "list", run: () => app.promote(node.id) },
          {
            label: "Move to list",
            icon: "arrow-right",
            submenu: app.doc.lists.filter((l) => l.id !== app.find(node.id)?.list.id).map((l) => ({ label: l.text || "Untitled", run: () => app.moveToList(node.id, l.id) })),
          },
        ],
      },
      { separator: true, label: "" },
      { label: "Delete item", icon: "trash", danger: true, run: () => app.deleteNode(node.id) },
    ];
    app.openMenu(e, items);
  }
</script>

<div
  class="row"
  data-row={node.id}
  style:--depth={depth}
  class:flash={app.flashId === node.id}
  class:selected={app.detailsId === node.id}
  class:drop-before={t === "before"}
  class:drop-after={t === "after"}
  class:drop-inside={t === "inside" || fileTarget}
  class:done={node.kind === "task" && node.done}
  oncontextmenu={menu}
  role="listitem"
>
  <span class="grip" role="button" tabindex="-1" aria-label="Drag to move" title="Drag to move" onpointerdown={(e) => startDrag(e, "item", node.id, node.text || "Untitled")}>
    <Icon name="grip" size={14} />
  </span>
  {#if node.children.length}
    <button class="caret" tabindex="-1" onclick={() => app.toggleCollapse(node.id)} aria-label={app.collapsed[node.id] ? "Expand" : "Collapse"}>
      <Icon name={app.collapsed[node.id] ? "chevron-right" : "chevron-down"} size={14} />
    </button>
  {:else}
    <span class="caret"></span>
  {/if}
  {#if node.kind === "task"}
    <button class="check" class:on={node.done} tabindex="-1" onclick={() => app.toggleDone(node.id)} aria-label={node.done ? "Mark not done" : "Mark done"}>
      {#if node.done}<Icon name="check" size={12} />{/if}
    </button>
  {:else}
    <span class="bullet"><span></span></span>
  {/if}
  <div class="body">
    <div
      class="text"
      bind:this={el}
      contenteditable="plaintext-only"
      role="textbox"
      tabindex="0"
      spellcheck="true"
      data-placeholder="New item"
      oninput={sync}
      onkeydown={keydown}
      onpaste={paste}
    ></div>
    {#each node.links as lid (lid)}
      <LinkChip id={lid} nodeId={node.id} />
    {/each}
    {#if hiddenKids}
      <button class="hidden-kids" onclick={() => app.toggleCollapse(node.id)}>{hiddenKids} more</button>
    {/if}
  </div>
  <button class="open" tabindex="-1" title="Details (Ctrl+I)" onclick={() => (app.detailsId = node.id)}>
    <Icon name="chevron-right" size={16} />
  </button>
</div>

<style>
  .row {
    position: relative;
    display: flex;
    align-items: flex-start;
    gap: 4px;
    padding: 5px 8px 5px calc(var(--depth) * 28px);
    border-radius: 8px;
    min-height: 36px;
  }
  .row:hover,
  .row.selected {
    background: var(--row-hover);
  }
  .row.flash {
    animation: flash 1.6s ease-out;
  }
  @keyframes flash {
    0%,
    40% {
      background: var(--sel);
    }
  }
  .grip {
    position: absolute;
    left: calc(var(--depth) * 28px - 22px);
    top: 9px;
    width: 18px;
    height: 20px;
    display: grid;
    place-items: center;
    color: var(--text3);
    cursor: grab;
    opacity: 0;
    touch-action: none;
  }
  .row:hover .grip {
    opacity: 1;
  }
  .caret {
    width: 18px;
    height: 26px;
    flex: none;
    display: grid;
    place-items: center;
    color: var(--text3);
  }
  button.caret {
    opacity: 0;
  }
  .row:hover button.caret,
  button.caret:focus-visible {
    opacity: 1;
  }
  .check {
    width: 18px;
    height: 18px;
    margin: 4px 8px 0 0;
    border-radius: 5px;
    border: 1.6px solid var(--line-strong);
    display: grid;
    place-items: center;
    flex: none;
    color: #fff;
    background: var(--bg);
  }
  .check:hover {
    border-color: var(--accent);
  }
  .check.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .bullet {
    width: 18px;
    height: 26px;
    margin-right: 8px;
    display: grid;
    place-items: center;
    flex: none;
  }
  .bullet span {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text2);
  }
  .body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 8px;
    padding-top: 1px;
  }
  .text {
    min-width: 3ch;
    max-width: 100%;
    font-size: 15px;
    line-height: 24px;
    outline: none;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    cursor: text;
  }
  .text:empty::before {
    content: attr(data-placeholder);
    color: var(--text3);
  }
  .done .text {
    color: var(--text3);
    text-decoration: line-through;
    text-decoration-color: color-mix(in srgb, var(--text3) 60%, transparent);
  }
  .hidden-kids {
    font-size: 12px;
    color: var(--text3);
    padding: 0 6px;
    border-radius: 5px;
  }
  .hidden-kids:hover {
    background: var(--hover);
  }
  .open {
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--text3);
    opacity: 0;
    flex: none;
  }
  .row:hover .open {
    opacity: 1;
  }
  .open:hover {
    background: var(--hover);
    color: var(--text);
  }
  .drop-before::before,
  .drop-after::after {
    content: "";
    position: absolute;
    left: calc(var(--depth) * 28px);
    right: 0;
    height: 2px;
    background: var(--accent);
    border-radius: 2px;
  }
  .drop-before::before {
    top: -1px;
  }
  .drop-after::after {
    bottom: -1px;
  }
  .drop-inside {
    background: var(--sel) !important;
    box-shadow: inset 3px 0 0 var(--accent);
  }
</style>
