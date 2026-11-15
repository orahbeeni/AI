<script lang="ts">
  import { bytes, dur, speed } from '../format';
  import type { Job } from '../types';

  let { job }: { job: Job } = $props();

  const ORDER = ['wait', 'setup', 'prepare', 'download', 'record', 'convert', 'transcribe', 'diarize'];
  const RUNNING: Record<string, string> = {
    wait: 'Waiting to start', setup: 'Setting up transcription', prepare: 'Preparing', download: 'Downloading',
    record: 'Recording', convert: 'Converting', transcribe: 'Transcribing', diarize: 'Identifying speakers',
  };
  const DONE: Record<string, string> = {
    wait: 'Waited', setup: 'Set up transcription', prepare: 'Prepared', download: 'Downloaded',
    record: 'Recorded', convert: 'Converted', transcribe: 'Transcribed', diarize: 'Identified speakers',
  };

  // Tick once a second while a phase is running so the current time moves.
  let now = $state(Date.now() / 1000);
  $effect(() => {
    if (!job.phase) return;
    now = Date.now() / 1000;
    const t = setInterval(() => (now = Date.now() / 1000), 1000);
    return () => clearInterval(t);
  });

  const finished = $derived(['done', 'failed', 'cancelled'].includes(job.status));
  const rows = $derived(
    ORDER.map(k => {
      const active = job.phase === k;
      const secs = (job.timings?.[k] ?? 0) + (active && job.phaseStarted ? Math.max(0, now - job.phaseStarted) : 0);
      return { k, active, secs };
    }).filter(r => r.active || r.secs >= 1),
  );
  // Time the app was actually working, so a long wait for a stream doesn't hide a slow download.
  const worked = $derived(rows.filter(r => r.k !== 'wait').reduce((a, r) => a + r.secs, 0));

  // When the last run began and ended (wall clock), so a job that sat in the queue or was paused
  // can still be placed in time. Includes the date when it did not finish today.
  const span = $derived.by(() => {
    if (!job.startedAt || !job.finishedAt) return '';
    const a = new Date(job.startedAt * 1000), b = new Date(job.finishedAt * 1000);
    const hm = (d: Date) => d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    const day = (d: Date) => (d.toDateString() === new Date().toDateString() ? '' : d.toLocaleDateString([], { month: 'short', day: 'numeric' }) + ' ');
    const elapsed = job.finishedAt - job.startedAt;
    return `${day(a)}${hm(a)} → ${day(b)}${hm(b)}${elapsed > worked + 120 ? ` · ${dur(elapsed)} elapsed` : ''}`;
  });

  function detail(k: string, secs: number, active: boolean): string {
    if (active) return '';
    if (k === 'download' && job.outputBytes > 0 && secs >= 1) return `${bytes(job.outputBytes)} at ${speed(job.outputBytes / secs)}`;
    if (k === 'record' && job.outputBytes > 0) return bytes(job.outputBytes);
    if (k === 'transcribe' && job.transcriptStats && secs >= 1) {
      const t = job.transcriptStats;
      const ratio = t.audioSecs / secs;
      const pace = ratio >= 1 ? `${ratio.toFixed(ratio >= 10 ? 0 : 1)}× faster than real time` : `${(1 / ratio).toFixed(1)}× slower than real time`;
      return `${dur(t.audioSecs)} of audio · ${pace} · ${t.words.toLocaleString()} words`;
    }
    if (k === 'diarize' && job.transcriptStats?.speakers) {
      const n = job.transcriptStats.speakers;
      return `${n} speaker${n === 1 ? '' : 's'} found`;
    }
    return '';
  }
</script>

{#if rows.length}
  <div class="stats" aria-label="Timing">
    {#each rows as r (r.k)}
      {@const d = detail(r.k, r.secs, r.active)}
      <div class="stat" class:active={r.active}>
        <span class="k">{r.active ? RUNNING[r.k] : DONE[r.k]}{#if r.active}<i class="dot"></i>{/if}</span>
        <b>{dur(r.secs)}</b>
        {#if d}<span class="d">{d}</span>{/if}
      </div>
    {/each}
    {#if finished && rows.filter(r => r.k !== 'wait').length > 1}
      <div class="stat total"><span class="k">Total work</span><b>{dur(worked)}</b>
        {#if job.timings?.wait >= 1}<span class="d">plus {dur(job.timings.wait)} waiting</span>{/if}
        {#if span}<span class="d">{span}</span>{/if}</div>
    {/if}
  </div>
{/if}

<style>
  .stats { display: flex; flex-wrap: wrap; gap: 6px 8px; }
  .stat { display: flex; flex-direction: column; gap: 1px; padding: 6px 10px; border-radius: 9px; background: var(--panel-2); border: 1px solid var(--line); min-width: 86px; font-variant-numeric: tabular-nums; }
  .stat .k { font-size: 11px; color: var(--muted); text-transform: uppercase; letter-spacing: .04em; display: flex; align-items: center; gap: 6px; }
  .stat b { font-size: 14px; }
  .stat .d { font-size: 11.5px; color: var(--muted); }
  .stat.active { border-color: color-mix(in srgb, var(--accent), transparent 55%); background: color-mix(in srgb, var(--accent), transparent 92%); }
  .stat.total { margin-left: auto; }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--accent); animation: blink 1.2s ease-in-out infinite; }
  @keyframes blink { 50% { opacity: .25; } }
</style>
