<script lang="ts">
  import Icon from '../components/Icon.svelte';
  import JobCard from '../components/JobCard.svelte';
  import { api } from '../api';
  import { isActive, isFinished } from '../format';
  import { app, toastError } from '../state.svelte';
  import type { Job } from '../types';

  const waiting = $derived(app.jobs.filter(j => j.status === 'waiting'));
  const running = $derived(app.jobs.filter(j => isActive(j) && j.status !== 'waiting'));
  const queued = $derived(app.jobs.filter(j => j.status === 'queued'));
  const finished = $derived(app.jobs.filter(isFinished).sort((a, b) => (b.finishedAt ?? b.createdAt) - (a.finishedAt ?? a.createdAt)));
  const empty = $derived(app.jobs.length === 0);
  const groups = $derived<[string, Job[]][]>([
    ['Running', running], ['Waiting for scheduled start', waiting], ['Up next', queued], ['Finished', finished],
  ]);
</script>

<div class="wrap">
  <header class="row">
    <div class="grow"><h1>Queue</h1>
      <p class="muted">{running.length} running · {waiting.length} waiting for a start · {queued.length} queued · {finished.length} finished</p></div>
    {#if finished.length}<button class="btn" onclick={() => api.clearFinished().catch(toastError)}><Icon name="trash" size={16} /> Clear finished</button>{/if}
  </header>

  {#if empty}
    <div class="empty card">
      <Icon name="queue" size={34} /><h2>Nothing here yet</h2>
      <p class="muted">Downloads you add will show up here with live progress.</p>
      <button class="btn primary" onclick={() => (app.page = 'download')}><Icon name="download" size={16} /> Add a download</button>
    </div>
  {/if}

  {#each groups as [title, list] (title)}
    {#if list.length}
      <section class="col">
        <h3>{title}</h3>
        {#each list as j (j.id)}<JobCard job={j} />{/each}
      </section>
    {/if}
  {/each}
</div>

<style>
  .wrap { max-width: 980px; margin: 0 auto; padding: 26px 28px 40px; display: flex; flex-direction: column; gap: 20px; }
  .empty { display: flex; flex-direction: column; align-items: center; gap: 10px; padding: 48px 20px; text-align: center; color: var(--muted); }
  .empty h2 { color: var(--text); }
</style>
