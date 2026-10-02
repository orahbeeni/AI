<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { api } from '../api';
  import { BUILTIN_PRESETS, blankOptions } from '../defaults';
  import { ago, isUrl } from '../format';
  import { allPresets, app, toast, toastError } from '../state.svelte';
  import type { Subscription } from '../types';

  const EVERY: [number, string][] = [[30, '30 minutes'], [60, 'hour'], [180, '3 hours'], [360, '6 hours'], [720, '12 hours'], [1440, 'day']];

  let editing = $state(false);
  let f = $state({ id: '', name: '', url: '', interval: 360, preset: 'mp4-best', latest: 10, folder: true });
  const presets = $derived(allPresets().filter(p => p.id !== 'live-mp4'));
  const valid = $derived(isUrl(f.url) && f.name.trim().length > 0);

  function startNew() { f = { id: '', name: '', url: '', interval: 360, preset: 'mp4-best', latest: 10, folder: true }; editing = true; }

  async function save() {
    const preset = allPresets().find(p => p.id === f.preset) ?? BUILTIN_PRESETS[2];
    const options = { ...blankOptions(f.url.trim(), app.settings?.downloadDir ?? ''), ...structuredClone($state.snapshot(preset.options)) };
    options.playlistItems = `1-${Math.max(1, f.latest)}`;
    options.useArchive = true;
    if (f.folder) options.outputTemplate = '%(uploader)s/%(title)s [%(id)s].%(ext)s';
    const existing = app.subs.find(s => s.id === f.id);
    const sub: Subscription = {
      id: f.id, name: f.name.trim(), url: f.url.trim(), intervalMinutes: f.interval, enabled: existing?.enabled ?? true,
      options, lastCheck: existing?.lastCheck ?? null, lastNote: existing?.lastNote ?? null,
    };
    try {
      app.subs = await api.saveSub(sub);
      editing = false;
      if (!existing) { toast('ok', 'Subscribed — checking now'); const created = app.subs.find(s => s.name === sub.name && s.url === sub.url); if (created) api.checkSubNow(created.id); }
    } catch (e) { toastError(e); }
  }

  const every = (m: number) => EVERY.find(e => e[0] === m)?.[1] ?? `${m} min`;
  async function toggle(s: Subscription) { app.subs = await api.saveSub({ ...$state.snapshot(s), enabled: !s.enabled } as Subscription); }
  async function remove(s: Subscription) { app.subs = await api.deleteSub(s.id); }
  async function checkNow(s: Subscription) { await api.checkSubNow(s.id); toast('info', `Checking “${s.name}”`, { action: { label: 'View queue', run: () => (app.page = 'queue') } }); }
</script>

<div class="wrap">
  <header class="row">
    <div class="grow"><h1>Subscriptions</h1>
      <p class="muted">Watch a channel or playlist and automatically download new videos. Each one is only downloaded once.</p></div>
    <button class="btn primary" onclick={startNew}><Icon name="plus" size={16} /> New subscription</button>
  </header>

  {#if editing}
    <form class="card pad col" onsubmit={(e) => { e.preventDefault(); save(); }}>
      <h2>{f.id ? 'Edit subscription' : 'New subscription'}</h2>
      <div class="grid">
        <label class="field">Name <input type="text" placeholder="My favourite channel" bind:value={f.name} /></label>
        <label class="field">Channel or playlist link <input type="text" placeholder="https://www.youtube.com/@channel/videos" bind:value={f.url} /></label>
        <label class="field">Check every
          <select bind:value={f.interval}>{#each EVERY as [m, l]}<option value={m}>{l}</option>{/each}</select></label>
        <label class="field">Quality
          <select bind:value={f.preset}>{#each presets as p}<option value={p.id}>{p.name}</option>{/each}</select></label>
        <label class="field">Look at the newest
          <input type="number" min="1" max="200" bind:value={f.latest} /><span class="hint">videos on each check</span></label>
        <label class="check" style="align-self:end"><input type="checkbox" bind:checked={f.folder} />
          <div><b>Put files in a folder named after the channel</b></div></label>
      </div>
      <div class="row"><button class="btn primary" disabled={!valid}>Save</button><button type="button" class="btn ghost" onclick={() => (editing = false)}>Cancel</button></div>
    </form>
  {/if}

  {#if !app.subs.length && !editing}
    <div class="empty card"><Icon name="rss" size={34} /><h2>No subscriptions yet</h2>
      <p class="muted">Add a channel and YT Grab will check it on a schedule while the app is open (or in the tray).</p></div>
  {/if}

  {#each app.subs as s (s.id)}
    <article class="card pad row sub" class:off={!s.enabled}>
      <span class="ico"><Icon name="rss" size={20} /></span>
      <div class="grow col" style="gap:3px">
        <div class="row" style="gap:8px"><b>{s.name}</b>{#if !s.enabled}<span class="pill">Paused</span>{/if}</div>
        <span class="muted small ellipsis mono">{s.url}</span>
        <span class="muted small">Every {every(s.intervalMinutes)} · last checked {ago(s.lastCheck)}{s.lastNote ? ` — ${s.lastNote}` : ''}</span>
      </div>
      <button class="btn sm" onclick={() => checkNow(s)}><Icon name="refresh" size={14} /> Check now</button>
      <button class="btn sm" onclick={() => toggle(s)}>{s.enabled ? 'Pause' : 'Resume'}</button>
      <button class="btn sm ghost danger" aria-label="Delete {s.name}" onclick={() => remove(s)}><Icon name="trash" size={14} /></button>
    </article>
  {/each}
</div>

<style>
  .wrap { max-width: 920px; margin: 0 auto; padding: 26px 28px 40px; display: flex; flex-direction: column; gap: 16px; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  .empty { display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 48px 20px; text-align: center; color: var(--muted); }
  .empty h2 { color: var(--text); }
  .sub.off { opacity: .65; }
  .ico { width: 40px; height: 40px; border-radius: 11px; display: grid; place-items: center; background: var(--accent-soft); color: var(--accent); flex: none; }
  @media (max-width: 760px) { .grid { grid-template-columns: 1fr; } }
</style>
