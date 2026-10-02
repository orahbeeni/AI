<script lang="ts">
  import Icon from './Icon.svelte';
  import { app, dismissToast } from '../state.svelte';
</script>

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast card {t.kind}">
      <span class="ico"><Icon name={t.kind === 'error' ? 'alert' : t.kind === 'ok' ? 'check' : 'clipboard'} size={17} /></span>
      <div class="grow">
        <div><b>{t.text}</b></div>
        {#if t.hint}<div class="muted small">{t.hint}</div>{/if}
        {#if t.action}
          <button class="btn sm primary act" onclick={() => { t.action?.run(); dismissToast(t.id); }}>{t.action.label}</button>
        {/if}
      </div>
      <button class="btn icon ghost" aria-label="Dismiss" onclick={() => dismissToast(t.id)}><Icon name="x" size={15} /></button>
    </div>
  {/each}
</div>

<style>
  .toasts { position: fixed; right: 18px; bottom: 18px; display: flex; flex-direction: column; gap: 10px; z-index: 60; width: min(380px, calc(100vw - 36px)); }
  .toast { display: flex; gap: 10px; align-items: flex-start; padding: 12px 10px 12px 14px; animation: in .18s ease-out; }
  .toast.error .ico { color: var(--err); } .toast.ok .ico { color: var(--ok); } .toast.info .ico { color: var(--accent); }
  .ico { margin-top: 1px; }
  .act { margin-top: 8px; }
  @keyframes in { from { opacity: 0; transform: translateY(8px); } }
</style>
