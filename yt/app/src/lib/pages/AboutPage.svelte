<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import { copyText } from '../api';
  import { app, toast } from '../state.svelte';

  const info = $derived(app.info);
  const build = $derived(info ? `Build ${info.buildNumber}${info.buildCommit ? ` (${info.buildCommit})` : ''}` : '');
</script>

{#if info}
<div class="wrap">
  <header><h1>About</h1></header>

  <section class="card pad col">
    <div class="row" style="gap:12px">
      <span class="logo"><Icon name="download" size={22} /></span>
      <div class="col" style="gap:2px"><h2>YT Grab</h2><span class="muted small">Version {info.version}</span></div>
    </div>
    <div class="row wrap">
      <b>{build}</b>
      <button class="btn sm" onclick={async () => { await copyText(build); toast('ok', 'Build number copied'); }}><Icon name="copy" size={14} /> Copy</button>
    </div>
    <span class="muted small">Built {info.buildTime}</span>
    <p class="muted small">A friendly desktop front-end for yt-dlp for {info.os} ({info.arch}).</p>
  </section>
</div>
{/if}

<style>
  .wrap { max-width: 860px; margin: 0 auto; padding: 26px 28px 40px; display: flex; flex-direction: column; gap: 16px; }
  .logo { width: 44px; height: 44px; border-radius: 12px; display: grid; place-items: center; color: #fff; background: linear-gradient(135deg, #6366f1, #ec4899); }
</style>
