<script lang="ts">
  import { bytes } from '../format';
  import type { FormatRow, JobOptions } from '../types';

  let { formats, draft = $bindable() }: { formats: FormatRow[]; draft: JobOptions } = $props();

  let show = $state<'video' | 'audio'>('video');
  let video = $state<string | null>(null);
  let audio = $state<string | null>(null);

  const videos = $derived(formats.filter(f => f.kind !== 'audio').sort((a, b) => b.height - a.height || (b.tbr ?? 0) - (a.tbr ?? 0)));
  const audios = $derived(formats.filter(f => f.kind === 'audio').sort((a, b) => (b.tbr ?? 0) - (a.tbr ?? 0)));
  const rows = $derived(show === 'video' ? videos : audios);

  function apply() {
    const v = formats.find(f => f.id === video);
    if (!v && !audio) return;
    if (v?.kind === 'both') draft.format = v.id;
    else draft.format = `${v?.id ?? 'bv*'}+${audio ?? 'ba'}`;
    if (!v && audio && draft.audioOnly) draft.format = audio;
  }
  function pick(f: FormatRow) {
    if (f.kind === 'audio') audio = audio === f.id ? null : f.id;
    else video = video === f.id ? null : f.id;
    apply();
  }
  function reset() { video = audio = null; draft.format = null; }
  const chosen = (f: FormatRow) => f.id === video || f.id === audio;
</script>

<div class="col" style="gap:8px">
  <div class="row wrap">
    <div class="row" style="gap:6px">
      <button class="chip" class:on={show === 'video'} onclick={() => (show = 'video')}>Video ({videos.length})</button>
      <button class="chip" class:on={show === 'audio'} onclick={() => (show = 'audio')}>Audio ({audios.length})</button>
    </div>
    <span class="grow"></span>
    {#if video || audio}<button class="btn sm ghost" onclick={reset}>Clear choice</button>{/if}
  </div>
  <div class="tablewrap card">
    <table>
      <thead><tr><th></th><th>ID</th><th>Quality</th><th>Type</th><th>Codec</th><th>Size</th><th>Notes</th></tr></thead>
      <tbody>
        {#each rows as f (f.id)}
          <tr class:sel={chosen(f)} onclick={() => pick(f)}>
            <td><input type="checkbox" checked={chosen(f)} tabindex="-1" aria-label="Use format {f.id}" /></td>
            <td class="mono">{f.id}</td>
            <td><b>{f.kind === 'audio' ? `${Math.round(f.tbr ?? 0)} kbps` : `${f.height}p${f.fps && f.fps > 30 ? f.fps : ''}`}</b></td>
            <td>{f.ext.toUpperCase()}{f.kind === 'both' ? ' · video+audio' : ''}</td>
            <td class="muted">{f.kind === 'audio' ? f.acodec : f.vcodec}</td>
            <td>{bytes(f.size)}</td>
            <td class="muted">{[f.hdr, f.note].filter(Boolean).join(' · ')}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
  <p class="muted small">
    Pick one video and (optionally) one audio stream. Sizes marked “—” are unknown until download.
    {#if draft.format}Selected: <code>{draft.format}</code>{/if}
  </p>
</div>

<style>
  .tablewrap { max-height: 260px; overflow: auto; box-shadow: none; }
  tbody tr { cursor: pointer; }
  tbody tr:hover { background: var(--panel-2); }
  tr.sel { background: var(--accent-soft) !important; }
</style>
