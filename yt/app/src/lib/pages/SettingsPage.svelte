<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { api, pickFolder, pickFile } from '../api';
  import { bytes, shortName } from '../format';
  import { app, checkForYtdlpUpdate, installTools, saveSettings, toast, whisperAction, ytdlpUpdateAvailable } from '../state.svelte';
  import { transcribeMinutesPerHour } from '../format';

  const s = $derived(app.settings);
  const WHAT: Record<string, string> = {
    'yt-dlp': 'The downloader itself',
    ffmpeg: 'Merges video + audio, converts audio, embeds subtitles',
    ffprobe: 'Reads media details (comes with ffmpeg)',
    deno: 'Runs the scripts YouTube needs to reveal video links',
  };

  async function chooseDir() { const d = await pickFolder(s?.downloadDir); if (d) saveSettings({ downloadDir: d }); }
  async function chooseYtdlp() { const f = await pickFile(); if (f) { await saveSettings({ customYtdlpPath: f }); app.info = await api.appInfo(); } }
  async function setSource(v: 'managed' | 'system') { await saveSettings({ binarySource: v }); app.info = await api.appInfo(); }
  const prog = (n: string) => app.binProgress[n];
</script>

{#if s}
<div class="wrap">
  <header><h1>Settings</h1></header>

  <section class="card pad col">
    <div class="row"><h2 class="grow">Tools</h2>
      <button class="btn sm" onclick={async () => { await checkForYtdlpUpdate(); toast('info', ytdlpUpdateAvailable() ? `yt-dlp ${app.latestYtdlp} is available` : 'yt-dlp is up to date'); }}><Icon name="refresh" size={14} /> Check for updates</button></div>
    <p class="muted small">YT Grab keeps its own copies of these in its own folder, so it works the same on any computer and can run from a USB stick.</p>
    {#if ytdlpUpdateAvailable()}
      <div class="notice"><Icon name="download" size={18} /><div class="grow"><b>yt-dlp {app.latestYtdlp} is available.</b> Sites change often — updating fixes most “it stopped working” problems.</div>
        <button class="btn primary sm" disabled={app.installing} onclick={() => installTools(['yt-dlp'])}>Update now</button></div>
    {/if}
    <div class="tools">
      {#each app.info?.bins ?? [] as b (b.name)}
        <div class="tool">
          <div class="grow col" style="gap:2px">
            <div class="row" style="gap:8px"><b>{b.name}</b>
              {#if b.source === 'missing'}<span class="pill err">Not installed</span>
              {:else}<span class="pill {b.source === 'managed' ? 'ok' : ''}">{b.source === 'managed' ? 'Built in' : b.source === 'system' ? 'From your system' : 'Custom'}</span>{/if}
              {#if b.version}<span class="muted small mono">{b.version}</span>{/if}</div>
            <span class="muted small">{WHAT[b.name]}</span>
            {#if prog(b.name) && prog(b.name).stage !== 'done'}
              <div class="progress" style="margin-top:6px" class:indeterminate={!prog(b.name).total}><i style="width:{prog(b.name).total ? (prog(b.name).downloaded / (prog(b.name).total ?? 1)) * 100 : 40}%"></i></div>
              <span class="muted small">{prog(b.name).stage}{prog(b.name).stage === 'downloading' ? ` ${bytes(prog(b.name).downloaded)}` : ''}…</span>
            {/if}
          </div>
          <button class="btn sm" disabled={app.installing} onclick={() => installTools([b.name])}>
            {b.source === 'missing' ? 'Install' : b.source === 'managed' ? 'Reinstall / update' : 'Install built-in copy'}</button>
        </div>
      {/each}
    </div>
    <div class="row wrap">
      <button class="btn" disabled={app.installing} onclick={() => installTools(['yt-dlp', 'ffmpeg', 'ffprobe', 'deno'])}>Install / update everything</button>
    </div>
  </section>

  <section class="card pad col">
    <h2>Transcription</h2>
    <p class="muted small">Turns a download into text with time stamps, using Whisper on this computer — nothing is uploaded. Turn it on per download under Options → Transcript. The engine and models are fetched automatically the first time you need them; you can also get them ahead of time here.</p>
    {#if app.whisper}
      <div class="tools">
        <div class="tool">
          <div class="grow col" style="gap:2px">
            <div class="row" style="gap:8px"><b>Whisper engine</b>
              <span class="pill {app.whisper.engineInstalled ? 'ok' : 'err'}">{app.whisper.engineInstalled ? 'Installed' : 'Not installed'}</span></div>
            <span class="muted small">A private Python environment with faster-whisper (about 500 MB). Does not touch any Python on your computer.</span>
            {#if prog('whisper') && prog('whisper').stage !== 'done'}
              <div class="progress indeterminate" style="margin-top:6px"><i></i></div>
              <span class="muted small">{prog('whisper').stage}{prog('whisper').stage === 'downloading' ? ` ${bytes(prog('whisper').downloaded)}` : ''}…</span>
            {/if}
          </div>
          {#if app.whisper.engineInstalled}
            <button class="btn sm" disabled={app.whisperBusy} onclick={() => whisperAction(api.whisperRemoveEngine, 'Transcription engine removed')}>Remove</button>
          {:else}
            <button class="btn sm" disabled={app.whisperBusy} onclick={() => whisperAction(api.whisperInstallEngine, 'Transcription engine installed')}>Install</button>
          {/if}
        </div>
        <div class="tool">
          <div class="grow col" style="gap:2px">
            <div class="row" style="gap:8px"><b>Speaker labels</b>
              <span class="muted small">{app.whisper.speakersMb} MB</span>
              {#if app.whisper.speakersInstalled}<span class="pill ok">Downloaded</span>{/if}</div>
            <span class="muted small">Two small models that tell voices apart, so the transcript can say “Speaker 1”, “Speaker 2”… Approximate.</span>
            {#if prog('speakers') && prog('speakers').stage !== 'done'}
              <div class="progress" class:indeterminate={!prog('speakers').total} style="margin-top:6px"><i style="width:{prog('speakers').total ? (prog('speakers').downloaded / (prog('speakers').total ?? 1)) * 100 : 40}%"></i></div>
              <span class="muted small">{prog('speakers').stage}{prog('speakers').stage === 'downloading' ? ` ${bytes(prog('speakers').downloaded)}` : ''}…</span>
            {/if}
          </div>
          {#if app.whisper.speakersInstalled}
            <button class="btn sm ghost danger" disabled={app.whisperBusy} onclick={() => whisperAction(api.whisperDeleteSpeakers)}>Delete</button>
          {:else}
            <button class="btn sm" disabled={app.whisperBusy} onclick={() => whisperAction(api.whisperDownloadSpeakers, 'Speaker models downloaded')}>Download</button>
          {/if}
        </div>
        {#each app.whisper.models as m (m.name)}
          {@const mp = prog('model:' + m.name)}
          <div class="tool">
            <div class="grow col" style="gap:2px">
              <div class="row" style="gap:8px"><b>{m.label}</b>
                <span class="muted small">{m.sizeMb >= 1000 ? (m.sizeMb / 1000).toFixed(1) + ' GB' : m.sizeMb + ' MB'}</span>
                {#if m.installed}<span class="pill ok">Downloaded</span>{/if}
                {#if transcribeMinutesPerHour(m.name)}<span class="muted small">≈ {transcribeMinutesPerHour(m.name)} min per audio hour</span>{/if}</div>
              <span class="muted small">{m.hint}</span>
              {#if mp && mp.stage !== 'done'}
                <div class="progress" style="margin-top:6px"><i style="width:{mp.total ? (mp.downloaded / mp.total) * 100 : 40}%"></i></div>
                <span class="muted small">downloading {bytes(mp.downloaded)}{mp.total ? ` of ${bytes(mp.total)}` : ''}…</span>
              {/if}
            </div>
            {#if m.installed}
              <button class="btn sm ghost danger" disabled={app.whisperBusy} onclick={() => whisperAction(() => api.whisperDeleteModel(m.name))}>Delete</button>
            {:else}
              <button class="btn sm" disabled={app.whisperBusy} onclick={() => whisperAction(() => api.whisperDownloadModel(m.name), `${m.label} model downloaded`)}>Download</button>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <section class="card pad col">
    <h2>Downloads</h2>
    <div class="row wrap"><span class="muted">Default folder</span><code class="grow ellipsis" title={s.downloadDir}>{s.downloadDir}</code>
      <button class="btn sm" onclick={chooseDir}>Change…</button></div>
    <div class="grid">
      <label class="field">At the same time
        <select value={String(s.maxConcurrent)} onchange={(e) => saveSettings({ maxConcurrent: +e.currentTarget.value })}>
          {#each [1, 2, 3, 4, 6, 8] as n}<option value={n}>{n} download{n > 1 ? 's' : ''}</option>{/each}</select>
        <span class="hint">Streams that are waiting to start don't count.</span></label>
      <label class="field">When everything finishes
        <select value={s.afterQueue} onchange={(e) => saveSettings({ afterQueue: e.currentTarget.value as never })}>
          <option value="none">Do nothing</option><option value="notify">Show a notification</option>
          <option value="quit">Quit YT Grab</option><option value="shutdown">Shut down the computer</option></select></label>
    </div>
    <label class="check"><input type="checkbox" checked={s.notifications} onchange={(e) => saveSettings({ notifications: e.currentTarget.checked })} />
      <div><b>Desktop notifications</b><div class="hint">When a download finishes or fails.</div></div></label>
    <label class="check"><input type="checkbox" checked={s.closeToTray} onchange={(e) => saveSettings({ closeToTray: e.currentTarget.checked })} />
      <div><b>Keep running in the tray when I close the window</b><div class="hint">Only while downloads or waits are active. Needed to record scheduled streams while you're away.</div></div></label>
    <label class="check"><input type="checkbox" checked={!!s.clipboardWatch} onchange={(e) => saveSettings({ clipboardWatch: e.currentTarget.checked })} />
      <div><b>Offer links I copy</b><div class="hint">When you switch back to this window, offer to download a link from your clipboard.</div></div></label>
  </section>

  <section class="card pad col">
    <h2>Appearance</h2>
    <div class="row" style="gap:6px">
      {#each [['system', 'Match system'], ['light', 'Light'], ['dark', 'Dark']] as [v, l]}
        <button class="chip" class:on={(s.theme ?? 'system') === v} onclick={() => saveSettings({ theme: v as never })}>{l}</button>
      {/each}
    </div>
  </section>

  <section class="card pad col">
    <h2>Portable mode &amp; your data</h2>
    <div class="row" style="gap:10px"><span class="pill {app.info?.portable ? 'accent' : ''}"><Icon name="usb" size={12} /> {app.info?.portable ? 'Portable' : 'Installed'}</span>
      <span class="muted small">{app.info?.os} · {app.info?.arch}</span></div>
    <p class="muted small">
      {#if app.info?.portable}
        Everything — settings, queue, downloaded tools — lives in <code>{app.info?.dataDir}</code> next to the app. Copy the whole folder to another computer or USB stick and it keeps working.
      {:else}
        Settings and tools are stored in <code>{app.info?.dataDir}</code>. To run from a USB stick, put the app in a folder and create an empty file named <code>portable</code> next to it — data will then be kept in a <code>ytgrab-data</code> folder beside the app.
      {/if}
    </p>
    <div class="row wrap"><button class="btn sm" onclick={() => api.revealPath(app.info!.dataDir)}><Icon name="folder" size={14} /> Open data folder</button></div>
  </section>

  <details class="card pad">
    <summary>Advanced</summary>
    <div class="col" style="margin-top:12px">
      <label class="field">Which yt-dlp / ffmpeg to prefer
        <select value={s.binarySource} onchange={(e) => setSource(e.currentTarget.value as never)}>
          <option value="managed">The built-in copies (recommended)</option><option value="system">The ones installed on this computer</option></select></label>
      <div class="row wrap"><span class="muted">Custom yt-dlp file</span><code class="grow ellipsis">{s.customYtdlpPath ? shortName(s.customYtdlpPath) : 'none'}</code>
        <button class="btn sm" onclick={chooseYtdlp}>Choose…</button>
        {#if s.customYtdlpPath}<button class="btn sm ghost" onclick={async () => { await saveSettings({ customYtdlpPath: '' }); app.info = await api.appInfo(); }}>Clear</button>{/if}</div>
      <div class="row"><button class="btn danger sm" onclick={() => api.quit()}>Quit YT Grab (stops all downloads)</button></div>
    </div>
  </details>
</div>
{/if}

<style>
  .wrap { max-width: 860px; margin: 0 auto; padding: 26px 28px 40px; display: flex; flex-direction: column; gap: 16px; }
  .tools { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: var(--radius-sm); }
  .tool { display: flex; align-items: center; gap: 14px; padding: 12px 14px; border-bottom: 1px solid var(--line); }
  .tool:last-child { border-bottom: 0; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  summary { cursor: pointer; font-weight: 650; }
  @media (max-width: 760px) { .grid { grid-template-columns: 1fr; } }
</style>
