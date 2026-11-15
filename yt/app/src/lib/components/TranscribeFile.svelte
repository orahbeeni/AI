<script lang="ts">
  import Icon from './Icon.svelte';
  import TranscriptFields from './TranscriptFields.svelte';
  import { api, pickMediaFiles } from '../api';
  import { DEFAULT_TRANSCRIBE } from '../defaults';
  import { shortName } from '../format';
  import { app, modelLabel, saveSettings, toast, toastError } from '../state.svelte';
  import type { TranscribeOptions } from '../types';

  // Make a transcript for audio or video that is already on this computer, with no download.
  // The choices are remembered, so the next file starts from the same setup.
  let files = $state<string[]>([]);
  let adding = $state(false);
  let t = $state<TranscribeOptions>({
    ...DEFAULT_TRANSCRIBE,
    model: (app.settings?.lastModel as string | undefined) ?? DEFAULT_TRANSCRIBE.model,
    ...structuredClone($state.snapshot(app.settings?.lastFileTranscribe ?? app.settings?.lastOptions?.transcribe ?? {})),
  });

  async function choose() {
    try {
      const picked = await pickMediaFiles();
      files = [...new Set([...files, ...picked])];
    } catch (e) { toastError(e); }
  }

  async function start() {
    adding = true;
    const mine = $state.snapshot(t) as TranscribeOptions;
    let made = 0;
    try {
      for (const f of files) {
        try { await api.transcribeFile(f, mine); made++; }
        catch (e) { toastError(e); }
      }
    } finally { adding = false; }
    if (!made) return;
    saveSettings({ lastFileTranscribe: mine, lastModel: mine.model });
    files = [];
    toast('ok', made > 1 ? `${made} transcripts added to the queue` : 'Transcript added to the queue', {
      hint: `Using the ${modelLabel(mine.model)} model. The transcript is saved next to each file.`,
      action: { label: 'View queue', run: () => (app.page = 'queue') },
    });
  }
</script>

<section class="card pad col" style="gap:12px">
  <div class="row" style="gap:12px">
    <span class="ico"><Icon name="tag" size={20} /></span>
    <div class="grow col" style="gap:2px">
      <b>Already have the video or audio?</b>
      <span class="muted small">Make a transcript of a file on this computer — nothing is downloaded or uploaded.</span>
    </div>
    <button class="btn" onclick={choose}><Icon name="folder" size={16} /> Choose files…</button>
  </div>

  {#if files.length}
    <div class="files col" style="gap:2px">
      {#each files as f (f)}
        <div class="row file">
          <Icon name="film" size={15} /><span class="grow ellipsis" title={f}>{shortName(f)}</span>
          <button class="btn icon ghost sm" aria-label="Remove {shortName(f)}" onclick={() => (files = files.filter(x => x !== f))}><Icon name="x" size={13} /></button>
        </div>
      {/each}
    </div>
    <TranscriptFields bind:t />
    <div class="row">
      <button class="btn primary" disabled={adding} onclick={start}>
        <Icon name="tag" size={16} /> Transcribe {files.length > 1 ? `${files.length} files` : 'this file'}
      </button>
      <button class="btn ghost" onclick={() => (files = [])}>Clear</button>
    </div>
  {/if}
</section>

<style>
  .ico { width: 42px; height: 42px; border-radius: 12px; display: grid; place-items: center; background: var(--accent-soft); color: var(--accent); flex: none; }
  .files { border: 1px solid var(--line); border-radius: 10px; padding: 4px 8px; max-height: 160px; overflow-y: auto; }
  .file { gap: 8px; padding: 2px 0; color: var(--muted); }
</style>
