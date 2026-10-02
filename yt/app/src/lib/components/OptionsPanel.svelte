<script lang="ts">
  import Icon from './Icon.svelte';
  import FormatTable from './FormatTable.svelte';
  import { BROWSERS, capSort, FILENAME_TEMPLATES, FILENAME_TOKENS, SPONSOR_CATEGORIES } from '../defaults';
  import { clock, parseClock } from '../format';
  import { pickFile } from '../api';
  import type { JobOptions, Probe } from '../types';

  let { draft = $bindable(), probe = null }: { draft: JobOptions; probe?: Probe | null } = $props();

  const TABS = [
    ['quality', 'Quality', 'film'], ['audio', 'Audio', 'music'], ['live', 'Live & scheduled', 'live'],
    ['subs', 'Subtitles', 'tag'], ['extras', 'Extras', 'shield'], ['playlist', 'Playlist', 'queue'],
    ['clip', 'Clip', 'scissors'], ['name', 'File names', 'folder'], ['net', 'Network & sign-in', 'sliders'],
    ['adv', 'Advanced', 'terminal'],
  ] as const;
  let tab = $state<(typeof TABS)[number][0]>('quality');

  // --- quality shortcuts
  const heights = [2160, 1440, 1080, 720, 480, 360];
  let cap = $derived(heights.find(h => draft.formatSort === capSort(h)) ?? 0);
  function setCap(h: number) { draft.formatSort = h ? capSort(h) : null; if (h && draft.format?.includes('height<=')) draft.format = null; }

  // --- live
  const waiting = $derived(!!draft.waitForVideo);
  function setWait(on: boolean) { draft.waitForVideo = on ? { min: 30, max: 60 } : null; }

  // --- subtitles
  function toggleLang(l: string) {
    draft.subtitles = draft.subtitles.includes(l) ? draft.subtitles.filter(x => x !== l) : [...draft.subtitles, l];
  }
  let langText = $derived(draft.subtitles.join(', '));

  // --- clip
  let clipOn = $state(false);
  let clipStart = $state('0:00');
  let clipEnd = $state('');
  $effect(() => {
    const s = parseClock(clipStart), e = parseClock(clipEnd);
    if (clipOn && s != null && e != null && e > s) draft.downloadSections = [`*${clipStart.trim()}-${clipEnd.trim()}`];
    else if (!clipOn) draft.downloadSections = [];
  });
  const clipMax = $derived(probe?.duration ?? 0);
  const clipValid = $derived(!clipOn || (parseClock(clipStart) != null && parseClock(clipEnd) != null && (parseClock(clipEnd) ?? 0) > (parseClock(clipStart) ?? 0)));

  // --- filenames
  const sample: Record<string, string> = $derived({
    title: probe?.title ?? 'My Video', uploader: probe?.channel ?? 'Some Channel', upload_date: '20260930', id: 'dQw4w9WgXcQ',
    resolution: '1920x1080', playlist: probe?.kind === 'playlist' ? probe.title : 'My Playlist', playlist_index: '01', ext: draft.audioOnly ? (draft.audioFormat ?? 'mp3') : (draft.container ?? 'mp4'),
  });
  const example = $derived((draft.outputTemplate || '%(title)s [%(id)s].%(ext)s').replace(/%\((\w+)\)s/g, (_, k) => sample[k] ?? k));

  async function chooseCookieFile() { const f = await pickFile(); if (f) draft.cookiesFile = f; }
  const num = (v: string) => (v.trim() === '' ? null : Math.max(0, Number(v)));
</script>

<div class="opts card">
  <div class="tabs" role="tablist" aria-label="Options">
    {#each TABS as [id, label, icon]}
      <button role="tab" aria-selected={tab === id} class:on={tab === id} onclick={() => (tab = id)}>
        <Icon name={icon} size={16} /><span>{label}</span>
      </button>
    {/each}
  </div>

  <div class="body">
    {#if tab === 'quality'}
      <div class="col">
        <div class="grid2">
          <label class="field">Video quality
            <select value={String(cap)} onchange={(e) => setCap(Number(e.currentTarget.value))} disabled={draft.audioOnly}>
              <option value="0">Best available</option>
              {#each heights as h}<option value={h}>Up to {h}p{h === 2160 ? ' (4K)' : ''}</option>{/each}
            </select>
            <span class="hint">Or pick exact streams from the table below.</span>
          </label>
          <label class="field">File type
            <select bind:value={draft.container} disabled={draft.audioOnly}>
              <option value={null}>Automatic</option>
              <option value="mp4">MP4 — plays everywhere</option>
              <option value="mkv">MKV — keeps any codec</option>
              <option value="webm">WebM</option>
            </select>
            <span class="hint">Converted without re-encoding when possible.</span>
          </label>
        </div>
        {#if probe?.formats.length}
          <FormatTable formats={probe.formats} bind:draft />
        {:else}
          <p class="muted">Look up a video first to see every available quality here.</p>
        {/if}
      </div>

    {:else if tab === 'audio'}
      <div class="col">
        <label class="check"><input type="checkbox" bind:checked={draft.audioOnly} />
          <div><b>Audio only</b><div class="hint">Skip the video and save just the sound.</div></div></label>
        <div class="grid2">
          <label class="field">Format
            <select bind:value={draft.audioFormat} disabled={!draft.audioOnly}>
              {#each ['mp3', 'm4a', 'opus', 'flac', 'wav', 'aac', 'vorbis'] as f}<option value={f}>{f.toUpperCase()}</option>{/each}
            </select>
          </label>
          <label class="field">Quality
            <select bind:value={draft.audioQuality} disabled={!draft.audioOnly}>
              <option value="0">Best</option><option value="3">High</option><option value="5">Medium</option><option value="9">Small file</option>
            </select>
          </label>
        </div>
      </div>

    {:else if tab === 'live'}
      <div class="col">
        <label class="check"><input type="checkbox" checked={waiting} onchange={(e) => setWait(e.currentTarget.checked)} />
          <div><b>Wait for it to start</b><div class="hint">For scheduled streams and premieres: keep checking until the video goes live, then start automatically.</div></div></label>
        {#if draft.waitForVideo}
          <div class="grid2">
            <label class="field">Check again after at least (seconds)
              <input type="number" min="1" bind:value={draft.waitForVideo.min} /></label>
            <label class="field">…and at most (seconds)
              <input type="number" min="1" bind:value={draft.waitForVideo.max} />
              <span class="hint">A random time in this range is used, which is friendlier to the site.</span></label>
          </div>
        {/if}
        <label class="check"><input type="checkbox" bind:checked={draft.liveFromStart} />
          <div><b>Record a live stream from its beginning</b><div class="hint">Instead of from the moment you joined. Works for YouTube streams that have DVR enabled.</div></div></label>
      </div>

    {:else if tab === 'subs'}
      <div class="col">
        {#if probe?.subtitleLangs.length}
          <div><h3>Available languages</h3>
            <div class="row wrap" style="margin-top:8px; gap:6px">
              {#each probe.subtitleLangs as l}<button class="chip" class:on={draft.subtitles.includes(l)} onclick={() => toggleLang(l)}>{l}</button>{/each}
            </div></div>
        {/if}
        <label class="field">Languages to download
          <input type="text" placeholder="en, fr — or en.* for all English variants" value={langText}
            oninput={(e) => (draft.subtitles = e.currentTarget.value.split(',').map(s => s.trim()).filter(Boolean))} />
          <span class="hint">Leave empty to skip subtitles.</span></label>
        <label class="check"><input type="checkbox" bind:checked={draft.autoSubs} />
          <div><b>Include auto-generated captions</b><div class="hint">Used when no human-written subtitles exist. Defaults to English if no language is set.</div></div></label>
        <label class="check"><input type="checkbox" bind:checked={draft.embedSubs} />
          <div><b>Embed inside the video file</b><div class="hint">Otherwise they are saved next to it as separate files.</div></div></label>
        <label class="field">Convert to
          <select bind:value={draft.subFormat}><option value={null}>Keep original</option><option value="srt">SRT</option><option value="vtt">VTT</option><option value="ass">ASS</option></select></label>
      </div>

    {:else if tab === 'extras'}
      <div class="grid2" style="align-items:start">
        <div class="col">
          <h3>Add to the file</h3>
          <label class="check"><input type="checkbox" bind:checked={draft.embedThumbnail} /><div><b>Cover art</b><div class="hint">Embed the thumbnail.</div></div></label>
          <label class="check"><input type="checkbox" bind:checked={draft.embedMetadata} /><div><b>Title, channel & description</b></div></label>
          <label class="check"><input type="checkbox" bind:checked={draft.embedChapters} /><div><b>Chapters</b></div></label>
          <h3 style="margin-top:6px">Save alongside</h3>
          <label class="check"><input type="checkbox" bind:checked={draft.writeThumbnail} /><div><b>Thumbnail image</b></div></label>
          <label class="check"><input type="checkbox" bind:checked={draft.writeDescription} /><div><b>Description text</b></div></label>
          <label class="check"><input type="checkbox" bind:checked={draft.writeInfoJson} /><div><b>Info file (.json)</b></div></label>
          <label class="check"><input type="checkbox" bind:checked={draft.splitChapters} /><div><b>Split into one file per chapter</b></div></label>
        </div>
        <div class="col">
          <h3>SponsorBlock <span class="muted" style="text-transform:none;letter-spacing:0;font-weight:400">(YouTube)</span></h3>
          <p class="muted small">Community-marked segments. Choose what to cut out of the video.</p>
          {#each SPONSOR_CATEGORIES as [id, label]}
            <label class="check"><input type="checkbox" checked={draft.sponsorblockRemove.includes(id)}
              onchange={(e) => (draft.sponsorblockRemove = e.currentTarget.checked ? [...draft.sponsorblockRemove, id] : draft.sponsorblockRemove.filter(x => x !== id))} />
              <div><b>Remove {label.toLowerCase()}</b></div></label>
          {/each}
        </div>
      </div>

    {:else if tab === 'playlist'}
      <div class="col">
        <label class="check"><input type="checkbox" bind:checked={draft.noPlaylist} />
          <div><b>Only this video</b><div class="hint">If a link points to a video inside a playlist, ignore the rest of the playlist.</div></div></label>
        <label class="check"><input type="checkbox" bind:checked={draft.playlistReverse} disabled={draft.noPlaylist} /><div><b>Newest last (reverse order)</b></div></label>
        <label class="check"><input type="checkbox" bind:checked={draft.useArchive} />
          <div><b>Skip videos I've already downloaded</b><div class="hint">Keeps a list so repeated runs only fetch new videos.</div></div></label>
        <label class="field">Only these items
          <input type="text" placeholder="e.g. 1-5, 8, 10-" bind:value={draft.playlistItems} disabled={draft.noPlaylist || probe?.kind === 'playlist'} />
          <span class="hint">{probe?.kind === 'playlist' ? 'Use the checkboxes in the playlist list above.' : 'Numbers and ranges, separated by commas.'}</span></label>
      </div>

    {:else if tab === 'clip'}
      <div class="col">
        <label class="check"><input type="checkbox" bind:checked={clipOn} />
          <div><b>Download only part of the video</b><div class="hint">Saves time and space when you only need a segment.</div></div></label>
        <div class="grid2">
          <label class="field">Start <input type="text" bind:value={clipStart} disabled={!clipOn} placeholder="0:00" />
            {#if clipMax}<input type="range" min="0" max={clipMax} step="1" value={parseClock(clipStart) ?? 0} disabled={!clipOn}
              oninput={(e) => (clipStart = clock(+e.currentTarget.value))} aria-label="Start position" />{/if}</label>
          <label class="field">End <input type="text" bind:value={clipEnd} disabled={!clipOn} placeholder={clipMax ? clock(clipMax) : '1:30'} />
            {#if clipMax}<input type="range" min="0" max={clipMax} step="1" value={parseClock(clipEnd) ?? clipMax} disabled={!clipOn}
              oninput={(e) => (clipEnd = clock(+e.currentTarget.value))} aria-label="End position" />{/if}</label>
        </div>
        {#if !clipValid}<p class="notice warn small">End must be after start. Use m:ss or h:mm:ss.</p>{/if}
        <label class="check"><input type="checkbox" bind:checked={draft.forceKeyframes} disabled={!clipOn} />
          <div><b>Cut exactly at the times</b><div class="hint">Slower (re-encodes the edges) but frame-accurate. Off = faster, may start a few seconds early.</div></div></label>
      </div>

    {:else if tab === 'name'}
      <div class="col">
        <label class="field">Pattern
          <select value={FILENAME_TEMPLATES.some(t => t[0] === draft.outputTemplate) ? draft.outputTemplate : ''}
            onchange={(e) => (draft.outputTemplate = e.currentTarget.value || draft.outputTemplate)}>
            <option value="">Custom…</option>
            {#each FILENAME_TEMPLATES as [v, l]}<option value={v}>{l}</option>{/each}
          </select></label>
        <input type="text" placeholder="%(title)s [%(id)s].%(ext)s" bind:value={draft.outputTemplate} aria-label="File name pattern" />
        <div class="row wrap" style="gap:6px">
          {#each FILENAME_TOKENS as [tok, label]}
            <button class="chip" onclick={() => (draft.outputTemplate = (draft.outputTemplate ?? '') + tok)} title={tok}>+ {label}</button>
          {/each}
        </div>
        <div class="notice"><Icon name="folder" size={16} /><div><span class="muted small">Example file name</span><div class="mono">{example}</div></div></div>
        <label class="check"><input type="checkbox" bind:checked={draft.restrictFilenames} />
          <div><b>Plain file names</b><div class="hint">Only letters, numbers and underscores — safest for old devices and network shares.</div></div></label>
      </div>

    {:else if tab === 'net'}
      <div class="col">
        <div class="grid2">
          <label class="field">Use cookies from
            <select bind:value={draft.cookiesBrowser}>
              <option value={null}>Nothing (not signed in)</option>
              {#each BROWSERS as b}<option value={b}>{b[0].toUpperCase() + b.slice(1)}</option>{/each}
            </select>
            <span class="hint">Lets YT Grab act as you: age-restricted, members-only, and “confirm you’re not a bot” videos. Close the browser if it fails.</span></label>
          <label class="field">…or a cookies file
            <div class="row"><input type="text" bind:value={draft.cookiesFile} placeholder="cookies.txt" />
              <button class="btn" onclick={chooseCookieFile}>Browse</button></div></label>
        </div>
        <div class="grid2">
          <label class="field">Speed limit <input type="text" placeholder="e.g. 2M or 500K" bind:value={draft.rateLimit} /></label>
          <label class="field">Proxy <input type="text" placeholder="http://host:port or socks5://…" bind:value={draft.proxy} /></label>
          <label class="field">Retries <input type="number" min="0" placeholder="10" value={draft.retries ?? ''} oninput={(e) => (draft.retries = num(e.currentTarget.value))} /></label>
          <label class="field">Parallel pieces <input type="number" min="1" max="16" placeholder="1" value={draft.concurrentFragments ?? ''} oninput={(e) => (draft.concurrentFragments = num(e.currentTarget.value))} />
            <span class="hint">Higher can speed up big files.</span></label>
        </div>
      </div>

    {:else if tab === 'adv'}
      <div class="col">
        <label class="field">Extra yt-dlp options
          <textarea rows="3" class="mono" placeholder="--user-agent &quot;My Agent&quot; --no-mtime" bind:value={draft.extraArgs}></textarea>
          <span class="hint">Typed exactly as on the command line. Anything here is added last, so it can override the choices above.</span></label>
      </div>
    {/if}
  </div>
</div>

<style>
  .opts { display: grid; grid-template-columns: 190px 1fr; min-height: 300px; overflow: hidden; }
  .tabs { display: flex; flex-direction: column; padding: 8px; gap: 2px; border-right: 1px solid var(--line); background: var(--panel-2); }
  .tabs button { display: flex; align-items: center; gap: 9px; padding: 8px 10px; border: 0; background: none; border-radius: 8px; color: var(--muted); font-weight: 550; text-align: left; font-size: 13px; }
  .tabs button:hover { background: var(--panel); color: var(--text); }
  .tabs button.on { background: var(--panel); color: var(--accent); box-shadow: var(--shadow); }
  .body { padding: 18px; min-width: 0; }
  .grid2 { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  @media (max-width: 1050px) { .opts { grid-template-columns: 1fr; } .tabs { flex-direction: row; flex-wrap: wrap; border-right: 0; border-bottom: 1px solid var(--line); } .grid2 { grid-template-columns: 1fr; } }
</style>
