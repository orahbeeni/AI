<script lang="ts">
  import Icon from './Icon.svelte';
  import JobStats from './JobStats.svelte';
  import { api, copyText } from '../api';
  import { bytes, clock, isActive, shortName, speed } from '../format';
  import { app, modelLabel, toast, toastError } from '../state.svelte';
  import { DEFAULT_TRANSCRIBE } from '../defaults';
  import type { Job } from '../types';

  let { job }: { job: Job } = $props();
  let open = $state(false);
  let log = $state<string[]>([]);
  let logPath = $state<string | null>(null);

  const running = $derived(isActive(job));
  const finished = $derived(['done', 'failed', 'cancelled'].includes(job.status));
  const name = $derived(job.title ?? job.url);
  const LABEL: Record<string, string> = {
    queued: 'Queued', starting: 'Starting', waiting: 'Waiting to start', downloading: 'Downloading', live: 'Recording live',
    processing: 'Processing', done: 'Finished', failed: 'Failed', cancelled: 'Stopped',
  };
  const tone = $derived(job.status === 'done' ? 'ok' : job.status === 'failed' ? 'err' : job.status === 'waiting' ? 'warn' : job.status === 'cancelled' || job.status === 'queued' ? '' : 'accent');

  async function toggle() {
    open = !open;
    if (open) {
      try { log = await api.jobLog(job.id); } catch { /* ignore */ }
      try { logPath = await api.jobLogPath(job.id); } catch { logPath = null; }
    }
  }
  const guard = (p: Promise<unknown>) => p.catch(toastError);
  // Stopped part-way: carrying on is better than starting over.
  const continuable = $derived(!job.localFile && (job.status !== 'done' || !!job.transcriptError) && ((job.playlist?.[1] ?? 0) > 1 || job.downloaded > 0 || job.files.length > 0));
  const hasMedia = $derived(job.files.some(f => /\.(mp4|mkv|webm|mov|avi|ts|mp3|m4a|opus|ogg|flac|wav|aac)$/i.test(f)));
  async function transcribeNow() {
    const o = job.options.transcribe ?? { ...DEFAULT_TRANSCRIBE, model: (app.settings?.lastModel as string | undefined) ?? DEFAULT_TRANSCRIBE.model };
    toast('info', `Transcribing with the ${modelLabel(o.model)} model…`, { hint: 'The model is downloaded first if you do not have it yet.' });
    await guard(api.transcribeJob(job.id, o));
  }
</script>

<article class="card job" class:err={job.status === 'failed'}>
  <div class="thumb">
    {#if job.thumbnail}<img src={job.thumbnail} alt="" loading="lazy" referrerpolicy="no-referrer" />
    {:else}<Icon name={job.localFile ? 'tag' : job.options.audioOnly ? 'music' : job.isLive ? 'live' : 'film'} size={26} />{/if}
  </div>

  <div class="grow col" style="gap:7px">
    <div class="row" style="gap:8px">
      <b class="grow ellipsis" title={name}>{name}</b>
      {#if job.status === 'live'}<span class="pill live"><span class="dot pulse"></span> LIVE</span>{/if}
      <span class="pill {tone}">{LABEL[job.status]}{job.playlist ? ` · ${job.playlist[0]}/${job.playlist[1]}` : ''}</span>
    </div>

    {#if job.status === 'waiting'}
      <div class="wait"><Icon name="clock" size={16} /><span>{job.message ?? 'Waiting…'}</span></div>
      <p class="muted small">The app keeps checking and will start recording by itself. You can close the window — it keeps running in the tray.</p>
    {:else if job.status === 'live'}
      <div class="progress indeterminate"><i></i></div>
      <div class="meta"><span><b>Recording</b> {clock(job.elapsed)}</span><span>{bytes(job.downloaded)}</span><span>{speed(job.speed)}</span></div>
    {:else if job.status === 'downloading'}
      <div class="progress" class:indeterminate={job.percent == null}><i style="width:{job.percent ?? 40}%"></i></div>
      <div class="meta">
        {#if job.percent != null}<span><b>{job.percent.toFixed(1)}%</b></span>{/if}
        <span>{bytes(job.downloaded)}{job.total ? ` of ${bytes(job.total)}` : ''}</span>
        <span>{speed(job.speed)}</span>
        {#if job.eta}<span>{clock(job.eta)} left</span>{/if}
        {#if job.stream > 1}<span class="muted">part {job.stream}</span>{/if}
        {#if job.message}<span class="pill warn">{job.message}</span>{/if}
      </div>
    {:else if job.status === 'processing' || job.status === 'starting'}
      <div class="progress" class:indeterminate={job.percent == null || job.percent >= 100}><i style="width:{job.percent ?? 40}%"></i></div>
      <div class="meta"><span>{job.message ?? 'Working…'}</span></div>
    {:else if job.status === 'queued'}
      <div class="meta muted"><span>{job.message ?? 'Waiting for a free slot…'}</span></div>
    {:else if job.status === 'failed'}
      <div class="notice err small"><Icon name="alert" size={16} /><div><b>{job.error || "It failed, but no reason was recorded. Open Details for the log."}</b>{#if job.hint}<div>{job.hint}</div>{/if}</div></div>
    {:else if job.status === 'done'}
      {#if job.files.length}
        <div class="files">{#each job.files.slice(0, 3) as f}<span class="ellipsis mono muted" title={f}>{shortName(f)}</span>{/each}
          {#if job.files.length > 3}<span class="muted small">+{job.files.length - 3} more</span>{/if}</div>
      {:else}<p class="muted small">Nothing new to download.</p>{/if}
      {#if job.transcripts.length}
        <div class="files"><span class="small"><b>Transcript</b></span>
          {#each job.transcripts as f}<span class="ellipsis mono muted" title={f}>{shortName(f)}</span>{/each}</div>
      {/if}
      {#if job.transcriptError}
        <div class="notice warn small"><Icon name="alert" size={16} /><div>{job.transcriptError}<div class="muted">The download itself is fine.</div></div></div>
      {/if}
    {:else if job.status === 'cancelled'}
      <p class="muted small">Stopped. Partial files, if any, were kept in the download folder.</p>
    {/if}

    {#if job.status !== 'queued'}<JobStats {job} />{/if}

    {#if open}
      <div class="details col" style="gap:8px">
        <div class="cmd"><code>{job.command}</code>
          <button class="btn sm" onclick={() => { copyText(job.command); toast('ok', 'Command copied'); }}><Icon name="copy" size={14} /> Copy</button></div>
        <pre class="log">{(log.length ? log : job.log).join('\n') || 'No output yet.'}</pre>
        {#if logPath}
          <div class="row" style="gap:8px"><span class="muted small ellipsis grow" title={logPath}>Full log kept in {logPath}</span>
            <button class="btn sm" onclick={() => guard(api.openPath(logPath!))}><Icon name="external" size={14} /> Open log file</button></div>
        {/if}
      </div>
    {/if}
  </div>

  <div class="actions">
    {#if running}
      <button class="btn sm" title={job.status === 'live' ? 'Stop recording' : 'Stop'} onclick={() => guard(api.cancelJob(job.id))}><Icon name="stop" size={14} /> {job.status === 'live' ? 'Stop recording' : 'Stop'}</button>
    {:else if job.status === 'queued'}
      <button class="btn sm" onclick={() => guard(api.cancelJob(job.id))}><Icon name="x" size={14} /> Cancel</button>
    {/if}
    {#if job.status === 'failed' || job.status === 'cancelled' || job.status === 'done'}
      <button class="btn sm" title={continuable ? 'Picks up where it stopped: finished videos are skipped and an unfinished download resumes.' : undefined} onclick={() => guard(api.retryJob(job.id))}><Icon name="retry" size={14} /> {continuable ? 'Continue' : job.status === 'done' ? 'Again' : 'Retry'}</button>
    {/if}
    {#if job.status === 'done' && hasMedia && !job.transcripts.length}
      <button class="btn sm" title="Write a transcript with Whisper" onclick={transcribeNow}><Icon name="tag" size={14} /> {job.transcriptError ? 'Retry transcript' : 'Transcribe'}</button>
    {/if}
    {#if job.transcripts.length}
      <button class="btn sm" onclick={() => guard(api.openPath(job.transcripts[0]))}><Icon name="tag" size={14} /> Transcript</button>
    {/if}
    {#if job.files.length}
      <button class="btn sm" onclick={() => guard(api.revealPath(job.files[0]))}><Icon name="folder" size={14} /> Show</button>
      <button class="btn sm" onclick={() => guard(api.openPath(job.files[0]))}><Icon name="play" size={14} /> Open</button>
    {:else if finished}
      <button class="btn sm" onclick={() => guard(api.revealPath(job.options.outputDir))}><Icon name="folder" size={14} /> Folder</button>
    {/if}
    <button class="btn sm ghost" onclick={toggle} aria-expanded={open}><Icon name="terminal" size={14} /> Details</button>
    {#if !running}<button class="btn sm ghost danger" aria-label="Remove" onclick={() => guard(api.removeJob(job.id))}><Icon name="trash" size={14} /></button>{/if}
  </div>
</article>

<style>
  .job { display: flex; gap: 14px; padding: 14px; align-items: flex-start; }
  .job.err { border-color: color-mix(in srgb, var(--err), transparent 65%); }
  .thumb { width: 128px; aspect-ratio: 16/9; flex: none; border-radius: 9px; overflow: hidden; background: var(--panel-2); display: grid; place-items: center; color: var(--muted); }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .meta { display: flex; flex-wrap: wrap; gap: 4px 16px; font-size: 12.5px; color: var(--muted); font-variant-numeric: tabular-nums; }
  .meta b { color: var(--text); }
  .wait { display: flex; align-items: center; gap: 8px; color: var(--warn); font-weight: 600; background: var(--warn-soft); padding: 8px 11px; border-radius: 9px; font-variant-numeric: tabular-nums; }
  .files { display: flex; flex-direction: column; gap: 2px; }
  .actions { display: flex; flex-direction: column; align-items: stretch; gap: 6px; flex: none; min-width: 112px; }
  .actions .btn { justify-content: flex-start; }
  .details .cmd { display: flex; gap: 10px; align-items: flex-start; }
  .details .cmd code { flex: 1; word-break: break-all; }
  .log { margin: 0; max-height: 200px; overflow: auto; background: var(--panel-2); border: 1px solid var(--line); border-radius: 8px; padding: 10px; font: 12px/1.5 var(--mono); white-space: pre-wrap; word-break: break-all; }
  @media (max-width: 980px) { .job { flex-wrap: wrap; } .actions { flex-direction: row; flex-wrap: wrap; width: 100%; } .thumb { width: 96px; } }
</style>
