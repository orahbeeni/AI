<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import EntryPicker from '../components/EntryPicker.svelte';
  import OptionsPanel from '../components/OptionsPanel.svelte';
  import SetupBanner from '../components/SetupBanner.svelte';
  import TranscribeFile from '../components/TranscribeFile.svelte';
  import { api, copyText, pickFolder, readClipboard, toApiError } from '../api';
  import { BUILTIN_PRESETS, BULK_MODES, MP4_BEST, NETWORK_KEYS, blankOptions, resolveBulkMode, type BulkMode } from '../defaults';
  import { ago, clock, dur, extractUrls, isUrl, normalizeProbe, toRanges, until } from '../format';
  import { allPresets, app, saveSettings, toast, toastError } from '../state.svelte';
  import type { ApiError, JobOptions, Preset, Probe, RecentLookup, Settings } from '../types';

  let input = $state('');
  let probing = $state(false);
  let probe = $state<Probe | null>(null);
  let batch = $state<string[]>([]);
  let error = $state<ApiError | null>(null);
  let presetId = $state('mp4-best');
  let draft = $state<JobOptions>(blankOptions());
  let showOptions = $state(false);
  let optionsTab = $state('quality');
  let showCommand = $state(false);
  let command = $state('');
  let picked = $state<Set<number>>(new Set());
  let adding = $state(false);
  /** The options that were last added; while nothing has changed the button says "Add again". */
  let addedKey = $state<string | null>(null);
  let savingPreset = $state(false);
  let presetName = $state('');
  let saveFolder = $state(false);
  let renaming = $state(false);
  let renameText = $state('');
  let tick = $state(Date.now());
  // The drop-down of earlier lookups under the link box.
  const RECENT_MAX = 10;
  let menuOpen = $state(false);
  let hi = $state(-1);
  /** Which recent entry the page is showing right now (its links joined), so changes are saved to it. */
  let currentKey = $state<string | null>(null);

  const recent = $derived(app.settings?.recent ?? []);
  const matches = $derived.by(() => {
    const q = input.trim().toLowerCase();
    return q ? recent.filter(r => r.title.toLowerCase().includes(q) || r.urls.some(u => u.toLowerCase().includes(q))) : recent;
  });
  const showMenu = $derived(menuOpen && matches.length > 0);
  const dir = $derived(draft.outputDir || app.settings?.downloadDir || '');
  const presets = $derived(allPresets());
  const active = $derived(probe !== null || batch.length > 0);
  const isWaitKind = $derived(probe?.kind === 'live' || probe?.kind === 'upcoming');
  const playlistItems = $derived.by(() => {
    if (probe?.kind !== 'playlist' || picked.size === probe.entries.length) return draft.playlistItems || null;
    return toRanges([...picked]);
  });
  // How many videos this will download, for pacing.
  const videoCount = $derived(batch.length || (probe?.kind === 'playlist' ? picked.size : 1));
  const bulkMode = $derived(resolveBulkMode(draft.bulkMode, videoCount));
  const bulk = $derived(BULK_MODES[bulkMode]);
  const finalOptions = $derived<JobOptions>({
    ...$state.snapshot(draft),
    url: probe?.url ?? batch[0] ?? input.trim(),
    outputDir: dir,
    playlistItems,
    pauseBetween: bulk.pause ? { ...bulk.pause } : null,
    oneAtATime: bulkMode !== 'quick',
  });
  // Roughly how much waiting the pauses add up to.
  const pauseTotal = $derived(bulk.pause ? videoCount * ((bulk.pause.min + bulk.pause.max) / 2) : 0);

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
    const keep = { url: draft.url, outputDir: draft.outputDir, playlistItems: draft.playlistItems, noPlaylist: draft.noPlaylist, transcribe: draft.transcribe };
    const net: Partial<JobOptions> = {};
    for (const k of NETWORK_KEYS) (net as Record<string, unknown>)[k] = draft[k];
    const opts: JobOptions = { ...blankOptions(), ...net, ...keep, ...structuredClone($state.snapshot(p.options)) } as JobOptions;
    // Waiting only makes sense for streams, but never lose it on an upcoming video.
    if (probe?.kind === 'upcoming' && !opts.waitForVideo) opts.waitForVideo = { min: 30, max: 60 };
    draft = opts;
    saveSettings({ lastPreset: p.id });
  }

  /**
   * Start a new link from what the user chose last time (preset, toggles, transcript, folder…),
   * not from a blank preset. Things that belong to one link are not carried over.
   */
  function applyDefaults(live: boolean) {
    const sticky = app.settings?.lastOptions;
    const home = !live ? presets.find(p => p.id === app.settings?.defaultPreset) : undefined;
    if (home) {
      applyPreset(home);
      return;
    }
    if (!sticky) {
      const last = presets.find(p => p.id === app.settings?.lastPreset);
      applyPreset(live ? BUILTIN_PRESETS[0] : (last && last.id !== 'live-mp4' ? last : BUILTIN_PRESETS[2]));
      return;
    }
    const opts = { ...blankOptions(draft.url, app.settings?.downloadDir ?? ''), ...networkDefaults(), ...structuredClone($state.snapshot(sticky)), url: draft.url } as JobOptions;
    if (live) {
      // Streams are waited for and recorded from the start (the safe way); the rest of the choices stay.
      opts.waitForVideo = { min: 30, max: 60 };
      opts.liveFromStart = true;
      if (!opts.audioOnly) opts.format ??= MP4_BEST;
    }
    draft = opts;
    const lastId = app.settings?.lastPreset;
    if (lastId && presets.some(p => p.id === lastId)) presetId = lastId;
  }

  /** Everything about this link that a later visit should get back. */
  function recentState(): Pick<RecentLookup, 'options' | 'presetId' | 'picked' | 'view'> {
    const options: Partial<JobOptions> = $state.snapshot(draft);
    delete options.url;
    const all = probe?.kind === 'playlist' && picked.size === probe.entries.length;
    return { options, presetId, view: { showOptions, tab: optionsTab }, picked: probe?.kind === 'playlist' && !all ? [...picked].sort((a, b) => a - b) : undefined };
  }

  /** Put this lookup at the top of the list (a repeat moves up instead of doubling). */
  function remember(urls: string[], title: string) {
    const key = urls.join('\n');
    currentKey = key;
    const entry: RecentLookup = { urls, title, thumbnail: probe?.thumbnail, kind: probe?.kind, at: Math.floor(Date.now() / 1000), ...recentState() };
    const rest = recent.filter(r => r.urls.join('\n') !== key);
    saveSettings({ recent: [entry, ...rest].slice(0, RECENT_MAX) });
  }

  /** Reopen an earlier lookup: look it up again (it may have changed) and bring its settings back. */
  function openRecent(r: RecentLookup) {
    menuOpen = false; hi = -1;
    input = r.urls.join(' ');
    lookup(r);
  }

  function applyRecent(r: RecentLookup) {
    const url = r.urls[0];
    const opts = { ...blankOptions(url, app.settings?.downloadDir ?? ''), ...networkDefaults(), ...structuredClone($state.snapshot(r.options)), url } as JobOptions;
    if (probe?.kind === 'upcoming' && !opts.waitForVideo) opts.waitForVideo = { min: 30, max: 60 };
    draft = opts;
    if (r.presetId) presetId = r.presetId;
    if (r.view) { showOptions = r.view.showOptions; optionsTab = r.view.tab; }
    if (probe?.kind === 'playlist' && r.picked) {
      const have = new Set(probe.entries.map(e => e.index));
      const keep = r.picked.filter(i => have.has(i));
      if (keep.length) picked = new Set(keep);
    }
  }

  function forget(r: RecentLookup) {
    const key = r.urls.join('\n');
    saveSettings({ recent: recent.filter(x => x.urls.join('\n') !== key) });
  }

  function summary(r: RecentLookup): string {
    const o = r.options;
    const parts = [o.audioOnly ? `Audio · ${(o.audioFormat || 'mp3').toUpperCase()}` : `Video${o.container ? ' · ' + o.container.toUpperCase() : ''}`];
    if (o.transcribe) parts.push(`Transcript (${o.transcribe.model}${o.transcribe.speakers && o.transcribe.speakers !== 'off' ? ', speakers' : ''})`);
    if (r.picked) parts.push(`${r.picked.length} picked`);
    else if (r.kind === 'playlist') parts.push('whole playlist');
    if (r.urls.length > 1) parts.push(`${r.urls.length} links`);
    return parts.join(' · ');
  }

  function onUrlKey(ev: KeyboardEvent) {
    if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
      if (!matches.length) return;
      ev.preventDefault();
      menuOpen = true;
      const n = matches.length;
      hi = ev.key === 'ArrowDown' ? Math.min(n - 1, hi + 1) : Math.max(-1, hi - 1);
    } else if (ev.key === 'Enter' && showMenu && hi >= 0 && matches[hi]) {
      ev.preventDefault();
      openRecent(matches[hi]);
    } else if (ev.key === 'Escape') {
      menuOpen = false; hi = -1;
    }
  }

  async function lookup(restore?: RecentLookup) {
    error = null; menuOpen = false;
    const urls = extractUrls(input);
    if (!urls.length) { error = { message: 'Paste a link that starts with http:// or https://' }; return; }
    probe = null; batch = []; picked = new Set(); currentKey = null;
    draft = { ...blankOptions(urls[0], app.settings?.downloadDir ?? ''), ...networkDefaults() } as JobOptions;
    if (urls.length > 1) {
      batch = urls;
      applyDefaults(false);
      if (restore) applyRecent(restore);
      remember(urls, `${urls.length} links`);
      return;
    }
    probing = true;
    try {
      const raw = await api.probe({ url: urls[0], cookiesBrowser: draft.cookiesBrowser, cookiesFile: draft.cookiesFile, proxy: draft.proxy });
      probe = normalizeProbe(raw, urls[0]);
      picked = new Set(probe.entries.map(e => e.index));
      applyDefaults(isWaitKind);
      if (restore) applyRecent(restore);
      remember(urls, probe.title);
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
    applyDefaults(false);
    remember([url], url);
  }

  async function paste() {
    const t = (await readClipboard()).trim();
    if (t) { input = t; lookup(); }
    else toast('info', 'Nothing to paste — copy a link first.');
  }

  /** Clear the link and start over. (Adding a download does NOT do this: the page stays as it was.) */
  function reset() { probe = null; batch = []; error = null; input = ''; picked = new Set(); addedKey = null; currentKey = null; }

  async function chooseFolder() {
    const d = await pickFolder(dir);
    if (d) draft.outputDir = d;
  }

  /** The choices worth keeping for the next link: everything except what belongs to this one. */
  function stickyOptions(): Partial<JobOptions> {
    const o: Partial<JobOptions> = $state.snapshot(draft);
    for (const k of ['url', 'playlistItems', 'noPlaylist', 'downloadSections', 'waitForVideo', 'liveFromStart'] as const) delete o[k];
    if (o.outputDir === app.settings?.downloadDir) delete o.outputDir;
    return o;
  }

  // Keep the user's choices as they make them, so they survive "Start over", a restart, or never adding.
  // The same goes for this link's entry in the recent list, so reopening it brings every choice back.
  $effect(() => {
    if (!active) return;
    const o = stickyOptions();
    const mine = recentState();
    const key = currentKey;
    const t = setTimeout(() => {
      const patch: Partial<Settings> = {};
      if (JSON.stringify(o) !== JSON.stringify(app.settings?.lastOptions ?? null)) patch.lastOptions = o;
      const list = app.settings?.recent ?? [];
      const at = key ? list.findIndex(r => r.urls.join('\n') === key) : -1;
      if (at >= 0) {
        const r = list[at];
        if (JSON.stringify([r.options, r.presetId ?? null, r.picked ?? null, r.view ?? null]) !== JSON.stringify([mine.options, mine.presetId ?? null, mine.picked ?? null, mine.view ?? null])) {
          patch.recent = list.map((x, i) => (i === at ? { ...x, ...mine } : x));
        }
      }
      if (Object.keys(patch).length) saveSettings(patch);
    }, 600);
    return () => clearTimeout(t);
  });

  async function start() {
    adding = true;
    try {
      const base = finalOptions;
      const key = JSON.stringify(base);
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
      saveSettings({ network: net as never, lastOptions: stickyOptions(), ...(base.transcribe ? { lastModel: base.transcribe.model } : {}) });
      toast('ok', urls.length > 1 ? `${urls.length} downloads added` : isWaitKind ? 'Added — waiting for it to start' : 'Added to the queue', {
        action: { label: 'View queue', run: () => (app.page = 'queue') },
      });
      addedKey = key; // the page stays exactly as it is, so more can be added with other choices
    } catch (e) { toastError(e); }
    finally { adding = false; }
  }

  /** The current choices as a preset: only what belongs to a recipe, not this link or the network. */
  function presetOptions(withFolder: boolean): Partial<JobOptions> {
    const o = structuredClone($state.snapshot(draft)) as Partial<JobOptions>;
    for (const k of ['url', 'playlistItems', ...NETWORK_KEYS] as const) delete (o as Record<string, unknown>)[k];
    if (!withFolder) delete o.outputDir;
    return o;
  }

  async function savePreset() {
    const name = presetName.trim();
    if (!name) return;
    const mine = app.settings?.presets ?? [];
    const same = mine.find(x => x.name.toLowerCase() === name.toLowerCase());
    const options = presetOptions(saveFolder);
    if (same) {
      // Saving under a name you already use updates that preset, so there are never two with one name.
      await saveSettings({ presets: mine.map(x => (x.id === same.id ? { ...x, options } : x)) });
      presetId = same.id;
      toast('ok', `Updated preset “${same.name}”`);
    } else {
      const p: Preset = { id: `u-${Date.now()}`, name, icon: 'sliders', description: saveFolder ? 'Your saved preset, including the download folder.' : 'Your saved preset', options };
      await saveSettings({ presets: [...mine, p] });
      presetId = p.id;
      toast('ok', `Saved preset “${name}”`);
    }
    savingPreset = false; presetName = ''; saveFolder = false;
  }

  /** Replace what a saved preset holds with the choices on screen now. */
  async function updatePreset(p: Preset) {
    const options = presetOptions('outputDir' in p.options);
    await saveSettings({ presets: (app.settings?.presets ?? []).map(x => (x.id === p.id ? { ...x, options } : x)) });
    presetId = p.id;
    toast('ok', `Updated preset “${p.name}”`);
  }

  async function renamePreset(p: Preset) {
    const name = renameText.trim();
    renaming = false;
    if (!name || name === p.name) return;
    if ((app.settings?.presets ?? []).some(x => x.id !== p.id && x.name.toLowerCase() === name.toLowerCase())) { toast('error', `There is already a preset called “${name}”.`); return; }
    await saveSettings({ presets: (app.settings?.presets ?? []).map(x => (x.id === p.id ? { ...x, name } : x)) });
  }

  /** Every new link starts from this preset; without one, from whatever was chosen last. */
  async function setDefault(p: Preset | null) {
    await saveSettings({ defaultPreset: p?.id ?? '' });
    toast('ok', p ? `New links will start from “${p.name}”.` : 'New links start from your last choices again.');
  }

  async function deletePreset(p: Preset) {
    await saveSettings({ presets: (app.settings?.presets ?? []).filter(x => x.id !== p.id), ...(app.settings?.defaultPreset === p.id ? { defaultPreset: '' } : {}) });
    if (presetId === p.id) presetId = 'mp4-best';
  }

  const when = $derived(probe?.releaseTimestamp ? new Date(probe.releaseTimestamp * 1000).toLocaleString([], { weekday: 'short', hour: '2-digit', minute: '2-digit', month: 'short', day: 'numeric' }) : '');
  const countdown = $derived(probe?.releaseTimestamp ? until(probe.releaseTimestamp, tick / 1000) : '');
  // A preset only counts as "selected" while the options really are that preset's: change the format,
  // tick audio only, and the highlight goes away instead of claiming something that is no longer true.
  // Compared on every setting that any preset controls, with blanks filled in: otherwise a preset with
  // no options of its own ("Best quality") would match everything.
  const presetKeys = $derived([...new Set(presets.flatMap(p => Object.keys(p.options)))].filter(k => k !== 'outputDir'));
  const norm = (v: unknown) => JSON.stringify(v ?? null);
  const matchesPreset = (p: Preset) => {
    const blank = blankOptions() as unknown as Record<string, unknown>;
    const mine = p.options as Record<string, unknown>;
    if ('outputDir' in mine && norm(draft.outputDir) !== norm(mine.outputDir)) return false;
    return presetKeys.every(k => norm((draft as unknown as Record<string, unknown>)[k]) === norm(k in mine ? mine[k] : blank[k]));
  };
  const activePresetId = $derived.by(() => {
    const chosen = presets.find(p => p.id === presetId);
    if (chosen && matchesPreset(chosen)) return chosen.id;
    return presets.find(matchesPreset)?.id ?? null;
  });
  // The preset these choices started from, when they have since been changed (e.g. Audio · MP3, then OPUS).
  const basedOn = $derived.by(() => {
    if (activePresetId) return null;
    const p = presets.find(x => x.id === presetId);
    return p && !!(p.options as Partial<JobOptions>).audioOnly === !!draft.audioOnly ? p : null;
  });
  /** The saved preset of yours that is selected, or that these choices started from. */
  const mine = $derived.by(() => {
    const p = presets.find(x => x.id === (activePresetId ?? basedOn?.id));
    return p && !p.builtin ? p : null;
  });
  const alreadyAdded = $derived(addedKey !== null && JSON.stringify(finalOptions) === addedKey);
  const canGo = $derived(active && !adding && (probe?.kind !== 'playlist' || picked.size > 0));
</script>

<div class="page-in">
  <header class="row">
    <div class="grow"><h1>Download</h1><p class="muted">Paste a link to a video, playlist, channel or live stream.</p></div>
  </header>

  <SetupBanner />

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="urlwrap" onfocusout={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) { menuOpen = false; hi = -1; } }}>
    <form class="urlbar card" onsubmit={(e) => { e.preventDefault(); lookup(); }}>
      <Icon name="search" />
      <input type="text" placeholder="https://www.youtube.com/watch?v=…" bind:value={input} aria-label="Video link"
        role="combobox" aria-expanded={showMenu} aria-controls="recent-list" autocomplete="off"
        onfocus={() => { if (recent.length) menuOpen = true; }}
        oninput={() => { menuOpen = true; hi = -1; }}
        onkeydown={onUrlKey}
        onpaste={() => setTimeout(() => { if (isUrl(input.trim()) || extractUrls(input).length > 1) lookup(); }, 0)} />
      {#if input}<button type="button" class="btn icon ghost" aria-label="Clear" onclick={reset}><Icon name="x" size={16} /></button>{/if}
      {#if recent.length}
        <button type="button" class="btn icon ghost" aria-label="Recent lookups" aria-expanded={showMenu} title="Recent lookups"
          onclick={() => (menuOpen = !menuOpen)}>
          <span style="display:inline-flex; transform: rotate({showMenu ? 180 : 0}deg)"><Icon name="chevron" size={16} /></span>
        </button>
      {/if}
      <button type="button" class="btn" onclick={paste}><Icon name="clipboard" size={16} /> Paste</button>
      <button class="btn primary" disabled={probing || !input.trim()}>
        {#if probing}<span class="spin"><Icon name="refresh" size={16} /></span> Looking up…{:else}Look up{/if}
      </button>
    </form>

    {#if showMenu}
      <div class="recent card" id="recent-list" role="listbox" aria-label="Recent lookups">
        <div class="row recenthead"><span class="muted small grow">Recent lookups · opens with the settings you used</span>
          <button type="button" class="btn sm ghost" onmousedown={(e) => e.preventDefault()} onclick={() => saveSettings({ recent: [] })}>Clear all</button>
        </div>
        {#each matches as r, i (r.urls.join('\n'))}
          <div class="ritem" class:hi={hi === i} role="option" aria-selected={hi === i}>
            <button type="button" class="rmain" onmousedown={(e) => e.preventDefault()} onclick={() => openRecent(r)} onpointerenter={() => (hi = i)}>
              <span class="rthumb">{#if r.thumbnail}<img src={r.thumbnail} alt="" loading="lazy" referrerpolicy="no-referrer" />{:else}<Icon name={r.urls.length > 1 ? 'queue' : 'film'} size={18} />{/if}</span>
              <span class="grow col" style="gap:1px; min-width:0">
                <b class="ellipsis">{r.title}</b>
                <span class="muted small ellipsis">{summary(r)}</span>
              </span>
              <span class="muted small rwhen">{ago(r.at)}</span>
            </button>
            <button type="button" class="rx" aria-label="Remove {r.title} from recent" onmousedown={(e) => e.preventDefault()} onclick={() => forget(r)}><Icon name="x" size={13} /></button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

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
    <TranscribeFile />
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
      <EntryPicker entries={probe.entries} bind:picked />
    {/if}
  {:else if batch.length}
    <section class="card pad col" style="gap:6px">
      <h3>{batch.length} link{batch.length > 1 ? 's' : ''} ready</h3>
      {#each batch.slice(0, 6) as u}<div class="ellipsis mono muted">{u}</div>{/each}
      {#if batch.length > 6}<div class="muted small">…and {batch.length - 6} more</div>{/if}
    </section>
  {/if}

  {#if active}
    {#if videoCount >= 2}
      <section class="col" style="gap:8px">
        <h3>How careful should it be? <span class="muted small" style="text-transform:none;letter-spacing:0;font-weight:400">· {videoCount} videos</span></h3>
        <div class="modes" role="radiogroup" aria-label="Bulk mode">
          {#each ['auto', 'quick', 'steady', 'careful'] as k}
            {@const on = (draft.bulkMode ?? 'auto') === k}
            <button class="preset mode" class:on role="radio" aria-checked={on} onclick={() => (draft.bulkMode = k as BulkMode | 'auto')}>
              <span class="grow col" style="gap:1px"><b>{k === 'auto' ? 'Auto' : BULK_MODES[k as BulkMode].label}</b>
                <span class="small mutedish">{k === 'auto' ? `→ ${bulk.label} for ${videoCount}` : BULK_MODES[k as BulkMode].short}</span></span>
            </button>
          {/each}
        </div>
        <p class="muted small" style="margin:0">{bulk.hint}{#if pauseTotal >= 60}{' '}Pauses add about {dur(pauseTotal)} in total.{/if}
          If YouTube pushes back anyway, the queue pauses by itself and tries again.</p>
      </section>
    {/if}

    <section class="col" style="gap:8px">
      <h3>What do you want?</h3>
      <div class="presets">
        {#each presets as p (p.id)}
          <div class="presetwrap">
            <button class="preset" class:on={activePresetId === p.id} class:based={basedOn?.id === p.id} onclick={() => applyPreset(p)} title={p.description}>
              <Icon name={p.icon} size={18} /><span>{p.name}</span>{#if app.settings?.defaultPreset === p.id}<span class="pill dflt" title="New links start from this preset">default</span>{/if}
            </button>
            {#if !p.builtin}<button class="x" aria-label="Delete preset {p.name}" onclick={() => deletePreset(p)}><Icon name="x" size={12} /></button>{/if}
          </div>
        {/each}
      </div>
      {#if mine}
        <div class="row wrap mine" role="group" aria-label="Your preset “{mine.name}”">
          {#if renaming}
            <form class="row" onsubmit={(e) => { e.preventDefault(); renamePreset(mine); }}>
              <input type="text" aria-label="New name" bind:value={renameText} style="width:170px" />
              <button class="btn sm primary" disabled={!renameText.trim()}>Rename</button>
              <button type="button" class="btn sm ghost" onclick={() => (renaming = false)}>Cancel</button>
            </form>
          {:else}
            <b class="small">“{mine.name}”</b>
            {#if !activePresetId}<button class="btn sm" onclick={() => updatePreset(mine)} title="Replace what this preset holds with the choices on screen"><Icon name="check" size={14} /> Save my changes to it</button>{/if}
            <button class="btn sm ghost" onclick={() => { renameText = mine.name; renaming = true; }}>Rename</button>
            {#if app.settings?.defaultPreset === mine.id}
              <button class="btn sm ghost" onclick={() => setDefault(null)} title="Go back to starting new links from your last choices">Stop using as default</button>
            {:else}
              <button class="btn sm ghost" onclick={() => setDefault(mine)} title="Every new link starts from this preset instead of your last choices">Use for every new link</button>
            {/if}
          {/if}
        </div>
      {/if}
      <p class="muted small">{activePresetId ? presets.find(p => p.id === activePresetId)?.description : basedOn ? `Started from “${basedOn.name}”, with your changes — saved for next time.` : 'Your own choices — saved for next time. Pick a preset above to start from one.'}</p>
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
            <input type="text" placeholder="Name this preset" aria-label="Preset name" bind:value={presetName} style="width:170px" />
            <label class="small row" style="gap:6px" title="Keep the 'Save to' folder as part of the preset"><input type="checkbox" bind:checked={saveFolder} /> with folder</label>
            <button class="btn sm primary" disabled={!presetName.trim()}>Save</button>
            <button type="button" class="btn sm ghost" onclick={() => { savingPreset = false; saveFolder = false; }}>Cancel</button>
          </form>
        {:else}
          <button class="btn ghost" onclick={() => (savingPreset = true)}><Icon name="plus" size={16} /> Save choices as preset</button>
        {/if}
      </div>
      {#if showOptions}<OptionsPanel bind:draft {probe} bind:tab={optionsTab} />{/if}
      {#if showCommand}
        <div class="cmd card">
          <code>{command}</code>
          <button class="btn sm" onclick={() => { copyText(command); toast('ok', 'Command copied'); }}><Icon name="copy" size={14} /> Copy</button>
        </div>
      {/if}
    </section>

    {#if alreadyAdded}
      <p class="notice ok small"><Icon name="check" size={16} /><span>Added to the queue. Everything stays as it is — change something to add it again with different choices, or press <b>Start over</b> to clear the link.</span></p>
    {/if}

    <div class="gobar">
      <button class="btn primary big" disabled={!canGo} onclick={start}>
        {#if alreadyAdded}<Icon name="download" />Add again
        {:else if isWaitKind}<Icon name="live" />Wait &amp; record{:else}<Icon name="download" />Download{batch.length > 1 ? ` ${batch.length} videos` : probe?.kind === 'playlist' ? ` ${picked.size} videos` : ''}{/if}
      </button>
      <button class="btn big ghost" onclick={reset}>Start over</button>
    </div>
  {/if}
</div>

<style>
  .page-in { max-width: 920px; margin: 0 auto; padding: 26px 28px 40px; display: flex; flex-direction: column; gap: 18px; }
  .urlwrap { position: relative; }
  .recent { position: absolute; left: 0; right: 0; top: calc(100% + 6px); z-index: 30; max-height: 440px; overflow-y: auto; padding: 6px; box-shadow: 0 12px 34px rgba(0,0,0,.35); }
  .recenthead { padding: 4px 6px 6px; }
  .ritem { position: relative; display: flex; align-items: center; border-radius: 9px; }
  .ritem.hi, .ritem:hover { background: var(--accent-soft); }
  .rmain { flex: 1; min-width: 0; display: flex; align-items: center; gap: 12px; padding: 7px 40px 7px 8px; border: 0; background: none; text-align: left; color: inherit; border-radius: 9px; }
  .rthumb { width: 64px; aspect-ratio: 16/9; flex: none; border-radius: 6px; overflow: hidden; background: var(--panel-2); display: grid; place-items: center; color: var(--muted); }
  .rthumb img { width: 100%; height: 100%; object-fit: cover; }
  .rwhen { flex: none; }
  .rx { position: absolute; right: 8px; top: 50%; transform: translateY(-50%); width: 22px; height: 22px; border-radius: 50%; border: 0; background: transparent; color: var(--muted); display: none; place-items: center; padding: 0; }
  .ritem:hover .rx, .ritem.hi .rx { display: grid; }
  .rx:hover { background: var(--panel-2); color: var(--text); }
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
  .modes { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 8px; }
  .mode { align-items: flex-start; }
  .mutedish { font-weight: 450; opacity: .75; }
  .presets { display: grid; grid-template-columns: repeat(auto-fill, minmax(190px, 1fr)); gap: 8px; }
  .presetwrap { position: relative; }
  .mine { gap: 8px; padding: 4px 2px; }
  .pill.dflt { margin-left: auto; font-size: 10px; text-transform: uppercase; letter-spacing: .04em; }
  .preset { width: 100%; display: flex; align-items: center; gap: 10px; padding: 11px 13px; border: 1px solid var(--line); background: var(--panel); border-radius: 11px; font-weight: 600; text-align: left; }
  .preset:hover { border-color: color-mix(in srgb, var(--line), var(--text) 25%); }
  .preset.based { border: 1.5px dashed var(--accent); color: var(--accent); }
  .preset.on { border-color: var(--accent); background: var(--accent-soft); color: var(--accent); box-shadow: 0 0 0 1px var(--accent) inset; }
  .presetwrap .x { position: absolute; top: 4px; right: 4px; width: 20px; height: 20px; border-radius: 50%; border: 0; background: var(--panel-2); color: var(--muted); display: none; place-items: center; padding: 0; }
  .presetwrap:hover .x { display: grid; }
  .dest { background: var(--panel); border: 1px solid var(--line); border-radius: var(--radius-sm); padding: 8px 10px 8px 14px; }
  .cmd { display: flex; align-items: flex-start; gap: 12px; padding: 12px 14px; }
  .cmd code { flex: 1; word-break: break-all; white-space: pre-wrap; line-height: 1.6; }
  .gobar { display: flex; gap: 10px; position: sticky; bottom: 0; padding: 12px 0 2px; background: linear-gradient(transparent, var(--bg) 35%); }
  @media (max-width: 780px) { .tips { grid-template-columns: 1fr; } .preview { flex-direction: column; align-items: stretch; } .thumb { width: 100%; } }
</style>
