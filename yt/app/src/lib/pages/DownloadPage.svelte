<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import OptionsPanel from '../components/OptionsPanel.svelte';
  import SetupBanner from '../components/SetupBanner.svelte';
  import { api, copyText, pickFolder, readClipboard, toApiError } from '../api';
  import { BUILTIN_PRESETS, NETWORK_KEYS, blankOptions } from '../defaults';
  import { clock, extractUrls, isUrl, normalizeProbe, toRanges, until } from '../format';
  import { allPresets, app, saveSettings, toast, toastError } from '../state.svelte';
  import type { ApiError, JobOptions, Preset, Probe } from '../types';

  let input = $state('');
  let probing = $state(false);
  let probe = $state<Probe | null>(null);
  let batch = $state<string[]>([]);
  let error = $state<ApiError | null>(null);
  let presetId = $state('mp4-best');
  let draft = $state<JobOptions>(blankOptions());
  let showOptions = $state(false);
  let showCommand = $state(false);
  let command = $state('');
  let picked = $state<Set<number>>(new Set());
  let adding = $state(false);
  let savingPreset = $state(false);
  let presetName = $state('');
  let tick = $state(Date.now());

  const dir = $derived(draft.outputDir || app.settings?.downloadDir || '');
  const presets = $derived(allPresets());
  const active = $derived(probe !== null || batch.length > 0);
  const isWaitKind = $derived(probe?.kind === 'live' || probe?.kind === 'upcoming');
  const playlistItems = $derived.by(() => {
    if (probe?.kind !== 'playlist' || picked.size === probe.entries.length) return draft.playlistItems || null;
    return toRanges([...picked]);
  });
  const finalOptions = $derived<JobOptions>({
    ...$state.snapshot(draft),
    url: probe?.url ?? batch[0] ?? input.trim(),
    outputDir: dir,
    playlistItems,
  });

  $effect(() => { const t = setInterval(() => (tick = Date.now()), 30_000); return () => clearInterval(t); });

  // A link handed over by the clipboard watcher or drag & drop.
  $effect(() => {
    const u = app.incomingUrl;
    if (u) { app.incomingUrl = null; input = u; lookup(); }
  });

  // Live command preview.
  $effect(() => {
    if (!active) return;
    const opts = finalOptions;
    const t = setTimeout(async () => { try { command = await api.previewCommand(opts); } catch { /* ignore */ } }, 200);
    return () => clearTimeout(t);
  });

  function networkDefaults(): Partial<JobOptions> {
    return { ...(app.settings?.network ?? {}) };
  }

  function applyPreset(p: Preset) {
    presetId = p.id;
    const keep = { url: draft.url, outputDir: draft.outputDir, playlistItems: draft.playlistItems, noPlaylist: draft.noPlaylist };
    const net: Partial<JobOptions> = {};
    for (const k of NETWORK_KEYS) (net as Record<string, unknown>)[k] = draft[k];
    const opts: JobOptions = { ...blankOptions(), ...net, ...keep, ...structuredClone($state.snapshot(p.options)) } as JobOptions;
    // Waiting only makes sense for streams, but never lose it on an upcoming video.
    if (probe?.kind === 'upcoming' && !opts.waitForVideo) opts.waitForVideo = { min: 30, max: 60 };
    draft = opts;
    saveSettings({ lastPreset: p.id });
  }

  async function lookup() {
    error = null;
    const urls = extractUrls(input);
    if (!urls.length) { error = { message: 'Paste a link that starts with http:// or https://' }; return; }
    probe = null; batch = []; picked = new Set();
    draft = { ...blankOptions(urls[0], app.settings?.downloadDir ?? ''), ...networkDefaults() } as JobOptions;
    if (urls.length > 1) {
      batch = urls;
      applyPreset(presets.find(p => p.id === (app.settings?.lastPreset ?? 'mp4-best')) ?? BUILTIN_PRESETS[2]);
      return;
    }
    probing = true;
    try {
      const raw = await api.probe({ url: urls[0], cookiesBrowser: draft.cookiesBrowser, cookiesFile: draft.cookiesFile, proxy: draft.proxy });
      probe = normalizeProbe(raw, urls[0]);
      picked = new Set(probe.entries.map(e => e.index));
      const last = presets.find(p => p.id === app.settings?.lastPreset);
      applyPreset(isWaitKind ? BUILTIN_PRESETS[0] : (last && last.id !== 'live-mp4' ? last : BUILTIN_PRESETS[2]));
    } catch (e) {
      error = toApiError(e);
    } finally {
      probing = false;
    }
  }

  /** Skip the lookup (e.g. an unusual site) and download with the chosen options. */
  function tryAnyway() {
    const url = extractUrls(input)[0];
    if (!url) return;
    error = null;
    batch = [url];
    applyPreset(presets.find(p => p.id === (app.settings?.lastPreset ?? 'mp4-best')) ?? BUILTIN_PRESETS[2]);
  }

  async function paste() {
    const t = (await readClipboard()).trim();
    if (t) { input = t; lookup(); }
    else toast('info', 'Nothing to paste — copy a link first.');
  }

  function reset() { probe = null; batch = []; error = null; input = ''; picked = new Set(); showOptions = false; }

  async function chooseFolder() {
    const d = await pickFolder(dir);
    if (d) draft.outputDir = d;
  }

  async function start() {
    adding = true;
    try {
      const base = finalOptions;
      const urls = batch.length ? batch : [base.url];
      for (const url of urls) {
        await api.addJob({
          options: { ...base, url },
          title: batch.length > 1 ? null : probe?.title ?? null,
          thumbnail: probe?.thumbnail ?? null,
          isLive: isWaitKind,
        });
      }
      const net: Record<string, unknown> = {};
      for (const k of NETWORK_KEYS) net[k] = draft[k] ?? null;
      saveSettings({ network: net as never });
      toast('ok', urls.length > 1 ? `${urls.length} downloads added` : isWaitKind ? 'Added — waiting for it to start' : 'Added to the queue', {
        action: { label: 'View queue', run: () => (app.page = 'queue') },
      });
      reset();
    } catch (e) { toastError(e); }
    finally { adding = false; }
  }

  async function savePreset() {
    const name = presetName.trim();
    if (!name) return;
    const o = structuredClone($state.snapshot(draft)) as Partial<JobOptions>;
    for (const k of ['url', 'outputDir', 'playlistItems', ...NETWORK_KEYS] as const) delete (o as Record<string, unknown>)[k];
    const p: Preset = { id: `u-${Date.now()}`, name, icon: 'sliders', description: 'Your saved preset', options: o };
    await saveSettings({ presets: [...(app.settings?.presets ?? []), p] });
    presetId = p.id; savingPreset = false; presetName = '';
    toast('ok', `Saved preset “${name}”`);
  }

  async function deletePreset(p: Preset) {
    await saveSettings({ presets: (app.settings?.presets ?? []).filter(x => x.id !== p.id) });
    if (presetId === p.id) presetId = 'mp4-best';
  }

  const when = $derived(probe?.releaseTimestamp ? new Date(probe.releaseTimestamp * 1000).toLocaleString([], { weekday: 'short', hour: '2-digit', minute: '2-digit', month: 'short', day: 'numeric' }) : '');
  const countdown = $derived(probe?.releaseTimestamp ? until(probe.releaseTimestamp, tick / 1000) : '');
  const canGo = $derived(active && !adding && (probe?.kind !== 'playlist' || picked.size > 0));
</script>

<div class="page-in">
  <header class="row">
    <div class="grow"><h1>Download</h1><p class="muted">Paste a link to a video, playlist, channel or live stream.</p></div>
  </header>

  <SetupBanner />

  <form class="urlbar card" onsubmit={(e) => { e.preventDefault(); lookup(); }}>
    <Icon name="search" />
    <input type="text" placeholder="https://www.youtube.com/watch?v=…" bind:value={input} aria-label="Video link"
      onpaste={() => setTimeout(() => { if (isUrl(input.trim()) || extractUrls(input).length > 1) lookup(); }, 0)} />
    {#if input}<button type="button" class="btn icon ghost" aria-label="Clear" onclick={reset}><Icon name="x" size={16} /></button>{/if}
    <button type="button" class="btn" onclick={paste}><Icon name="clipboard" size={16} /> Paste</button>
    <button class="btn primary" disabled={probing || !input.trim()}>
      {#if probing}<span class="spin"><Icon name="refresh" size={16} /></span> Looking up…{:else}Look up{/if}
    </button>
  </form>

  {#if error}
    <div class="notice err">
      <Icon name="alert" size={20} />
      <div class="grow"><b>{error.message}</b>{#if error.hint}<div>{error.hint}</div>{/if}</div>
      <button class="btn sm" onclick={tryAnyway}>Try downloading anyway</button>
    </div>
  {/if}

  {#if !active && !probing && !error}
    <section class="tips">
      <div class="card pad tip"><span class="tipico"><Icon name="clipboard" size={22} /></span><b>1 · Paste a link</b><p class="muted">Video, playlist, channel or a stream that hasn't started yet. Several links at once also works.</p></div>
      <div class="card pad tip"><span class="tipico"><Icon name="film" size={22} /></span><b>2 · Pick what you want</b><p class="muted">Choose a preset like “Best MP4” or “Audio · MP3”. Every advanced option is one click away.</p></div>
      <div class="card pad tip"><span class="tipico"><Icon name="live" size={22} /></span><b>3 · Let it run</b><p class="muted">Scheduled streams are waited for and recorded from the start — even while the window is closed to the tray.</p></div>
    </section>
  {/if}

  {#if probe}
    <section class="card preview">
      <div class="thumb">
        {#if probe.thumbnail}<img src={probe.thumbnail} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<Icon name="film" size={34} />{/if}
        {#if probe.duration}<span class="dur">{clock(probe.duration)}</span>{/if}
      </div>
      <div class="grow col" style="gap:6px">
        <div class="row wrap" style="gap:6px">
          {#if probe.kind === 'live'}<span class="pill live"><span class="dot pulse"></span> LIVE NOW</span>
          {:else if probe.kind === 'upcoming'}<span class="pill warn"><Icon name="clock" size={12} /> SCHEDULED{countdown ? ` · in ${countdown}` : ''}</span>
          {:else if probe.kind === 'playlist'}<span class="pill accent">PLAYLIST · {probe.entries.length} videos</span>
          {:else}<span class="pill">VIDEO</span>{/if}
          {#if probe.channel}<span class="muted small">{probe.channel}</span>{/if}
        </div>
        <h2>{probe.title}</h2>
        {#if probe.kind === 'upcoming'}
          <p class="muted">{#if when}Starts <b>{when}</b>.{/if} YT Grab will keep checking and start recording automatically{draft.liveFromStart ? ' from the beginning' : ''}. {probe.note ?? ''}</p>
        {:else if probe.kind === 'live'}
          <p class="muted">Streaming right now. {draft.liveFromStart ? 'Recording will start from the beginning of the stream.' : 'Recording will start from now.'}</p>
        {/if}
      </div>
    </section>

    {#if probe.kind === 'playlist'}
      <section class="card">
        <div class="row pad" style="padding-bottom:10px">
          <h3 class="grow">Choose videos ({picked.size} of {probe.entries.length})</h3>
          <button class="btn sm ghost" onclick={() => (picked = new Set(probe!.entries.map(e => e.index)))}>All</button>
          <button class="btn sm ghost" onclick={() => (picked = new Set())}>None</button>
        </div>
        <div class="entries">
          {#each probe.entries as e (e.index)}
            <label class="entry">
              <input type="checkbox" checked={picked.has(e.index)}
                onchange={(ev) => { const s = new Set(picked); ev.currentTarget.checked ? s.add(e.index) : s.delete(e.index); picked = s; }} />
              <span class="muted num">{e.index}</span><span class="grow ellipsis">{e.title}</span>
              <span class="muted small">{e.duration ? clock(e.duration) : ''}</span>
            </label>
          {/each}
        </div>
      </section>
    {/if}
  {:else if batch.length}
    <section class="card pad col" style="gap:6px">
      <h3>{batch.length} link{batch.length > 1 ? 's' : ''} ready</h3>
      {#each batch.slice(0, 6) as u}<div class="ellipsis mono muted">{u}</div>{/each}
      {#if batch.length > 6}<div class="muted small">…and {batch.length - 6} more</div>{/if}
    </section>
  {/if}

  {#if active}
    <section class="col" style="gap:8px">
      <h3>What do you want?</h3>
      <div class="presets">
        {#each presets as p (p.id)}
          <div class="presetwrap">
            <button class="preset" class:on={presetId === p.id} onclick={() => applyPreset(p)} title={p.description}>
              <Icon name={p.icon} size={18} /><span>{p.name}</span>
            </button>
            {#if !p.builtin}<button class="x" aria-label="Delete preset {p.name}" onclick={() => deletePreset(p)}><Icon name="x" size={12} /></button>{/if}
          </div>
        {/each}
      </div>
      <p class="muted small">{presets.find(p => p.id === presetId)?.description}</p>
    </section>

    <section class="row wrap dest">
      <Icon name="folder" />
      <span class="muted">Save to</span>
      <code class="ellipsis grow" title={dir}>{dir}</code>
      <button class="btn sm" onclick={chooseFolder}>Change…</button>
    </section>

    <section class="col" style="gap:10px">
      <div class="row wrap">
        <button class="btn" onclick={() => (showOptions = !showOptions)} aria-expanded={showOptions}>
          <Icon name="sliders" size={16} /> More options <span style="display:inline-flex; transform: rotate({showOptions ? 180 : 0}deg)"><Icon name="chevron" size={14} /></span>
        </button>
        <button class="btn ghost" onclick={() => (showCommand = !showCommand)} aria-expanded={showCommand}><Icon name="terminal" size={16} /> {showCommand ? 'Hide' : 'Show'} command</button>
        <span class="grow"></span>
        {#if savingPreset}
          <form class="row" onsubmit={(e) => { e.preventDefault(); savePreset(); }}>
            <input type="text" placeholder="Preset name" bind:value={presetName} style="width:170px" />
            <button class="btn sm primary" disabled={!presetName.trim()}>Save</button>
            <button type="button" class="btn sm ghost" onclick={() => (savingPreset = false)}>Cancel</button>
          </form>
        {:else}
          <button class="btn ghost" onclick={() => (savingPreset = true)}><Icon name="plus" size={16} /> Save as preset</button>
        {/if}
      </div>
      {#if showOptions}<OptionsPanel bind:draft {probe} />{/if}
      {#if showCommand}
        <div class="cmd card">
          <code>{command}</code>
          <button class="btn sm" onclick={() => { copyText(command); toast('ok', 'Command copied'); }}><Icon name="copy" size={14} /> Copy</button>
        </div>
      {/if}
    </section>

    <div class="gobar">
      <button class="btn primary big" disabled={!canGo} onclick={start}>
        {#if isWaitKind}<Icon name="live" />Wait &amp; record{:else}<Icon name="download" />Download{batch.length > 1 ? ` ${batch.length} videos` : probe?.kind === 'playlist' ? ` ${picked.size} videos` : ''}{/if}
      </button>
      <button class="btn big ghost" onclick={reset}>Cancel</button>
    </div>
  {/if}
</div>

<style>
  .page-in { max-width: 920px; margin: 0 auto; padding: 26px 28px 40px; display: flex; flex-direction: column; gap: 18px; }
  .urlbar { display: flex; align-items: center; gap: 10px; padding: 8px 8px 8px 16px; }
  .urlbar > :global(svg:first-child) { color: var(--muted); flex: none; }
  .urlbar input { border: 0; background: none; height: 42px; font-size: 15px; outline: none !important; }
  .tips { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; }
  .tip { display: flex; flex-direction: column; gap: 6px; }
  .tipico { width: 42px; height: 42px; border-radius: 12px; display: grid; place-items: center; background: var(--accent-soft); color: var(--accent); margin-bottom: 4px; }
  .preview { display: flex; gap: 18px; padding: 16px; align-items: center; }
  .thumb { position: relative; width: 208px; aspect-ratio: 16/9; flex: none; border-radius: 10px; overflow: hidden; background: var(--panel-2); display: grid; place-items: center; color: var(--muted); }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .dur { position: absolute; right: 6px; bottom: 6px; background: rgba(0,0,0,.78); color: #fff; font-size: 12px; padding: 1px 6px; border-radius: 5px; font-variant-numeric: tabular-nums; }
  .entries { max-height: 240px; overflow-y: auto; border-top: 1px solid var(--line); }
  .entry { display: flex; align-items: center; gap: 10px; padding: 7px 18px; cursor: pointer; }
  .entry:hover { background: var(--panel-2); }
  .num { width: 28px; text-align: right; font-variant-numeric: tabular-nums; }
  .presets { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 8px; }
  .presetwrap { position: relative; }
  .preset { width: 100%; display: flex; align-items: center; gap: 10px; padding: 11px 13px; border: 1px solid var(--line); background: var(--panel); border-radius: 11px; font-weight: 600; text-align: left; }
  .preset:hover { border-color: color-mix(in srgb, var(--line), var(--text) 25%); }
  .preset.on { border-color: var(--accent); background: var(--accent-soft); color: var(--accent); box-shadow: 0 0 0 1px var(--accent) inset; }
  .presetwrap .x { position: absolute; top: 4px; right: 4px; width: 20px; height: 20px; border-radius: 50%; border: 0; background: var(--panel-2); color: var(--muted); display: none; place-items: center; padding: 0; }
  .presetwrap:hover .x { display: grid; }
  .dest { background: var(--panel); border: 1px solid var(--line); border-radius: var(--radius-sm); padding: 8px 10px 8px 14px; }
  .cmd { display: flex; align-items: flex-start; gap: 12px; padding: 12px 14px; }
  .cmd code { flex: 1; word-break: break-all; white-space: pre-wrap; line-height: 1.6; }
  .gobar { display: flex; gap: 10px; position: sticky; bottom: 0; padding: 12px 0 2px; background: linear-gradient(transparent, var(--bg) 35%); }
  @media (max-width: 780px) { .tips { grid-template-columns: 1fr; } .preview { flex-direction: column; align-items: stretch; } .thumb { width: 100%; } }
</style>
