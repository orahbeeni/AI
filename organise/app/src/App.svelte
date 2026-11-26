<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { app } from "./lib/state.svelte";
  import { fileTargetAt, handleDrop } from "./lib/drop";
  import { draggedItem, relayedDrag, edgeScroll, systemDragLeave, systemDragOver, systemDrop } from "./lib/dnd.svelte";
  import { cardSelftest, selftest } from "./lib/selftest";
  import type { Doc, Link, MoveRecord, View } from "./lib/types";
  import Sidebar from "./lib/Sidebar.svelte";
  import ListView from "./lib/ListView.svelte";
  import BoardView from "./lib/BoardView.svelte";
  import LinksView from "./lib/LinksView.svelte";
  import SettingsView from "./lib/SettingsView.svelte";
  import DetailsPanel from "./lib/DetailsPanel.svelte";
  import Overlays from "./lib/Overlays.svelte";
  import CardView from "./lib/CardView.svelte";

  let sidebar: Sidebar | undefined = $state();

  // A window opened for one card (see open_card_window) is told which card before this runs.
  const myCard = (window as unknown as { __ORGANISE_CARD__?: string }).__ORGANISE_CARD__;
  if (myCard) app.cardWindow = myCard;

  // colour mode: Settings › Appearance. Remembered here too so the window opens in it before the data loads.
  function applyTheme(t: string) {
    if (t === "light" || t === "dark") document.documentElement.dataset.theme = t;
    else delete document.documentElement.dataset.theme;
  }
  try {
    applyTheme(localStorage.getItem("organise.theme") ?? "system");
  } catch {
    /* first run */
  }
  $effect(() => {
    if (!app.loaded) return;
    const t = app.doc.settings.theme ?? "system";
    applyTheme(t);
    getCurrentWindow().setTheme(t === "system" ? null : t).catch(() => {}); // the title bar too
    try {
      localStorage.setItem("organise.theme", t);
    } catch {
      /* fine */
    }
  });

  // a card window follows its card: its title, and it closes if the card is deleted
  $effect(() => {
    if (!app.cardWindow || !app.loaded) return;
    const f = app.find(app.cardWindow);
    if (!f) getCurrentWindow().close();
    else getCurrentWindow().setTitle(f.node.text || "Untitled").catch(() => {});
  });

  onMount(() => {
    const unlisten: (() => void)[] = [];
    let dragPaths: string[] = [];
    (async () => {
      const r = await app.load();
      if (app.cardWindow) {
        // nothing to set up: it shows its card
      } else if (r.startView?.startsWith("details:")) {
        app.view = "list";
        app.openList(r.startView.slice(8));
        app.detailsId = r.startView.slice(8);
        app.focus = null;
      } else if (r.startView) app.view = r.startView as View;

      unlisten.push(
        await listen<{ links: Link[]; moves: MoveRecord[] }>("links-updated", (e) => {
          app.updateLinks(e.payload.links);
          const moves = e.payload.moves;
          if (moves.length) {
            app.moves = [...moves.slice().reverse(), ...app.moves].slice(0, 50);
            app.say(moves.length === 1 ? `${moves[0].name} moved. Its links now point to the new place.` : `${moves.length} files moved. Their links were updated.`);
          }
        }),
      );
      unlisten.push(await listen<boolean>("search-state", (e) => (app.searching = e.payload)));

      // another window (a card in its own window, or the main window) saved the document
      const me = getCurrentWindow().label;
      unlisten.push(await listen<{ doc: Doc; from: string }>("doc-changed", (e) => e.payload.from !== me && app.remoteDoc(e.payload.doc)));
      // a card window asks the main window to show its card
      if (!app.cardWindow) {
        unlisten.push(await listen<{ id: string; phase: string; x: number; y: number }>("item-drag", (e) => relayedDrag(e.payload.id, e.payload.phase, e.payload.x, e.payload.y)));
        unlisten.push(
          await listen<{ action: string; id: string }>("card-action", (e) => {
            if (e.payload.action === "full") app.openFull(e.payload.id);
            else if (app.find(e.payload.id)) {
              app.view = "board";
              app.peekId = e.payload.id;
            }
          }),
        );
      }
      // save before you switch to another window, so it shows what you just did
      const blur = () => app.flushIfDirty();
      window.addEventListener("blur", blur);
      unlisten.push(() => window.removeEventListener("blur", blur));

      // files dragged in from the file manager
      unlisten.push(
        await getCurrentWebview().onDragDropEvent((ev) => {
          const p = ev.payload;
          const scale = window.devicePixelRatio || 1;
          // an item dragged out of a card window (see startSystemDrag), not files
          const item = draggedItem(p.type === "enter" || p.type === "drop" ? p.paths : dragPaths);
          if (item) {
            if (p.type === "enter") dragPaths = p.paths;
            if (p.type === "enter" || p.type === "over") {
              edgeScroll(p.position.x / scale, p.position.y / scale);
              systemDragOver(item, p.position.x / scale, p.position.y / scale);
            } else if (p.type === "drop") {
              dragPaths = [];
              systemDrop(item, p.position.x / scale, p.position.y / scale);
            } else {
              dragPaths = [];
              systemDragLeave();
            }
            return;
          }
          if (p.type === "enter") {
            dragPaths = p.paths;
            app.fileDrop = fileTargetAt(p.position.x / scale, p.position.y / scale);
          } else if (p.type === "over") {
            edgeScroll(p.position.x / scale, p.position.y / scale);
            app.fileDrop = fileTargetAt(p.position.x / scale, p.position.y / scale);
          } else if (p.type === "drop") {
            const t = fileTargetAt(p.position.x / scale, p.position.y / scale);
            app.fileDrop = null;
            handleDrop(p.paths.length ? p.paths : dragPaths, t);
          } else {
            app.fileDrop = null;
          }
        }),
      );

      // Save anything pending before the window closes (at most 1.5 s, so closing never hangs).
      // Tauri then destroys the window, which needs core:window:allow-destroy in capabilities/default.json.
      unlisten.push(
        await getCurrentWindow().onCloseRequested(async () => {
          await Promise.race([app.flush(), new Promise((r) => setTimeout(r, 1500))]);
        }),
      );

      if (r.selftestFile && app.cardWindow) setTimeout(() => cardSelftest(app.cardWindow!), 300);
      else if (r.selftestFile) setTimeout(() => selftest(r.selftestFile!), 300);
    })();
    return () => unlisten.forEach((u) => u());
  });

  // The browser's own right-click menu (Back, Reload…) makes no sense in an app. Text fields keep theirs
  // for Cut/Copy/Paste and spelling. Our own menus stop the event before it gets here.
  function nativeMenu(e: MouseEvent) {
    const t = e.target as HTMLElement | null;
    if (t?.closest("input, textarea, select, [contenteditable]")) return;
    e.preventDefault();
  }

  function keydown(e: KeyboardEvent) {
    const ctrl = e.ctrlKey || e.metaKey;
    const tag = (e.target as HTMLElement | null)?.tagName;
    const inField = tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
    const k = e.key.toLowerCase();
    if (app.cardWindow && ctrl && (k === "k" || k === "1" || k === "2" || k === "n")) {
      e.preventDefault(); // a card window has no sidebar, board or lists to switch to
    } else if (ctrl && k === "k") {
      e.preventDefault();
      sidebar?.focusSearch();
    } else if (ctrl && !inField && k === "z") {
      e.preventDefault();
      if (e.shiftKey) app.redo();
      else app.undo();
    } else if (ctrl && !inField && k === "y") {
      e.preventDefault();
      app.redo();
    } else if (ctrl && e.shiftKey && k === "n") {
      e.preventDefault();
      app.addList();
    } else if (ctrl && k === "1") {
      e.preventDefault();
      app.view = "list";
    } else if (ctrl && k === "2") {
      e.preventDefault();
      app.view = "board";
    } else if (e.key === "Escape" && !app.menu && !app.prompt && !app.confirm && app.detailsId) {
      app.detailsId = null;
    } else if (e.key === "Escape" && !app.menu && !app.prompt && !app.confirm && app.peekId) {
      // Esc in a text field just leaves the field; the next Esc closes the card pop-up
      if (!(e.target as HTMLElement | null)?.closest?.("input, textarea, [contenteditable]")) app.closePeek();
    }
  }
</script>

<svelte:window onkeydown={keydown} oncontextmenu={nativeMenu} />

{#if app.loadError}
  <div class="fatal">
    <h2>Organise couldn't open its data</h2>
    <p>{app.loadError}</p>
  </div>
{:else if app.loaded && app.cardWindow}
  <div class="app">
    <main class="card-main">
      <CardView id={app.cardWindow} mode="window" />
    </main>
    {#if app.detailsId}
      <DetailsPanel />
    {/if}
  </div>
  <Overlays />
{:else if app.loaded}
  <div class="app">
    <Sidebar bind:this={sidebar} />
    <main>
      {#if app.view === "board"}
        <BoardView />
      {:else if app.view === "links"}
        <LinksView />
      {:else if app.view === "settings"}
        <SettingsView />
      {:else}
        <ListView />
      {/if}
    </main>
    {#if app.detailsId && (app.view === "list" || (app.view === "board" && app.peekId))}
      <DetailsPanel />
    {/if}
  </div>
  <Overlays />
{/if}

<style>
  .app {
    display: flex;
    height: 100vh;
  }
  main {
    flex: 1;
    min-width: 0;
    position: relative;
    container: main / inline-size;
  }
  :global(body.dragging),
  :global(body.dragging *) {
    cursor: grabbing !important;
    user-select: none !important;
  }
  .card-main {
    background: var(--bg);
  }
  .fatal {
    padding: 40px;
  }
</style>
