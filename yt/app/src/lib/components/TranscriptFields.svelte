<script lang="ts">
  import Icon from './Icon.svelte';
  import { SPEAKER_CHOICES, TIMESTAMP_INTERVALS, TRANSCRIBE_LANGUAGES } from '../defaults';
  import { transcribeMinutesPerHour } from '../format';
  import { app } from '../state.svelte';
  import type { TranscribeOptions } from '../types';

  /** The transcript choices: model, language, time stamps, speakers, file types. Used for downloads and for existing files. */
  let { t = $bindable() }: { t: TranscribeOptions } = $props();

  const models = $derived(app.whisper?.models ?? []);
  const chosen = $derived(models.find(m => m.name === t.model));
  function toggleFormat(f: string, on: boolean) {
    const next = on ? [...t.formats, f] : t.formats.filter(x => x !== f);
    t.formats = next.length ? next : ['txt'];
  }
  const fmtOn = (f: string) => t.formats.includes(f);
</script>

<div class="col">
  <label class="field">Model
    <select bind:value={t.model}>
      {#each models as m}<option value={m.name}>{m.label} — {m.installed ? 'downloaded' : `${m.sizeMb >= 1000 ? (m.sizeMb / 1000).toFixed(1) + ' GB' : m.sizeMb + ' MB'} to download`}</option>{/each}
    </select>
    <span class="hint">{chosen?.hint}
      {#if transcribeMinutesPerHour(t.model)} About {transcribeMinutesPerHour(t.model)} minutes per hour of audio on a 2016 laptop; faster on a newer one.{/if}</span></label>
  <div class="grid2">
    <label class="field">Language
      <select bind:value={t.language}>{#each TRANSCRIBE_LANGUAGES as [v, l]}<option value={v}>{l}</option>{/each}</select>
      <span class="hint">If you know it, pick it: automatic detection listens to the first 30 seconds and can guess wrong on talks that mix languages.</span></label>
    <label class="field">Time stamps
      <select bind:value={t.intervalSecs}>{#each TIMESTAMP_INTERVALS as [v, l]}<option value={v}>{l}</option>{/each}</select>
      <span class="hint">A [00:12:30] mark starts each paragraph, so you can find the same spot in the audio.</span></label>
  </div>
  <label class="field">Who is speaking
    <select value={t.speakers ?? 'off'} onchange={(e) => (t.speakers = e.currentTarget.value)}>
      {#each SPEAKER_CHOICES as [v, l]}<option value={v}>{l}</option>{/each}</select>
    <span class="hint">Marks each paragraph “Speaker 1:”, “Speaker 2:”… It is approximate: it works best when the voices sound clearly different and the audio is clean, and mixes people up when they talk over each other or sound alike. If you know how many people talk, choosing the number keeps the result tidier. Adds roughly 15–20 minutes per audio hour on a 2016 laptop.</span></label>
  <div class="col" style="gap:6px"><h3>Save as</h3>
    <label class="check"><input type="checkbox" checked={fmtOn('txt')} onchange={(e) => toggleFormat('txt', e.currentTarget.checked)} /><div><b>Text with time stamps</b> <span class="muted small">.txt</span></div></label>
    <label class="check"><input type="checkbox" checked={fmtOn('srt')} onchange={(e) => toggleFormat('srt', e.currentTarget.checked)} /><div><b>Subtitles</b> <span class="muted small">.srt — a time stamp on every line, opens in any video player</span></div></label>
    <label class="check"><input type="checkbox" checked={fmtOn('vtt')} onchange={(e) => toggleFormat('vtt', e.currentTarget.checked)} /><div><b>Web subtitles</b> <span class="muted small">.vtt</span></div></label>
  </div>
  {#if t.speakers && t.speakers !== 'off' && !app.whisper?.speakersInstalled}
    <p class="notice small"><Icon name="info" size={16} /><span>Speaker labels need two small models (about {app.whisper?.speakersMb ?? 36} MB), downloaded automatically the first time.</span></p>
  {/if}
  {#if !app.whisper?.engineInstalled || chosen?.installed === false}
    <p class="notice small"><Icon name="info" size={16} /><span>First use downloads the transcription engine{app.whisper?.engineInstalled ? '' : ' (about 500 MB)'} and the model. That needs internet once; after that it works offline.</span></p>
  {/if}
</div>

<style>
  .grid2 { display: grid; grid-template-columns: 1fr 1fr; gap: 14px; }
  @media (max-width: 1050px) { .grid2 { grid-template-columns: 1fr; } }
</style>
