<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from './lib/components/Icon.svelte';
  import Toasts from './lib/components/Toasts.svelte';
  import DownloadPage from './lib/pages/DownloadPage.svelte';
  import QueuePage from './lib/pages/QueuePage.svelte';
  import SubscriptionsPage from './lib/pages/SubscriptionsPage.svelte';
  import SettingsPage from './lib/pages/SettingsPage.svelte';
  import { app, init, type Page } from './lib/state.svelte';
  import { extractUrls, isActive } from './lib/format';

  const nav: { id: Page; label: string; icon: string }[] = [
    { id: 'download', label: 'Download', icon: 'download' },
    { id: 'queue', label: 'Queue', icon: 'queue' },
    { id: 'subs', label: 'Subscriptions', icon: 'rss' },
    { id: 'settings', label: 'Settings', icon: 'settings' },
  ];

  const activeCount = $derived(app.jobs.filter(j => isActive(j) || j.status === 'queued').length);
  let dragging = $state(false);

  onMount(() => { init(); });

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    const text = e.dataTransfer?.getData('text/uri-list') || e.dataTransfer?.getData('text/plain') || '';
    const urls = extractUrls(text.replace(/^#.*$/gm, ''));
    if (urls.length) { app.incomingUrl = urls.join('\n'); app.page = 'download'; }
  }
</script>

<svelte:window
  ondragover={(e) => { e.preventDefault(); dragging = true; }}
  ondragleave={(e) => { if (!e.relatedTarget) dragging = false; }}
  ondrop={onDrop}
/>

<div class="shell">
  <aside class="side">
    <div class="brand">
      <span class="logo"><Icon name="download" size={18} /></span>
      <b>YT Grab</b>
    </div>
    <nav>
      {#each nav as n}
        <button class="nav-item" class:on={app.page === n.id} onclick={() => (app.page = n.id)}>
          <Icon name={n.icon} />
          <span>{n.label}</span>
          {#if n.id === 'queue' && activeCount}<span class="badge">{activeCount}</span>{/if}
        </button>
      {/each}
    </nav>
    <div class="side-foot muted small">
      {#if app.info?.portable}<span class="pill accent"><Icon name="usb" size={12} /> Portable</span>{/if}
      <span>v{app.info?.version ?? ''}</span>
    </div>
  </aside>

  <main>
    {#if !app.ready}
      <div class="loading muted">Loading…</div>
    {:else}
      <!-- Pages stay mounted so half-filled forms survive tab switches. -->
      <div class="page" hidden={app.page !== 'download'}><DownloadPage /></div>
      <div class="page" hidden={app.page !== 'queue'}><QueuePage /></div>
      <div class="page" hidden={app.page !== 'subs'}><SubscriptionsPage /></div>
      <div class="page" hidden={app.page !== 'settings'}><SettingsPage /></div>
    {/if}
  </main>

  {#if dragging}
    <div class="drop"><div><Icon name="download" size={34} /><h2>Drop the link to add it</h2></div></div>
  {/if}
  <Toasts />
</div>

<style>
  .shell { display: grid; grid-template-columns: 208px 1fr; height: 100%; }
  .side { background: var(--panel); border-right: 1px solid var(--line); display: flex; flex-direction: column; padding: 16px 12px; gap: 18px; }
  .brand { display: flex; align-items: center; gap: 10px; padding: 4px 8px; font-size: 16px; letter-spacing: -.01em; }
  .logo { width: 30px; height: 30px; border-radius: 9px; display: grid; place-items: center; color: #fff; background: linear-gradient(135deg, #6366f1, #ec4899); }
  nav { display: flex; flex-direction: column; gap: 3px; }
  .nav-item { display: flex; align-items: center; gap: 11px; height: 40px; padding: 0 12px; border: 0; border-radius: 10px; background: none; color: var(--muted); font-weight: 550; text-align: left; }
  .nav-item:hover { background: var(--panel-2); color: var(--text); }
  .nav-item.on { background: var(--accent-soft); color: var(--accent); }
  .nav-item span:first-of-type { flex: 1; }
  .badge { background: var(--accent); color: var(--accent-text); border-radius: 99px; font-size: 11.5px; min-width: 20px; height: 20px; padding: 0 6px; display: grid; place-items: center; font-weight: 700; }
  .side-foot { margin-top: auto; display: flex; align-items: center; justify-content: space-between; padding: 0 8px; }
  main { min-width: 0; height: 100%; overflow: hidden; }
  .page { height: 100%; overflow-y: auto; }
  .page[hidden] { display: none; }
  .loading { height: 100%; display: grid; place-items: center; }
  .drop { position: fixed; inset: 0; z-index: 50; display: grid; place-items: center; background: color-mix(in srgb, var(--bg), transparent 15%); backdrop-filter: blur(3px); border: 3px dashed var(--accent); color: var(--accent); text-align: center; pointer-events: none; }
  .drop div { display: flex; flex-direction: column; align-items: center; gap: 10px; }
  @media (max-width: 900px) {
    .shell { grid-template-columns: 64px 1fr; }
    .nav-item span, .brand b, .side-foot { display: none; }
    .nav-item { justify-content: center; padding: 0; position: relative; }
    .badge { position: absolute; top: 2px; right: 6px; display: grid !important; }
  }
</style>
