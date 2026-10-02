<script lang="ts">
  import Icon from './Icon.svelte';
  import { app, installTools, missingTools } from '../state.svelte';
  import { bytes } from '../format';

  const missing = $derived(missingTools());
  const NAMES: Record<string, string> = { 'yt-dlp': 'the downloader (yt-dlp)', ffmpeg: 'ffmpeg (merges video and audio)', deno: 'deno (needed for YouTube)' };
  const current = $derived(Object.values(app.binProgress).find(p => p.stage !== 'done'));
</script>

{#if missing.length}
  <div class="notice warn">
    <Icon name="download" size={20} />
    <div class="grow col" style="gap:8px">
      {#if app.installing}
        <div><b>Setting up… {current?.name ?? ''}</b>
          <span class="small">{current?.stage === 'downloading' ? `${bytes(current.downloaded)}${current.total ? ' of ' + bytes(current.total) : ''}` : current?.stage ?? ''}</span></div>
        <div class="progress" class:indeterminate={!current?.total}><i style="width:{current?.total ? (current.downloaded / current.total) * 100 : 40}%"></i></div>
      {:else}
        <div><b>A one-time setup is needed.</b> YT Grab will download {missing.map(m => NAMES[m.name]).join(', ')} into its own folder — nothing is installed on your system.</div>
      {/if}
    </div>
    <button class="btn primary" disabled={app.installing} onclick={() => installTools(missing.map(m => m.name))}>
      {app.installing ? 'Downloading…' : 'Set up now'}
    </button>
  </div>
{/if}
