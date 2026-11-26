<script lang="ts">
  // The card pop-up over the board. Drag it by its header anywhere in the main area; drag it past the edge
  // of the app window (or press the pop-out button) and it opens in a window of its own.
  import { onMount } from "svelte";
  import { app } from "./state.svelte";
  import { dnd } from "./dnd.svelte";
  import CardView from "./CardView.svelte";

  const f = $derived(app.peekId ? app.find(app.peekId) : null);
  // while an item is dragged out of the pop-up the board behind it takes drops (it becomes a card there)
  const passThrough = $derived(dnd.s?.kind === "item");

  // the card went away (deleted, undone): nothing left to show
  $effect(() => {
    if (app.peekId && !f) app.peekId = null;
  });

  // where it's been dragged to, from its usual place (top centre); remembered on this computer
  const KEY = "organise.peekOffset";
  function saved(): { x: number; y: number } {
    try {
      const o = JSON.parse(localStorage.getItem(KEY) || "null");
      if (o && Number.isFinite(o.x) && Number.isFinite(o.y)) return o;
    } catch {
      /* not remembered */
    }
    return { x: 0, y: 0 };
  }
  let off = $state(saved());
  let dragging = $state(false);
  let outside = $state(false);
  let scrim: HTMLDivElement | undefined = $state();
  let panel: HTMLDivElement | undefined = $state();

  function remember() {
    try {
      localStorage.setItem(KEY, JSON.stringify(off));
    } catch {
      /* fine */
    }
  }

  /** Keeps the pop-up inside the main area: all of it across, and down until only its top part shows. */
  function clamp(x: number, y: number) {
    if (!scrim || !panel) return { x, y };
    const s = scrim.getBoundingClientRect();
    const p = panel.getBoundingClientRect();
    const left0 = p.left - off.x;
    const top0 = p.top - off.y;
    const minX = s.left + 8 - left0;
    const maxX = s.right - 8 - p.width - left0;
    x = minX <= maxX ? Math.min(maxX, Math.max(minX, x)) : (minX + maxX) / 2;
    y = Math.min(s.bottom - 140 - top0, Math.max(s.top + 8 - top0, y));
    return { x: Math.round(x), y: Math.round(y) };
  }

  onMount(() => {
    panel?.focus();
    off = clamp(off.x, off.y); // the window may be smaller than last time
    const fit = () => (off = clamp(off.x, off.y));
    window.addEventListener("resize", fit);
    return () => window.removeEventListener("resize", fit);
  });

  function grab(e: PointerEvent) {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, [contenteditable], input")) return;
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    try {
      handle.setPointerCapture(e.pointerId); // keeps the drag going outside the window
    } catch {
      /* still draggable inside the window */
    }
    const sx = e.clientX;
    const sy = e.clientY;
    const ox = off.x;
    const oy = off.y;
    const r = panel!.getBoundingClientRect();
    const grabX = e.clientX - r.left;
    const grabY = e.clientY - r.top;
    let moved = false;
    const move = (ev: PointerEvent) => {
      if (!moved && Math.hypot(ev.clientX - sx, ev.clientY - sy) < 3) return;
      moved = true;
      dragging = true;
      // the pointer has left the app window: letting go here opens the card in its own window
      outside = ev.clientX < 0 || ev.clientY < 0 || ev.clientX >= window.innerWidth || ev.clientY >= window.innerHeight;
      off = clamp(ox + ev.clientX - sx, oy + ev.clientY - sy);
    };
    const up = (ev: PointerEvent) => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      handle.removeEventListener("pointercancel", up);
      const out = outside && ev.type === "pointerup";
      dragging = false;
      outside = false;
      if (out) {
        off = { x: ox, y: oy }; // next time it pops up where it was
        popOut({ x: ev.screenX - grabX, y: ev.screenY - grabY });
      } else if (moved) remember();
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
    handle.addEventListener("pointercancel", up);
  }

  /** Opens the card in its own window, the size of the pop-up, where it was (or where it was dropped). */
  function popOut(at?: { x: number; y: number }) {
    if (!f || !panel) return;
    const r = panel.getBoundingClientRect();
    app.popOut(f.node.id, { w: r.width, h: r.height }, at ?? { x: window.screenX + r.left, y: window.screenY + r.top });
  }

  function recentre(e: MouseEvent) {
    if ((e.target as HTMLElement).closest("button, [contenteditable], input")) return;
    off = { x: 0, y: 0 };
    remember();
  }
</script>

{#if f}
  <div class="scrim" class:pass={passThrough} bind:this={scrim} role="presentation" onpointerdown={(e) => e.target === e.currentTarget && app.closePeek()}>
    <div
      class="panel"
      class:dragging
      class:outside
      bind:this={panel}
      role="dialog"
      aria-modal="true"
      aria-label={f.node.text || "Untitled"}
      tabindex="-1"
      style:transform="translate({off.x}px, {off.y}px)"
      ondblclick={(e) => (e.target as HTMLElement).closest("header") && recentre(e)}
    >
      <CardView id={f.node.id} mode="peek" {grab} popOut={() => popOut()} />
      {#if outside}
        <div class="out-hint"><span>Let go to open it in its own window</span></div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 20;
    background: rgba(8, 12, 22, 0.36);
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding: 5vh 32px 32px;
    overflow: hidden;
    animation: fade 0.12s ease-out;
  }
  .scrim.pass {
    pointer-events: none;
    background: rgba(8, 12, 22, 0.12);
    transition: background 0.15s;
  }
  .scrim.pass .panel {
    pointer-events: auto;
  }
  .panel {
    position: relative;
    width: min(760px, 100%);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 14px;
    box-shadow: var(--shadow), 0 24px 60px rgba(15, 23, 42, 0.18);
    outline: none;
    animation: rise 0.16s ease-out;
  }
  .panel.dragging {
    box-shadow: var(--shadow), 0 30px 80px rgba(15, 23, 42, 0.28);
    user-select: none;
  }
  .panel.dragging :global(header) {
    cursor: grabbing;
  }
  .out-hint {
    position: absolute;
    inset: 0;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 120px;
    border-radius: 14px;
    border: 2px dashed var(--accent);
    background: color-mix(in srgb, var(--bg) 55%, transparent);
    pointer-events: none;
  }
  .out-hint span {
    padding: 9px 16px;
    border-radius: 9px;
    background: var(--accent);
    color: #fff;
    font-weight: 600;
    font-size: 13.5px;
    box-shadow: var(--shadow);
  }
  @keyframes fade {
    from {
      background: transparent;
    }
  }
  @keyframes rise {
    from {
      opacity: 0;
    }
  }
</style>
