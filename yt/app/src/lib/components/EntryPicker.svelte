<script lang="ts">
  import Icon from './Icon.svelte';
  import { clock } from '../format';
  import type { EntryRow } from '../types';

  let { entries, picked = $bindable() }: { entries: EntryRow[]; picked: Set<number> } = $props();

  // Selection works like a file manager:
  //   click            select only this row         ctrl/⌘+click   add or remove one row
  //   shift+click      select the range from the    drag           sweep over rows
  //                    last clicked row             checkbox       toggles just that row
  //   ↑ ↓ (+shift)     move / extend                space, ctrl+A  toggle / select all
  let filter = $state('');
  let anchor = $state<number | null>(null); // position in `visible`
  let cursor = $state<number | null>(null);
  let drag = $state<{ base: Set<number> } | null>(null);
  let list: HTMLDivElement | undefined = $state();

  const visible = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    return q ? entries.filter(e => e.title.toLowerCase().includes(q) || String(e.index) === q) : entries;
  });
  const shownPicked = $derived(visible.filter(e => picked.has(e.index)).length);

  $effect(() => { filter; anchor = null; cursor = null; });

  const idsBetween = (a: number, b: number) => visible.slice(Math.min(a, b), Math.max(a, b) + 1).map(e => e.index);
  const withIds = (base: Set<number>, ids: number[], on = true) => {
    const s = new Set(base);
    for (const i of ids) on ? s.add(i) : s.delete(i);
    return s;
  };

  function choose(pos: number, mod: { shift: boolean; ctrl: boolean }, checkbox = false) {
    const id = visible[pos].index;
    if (mod.shift && anchor != null) {
      const ids = idsBetween(anchor, pos);
      if (checkbox) picked = withIds(picked, ids, !picked.has(id));
      else picked = withIds(mod.ctrl ? picked : new Set(), ids);
    } else if (mod.ctrl || checkbox) {
      picked = withIds(picked, [id], !picked.has(id));
      anchor = pos;
    } else {
      picked = new Set([id]);
      anchor = pos;
    }
    cursor = pos;
  }

  function down(ev: PointerEvent, pos: number) {
    if (ev.button !== 0) return;
    list?.focus({ preventScroll: true });
    const mod = { shift: ev.shiftKey, ctrl: ev.ctrlKey || ev.metaKey };
    choose(pos, mod);
    if (!mod.shift) drag = { base: mod.ctrl ? new Set(picked) : new Set() };
    ev.preventDefault(); // no text selection while shift-clicking or dragging
  }

  function enter(pos: number) {
    if (!drag || anchor == null) return;
    picked = withIds(drag.base, idsBetween(anchor, pos));
    cursor = pos;
  }

  function onKey(ev: KeyboardEvent) {
    const n = visible.length;
    if (!n) return;
    const ctrl = ev.ctrlKey || ev.metaKey;
    let to: number | null = null;
    if (ev.key === 'ArrowDown') to = Math.min(n - 1, (cursor ?? -1) + 1);
    else if (ev.key === 'ArrowUp') to = Math.max(0, (cursor ?? n) - 1);
    else if (ev.key === 'Home') to = 0;
    else if (ev.key === 'End') to = n - 1;
    else if (ev.key === ' ' && cursor != null) { choose(cursor, { shift: false, ctrl: true }); ev.preventDefault(); return; }
    else if ((ev.key === 'a' || ev.key === 'A') && ctrl) { picked = withIds(picked, visible.map(e => e.index)); ev.preventDefault(); return; }
    else if (ev.key === 'Escape') { picked = withIds(picked, visible.map(e => e.index), false); return; }
    if (to == null) return;
    ev.preventDefault();
    if (ev.shiftKey) {
      anchor ??= cursor ?? to;
      picked = withIds(ctrl ? picked : new Set(), idsBetween(anchor, to));
    } else if (!ctrl) {
      picked = new Set([visible[to].index]);
      anchor = to;
    }
    cursor = to;
    list?.querySelector(`[data-pos="${to}"]`)?.scrollIntoView({ block: 'nearest' });
  }

  const all = () => (picked = withIds(picked, visible.map(e => e.index)));
  const none = () => (picked = withIds(picked, visible.map(e => e.index), false));
  const invert = () => {
    const s = new Set(picked);
    for (const e of visible) (s.has(e.index) ? s.delete(e.index) : s.add(e.index));
    picked = s;
  };
</script>

<svelte:window onpointerup={() => (drag = null)} />

<section class="card">
  <div class="row head">
    <h3 class="grow">Choose videos ({picked.size} of {entries.length}{filter.trim() ? `, ${shownPicked} of ${visible.length} shown` : ''})</h3>
    <button class="btn sm ghost" onclick={all}>All</button>
    <button class="btn sm ghost" onclick={none}>None</button>
    <button class="btn sm ghost" onclick={invert}>Invert</button>
  </div>
  <div class="row filter">
    <input type="search" placeholder="Filter by title or number…" bind:value={filter} aria-label="Filter videos" />
    <span class="muted small hint">Click · Shift+click range · Ctrl+click add/remove · drag to sweep</span>
  </div>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="entries" role="listbox" aria-multiselectable="true" tabindex="0" bind:this={list} onkeydown={onKey}>
    {#each visible as e, pos (e.index)}
      <div class="entry" role="option" aria-selected={picked.has(e.index)} class:on={picked.has(e.index)} class:cur={cursor === pos}
        data-pos={pos} tabindex="-1" onpointerdown={(ev) => down(ev, pos)} onpointerenter={() => enter(pos)}>
        <!-- A drawn box, not <input type=checkbox>: a native one keeps its own state and gets out of
             step with the selection. Keyboard use goes through the list (arrows, space). -->
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <span class="box" class:checked={picked.has(e.index)} aria-hidden="true"
          onpointerdown={(ev) => ev.stopPropagation()}
          onclick={(ev) => { ev.stopPropagation(); list?.focus({ preventScroll: true }); choose(pos, { shift: ev.shiftKey, ctrl: ev.ctrlKey || ev.metaKey }, true); }}>
          {#if picked.has(e.index)}<Icon name="check" size={13} />{/if}
        </span>
        <span class="muted num">{e.index}</span><span class="grow ellipsis">{e.title}</span>
        <span class="muted small">{e.duration ? clock(e.duration) : ''}</span>
      </div>
    {:else}
      <p class="muted small empty">No videos match “{filter}”.</p>
    {/each}
  </div>
</section>

<style>
  .head { padding: 18px 18px 8px; }
  .filter { padding: 0 18px 10px; gap: 12px; }
  .filter input { flex: 0 1 260px; }
  .hint { margin-left: auto; text-align: right; }
  .entries { max-height: 280px; overflow-y: auto; border-top: 1px solid var(--line); outline: none; user-select: none; }
  .entries:focus-visible { box-shadow: 0 0 0 2px var(--accent) inset; }
  .entry { display: flex; align-items: center; gap: 10px; padding: 7px 18px; cursor: default; }
  .entry:hover { background: var(--panel-2); }
  .entry.on { background: var(--accent-soft); }
  .entry.cur { box-shadow: 0 0 0 1px color-mix(in srgb, var(--accent), transparent 40%) inset; }
  .box { width: 17px; height: 17px; flex: none; border-radius: 5px; border: 1.5px solid color-mix(in srgb, var(--muted), transparent 35%); background: var(--panel); display: grid; place-items: center; color: #fff; cursor: pointer; }
  .box:hover { border-color: var(--accent); }
  .box.checked { background: var(--accent); border-color: var(--accent); }
  .num { width: 28px; text-align: right; font-variant-numeric: tabular-nums; }
  .empty { padding: 14px 18px; margin: 0; }
  @media (max-width: 780px) { .hint { display: none; } }
</style>
