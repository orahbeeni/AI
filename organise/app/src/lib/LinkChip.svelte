<script lang="ts">
  import { app, shortPath } from "./state.svelte";
  import { linkIcon, linkMenu } from "./linkActions";
  import Icon from "./Icon.svelte";

  let { id, nodeId }: { id: string; nodeId: string } = $props();
  const l = $derived(app.links[id]);
  const title = $derived(
    !l ? "" : l.kind === "url" ? l.url : l.status === "ok" ? shortPath(l.path, app.home) : `${l.name} can't be found at ${shortPath(l.path, app.home)}`,
  );

  function click(e: MouseEvent) {
    e.stopPropagation();
    if (!l) return;
    if (l.status === "ok") app.openLink(id);
    else app.openMenu(e, linkMenu(l, nodeId));
  }
</script>

{#if l}
  <button
    class="chip"
    class:missing={l.status === "missing"}
    class:searching={l.status === "searching"}
    {title}
    onclick={click}
    oncontextmenu={(e) => app.openMenu(e, linkMenu(l, nodeId))}
    onpointerdown={(e) => e.stopPropagation()}
  >
    {#if l.status === "searching"}
      <span class="spinner"></span>
    {:else if l.status === "missing"}
      <Icon name="warning" size={13} />
    {:else}
      <Icon name={linkIcon(l)} size={13} />
    {/if}
    <span class="name">{l.name}</span>
  </button>
{/if}

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    max-width: 260px;
    padding: 0 9px;
    border-radius: 6px;
    background: var(--chip);
    color: var(--text);
    font-size: 12.5px;
    flex: none;
  }
  .chip :global(.icon) {
    color: var(--text2);
  }
  .chip:hover {
    background: var(--chip-hover);
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .missing {
    background: var(--amber-bg);
    color: var(--amber);
  }
  .missing :global(.icon) {
    color: var(--amber);
  }
  .searching {
    color: var(--text2);
  }
</style>
