<script lang="ts">
  import Icon from './Icon.svelte';
  import { api, copyText } from '../api';
  import { bytes, clock, isActive, shortName, speed } from '../format';
  import { toast, toastError } from '../state.svelte';
  import type { Job } from '../types';

  let { job }: { job: Job } = $props();
  let open = $state(false);
  let log = $state<string[]>([]);

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
    if (open) { try { log = await api.jobLog(job.id); } catch { /* ignore */ } }
  }
  const guard = (p: Promise<unknown>) => p.catch(toastError);
</script>

<article class="card job" class:err={job.status === 'failed'}>
  <div class="thumb">
    {#if job.thumbnail}<img src={job.thumbnail} alt="" loading="lazy" referrerpolicy="no-referrer" />
    {:else}<Icon name={job.options.audioOnly ? 'music' : job.isLive ? 'live' : 'film'} size={26} />{/if}
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
      </div>
    {:else if job.status === 'processing' || job.status === 'starting'}
      <div class="progress indeterminate"><i></i></div>
      <div class="meta"><span>{job.message ?? 'Working…'}</span></div>
    {:else if job.status === 'queued'}
      <div class="meta muted"><span>{job.message ?? 'Waiting for a free slot…'}</span></div>
    {:else if job.status === 'failed'}
      <div class="notice err small"><Icon name="alert" size={16} /><div><b>{job.error}</b>{#if job.hint}<div>{job.hint}</div>{/if}</div></div>
    {:else if job.status === 'done'}
      {#if job.files.length}
        <div class="files">{#each job.files.slice(0, 3) as f}<span class="ellipsis mono muted" title={f}>{shortName(f)}</span>{/each}
          {#if job.files.length > 3}<span class="muted small">+{job.files.length - 3} more</span>{/if}</div>
      {:else}<p class="muted small">Nothing new to download.</p>{/if}
    {:else if job.status === 'cancelled'}
      <p class="muted small">Stopped. Partial files, if any, were kept in the download folder.</p>
    {/if}

    {#if open}
      <div class="details col" style="gap:8px">
        <div class="cmd"><code>{job.command}</code>
          <button class="btn sm" onclick={() => { copyText(job.command); toast('ok', 'Command copied'); }}><Icon name="copy" size={14} /> Copy</button></div>
        <pre class="log">{(log.length ? log : job.log).join('\n') || 'No output yet.'}</pre>
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
      <button class="btn sm" onclick={() => guard(api.retryJob(job.id))}><Icon name="retry" size={14} /> {job.status === 'done' ? 'Again' : 'Retry'}</button>
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
