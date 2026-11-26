<script lang="ts">
  // Context menu, prompt and confirm dialogs, toast, and the drag ghost.
  import { app } from "./state.svelte";
  import { dnd } from "./dnd.svelte";
  import Icon from "./Icon.svelte";
  import type { MenuItem } from "./types";

  let menuEl: HTMLDivElement | undefined = $state();
  let stack = $state<MenuItem[][]>([]);
  let pos = $state({ x: 0, y: 0 });

  $effect(() => {
    if (app.menu) {
      stack = [app.menu.items];
      pos = { x: app.menu.x, y: app.menu.y };
    }
  });

  // keep the menu on screen
  $effect(() => {
    if (!menuEl || !app.menu) return;
    void stack.length;
    const r = menuEl.getBoundingClientRect();
    const x = Math.min(pos.x, window.innerWidth - r.width - 8);
    const y = pos.y + r.height > window.innerHeight - 8 ? Math.max(8, pos.y - r.height) : pos.y;
    if (x !== pos.x || y !== pos.y) pos = { x: Math.max(8, x), y };
  });

  const items = $derived(stack[stack.length - 1] ?? []);

  function pick(it: MenuItem) {
    if (it.disabled || it.separator) return;
    if (it.submenu) {
      stack = [...stack, it.submenu];
      return;
    }
    app.menu = null;
    it.run?.();
  }

  let promptValue = $state("");
  let promptEl: HTMLInputElement | undefined = $state();
  $effect(() => {
    if (app.prompt) {
      promptValue = app.prompt.value;
      queueMicrotask(() => {
        promptEl?.focus();
        promptEl?.select();
      });
    }
  });

  function promptOk() {
    const p = app.prompt;
    app.prompt = null;
    if (p) p.ok(promptValue);
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (app.menu) {
        if (stack.length > 1) stack = stack.slice(0, -1);
        else app.menu = null;
        e.stopPropagation();
      } else if (app.prompt) app.prompt = null;
      else if (app.confirm) app.confirm = null;
    }
  }
</script>

<svelte:window onkeydown={keydown} />

{#if app.menu}
  <div class="scrim clear" role="presentation" onpointerdown={() => (app.menu = null)} oncontextmenu={(e) => { e.preventDefault(); app.menu = null; }}></div>
  <div class="menu" bind:this={menuEl} style:left="{pos.x}px" style:top="{pos.y}px" role="menu">
    {#if stack.length > 1}
      <button class="mi back" onclick={() => (stack = stack.slice(0, -1))}><Icon name="chevron-left" size={15} /> Back</button>
      <div class="sep"></div>
    {/if}
    {#each items as it, i (i)}
      {#if it.separator}
        <div class="sep"></div>
      {:else}
        <button class="mi" class:danger={it.danger} class:disabled={it.disabled} role="menuitem" onclick={() => pick(it)} title={it.hint ?? ""}>
          {#if it.color}
            <span class="swatch" style:background={it.color}></span>
          {:else if it.icon}
            <Icon name={it.icon} size={15} />
          {:else}
            <span class="swatch none"></span>
          {/if}
          <span class="lbl">
            {it.label}
            {#if it.hint && it.disabled}<span class="hint">{it.hint}</span>{/if}
          </span>
          {#if it.checked}<Icon name="check" size={14} />{/if}
          {#if it.shortcut}<span class="sc">{it.shortcut}</span>{/if}
          {#if it.submenu}<Icon name="chevron-right" size={14} />{/if}
        </button>
      {/if}
    {/each}
  </div>
{/if}

{#if app.prompt}
  <div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (app.prompt = null)}>
    <form class="dialog" onsubmit={(e) => { e.preventDefault(); promptOk(); }}>
      <h3>{app.prompt.title}</h3>
      <input bind:this={promptEl} bind:value={promptValue} placeholder={app.prompt.placeholder} spellcheck="false" />
      <div class="buttons">
        <button type="button" class="btn ghost" onclick={() => (app.prompt = null)}>Cancel</button>
        <button type="submit" class="btn primary">{app.prompt.okLabel ?? "OK"}</button>
      </div>
    </form>
  </div>
{/if}

{#if app.confirm}
  <div class="scrim" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (app.confirm = null)}>
    <div class="dialog" role="dialog">
      <h3>{app.confirm.title}</h3>
      {#if app.confirm.body}<p class="muted">{app.confirm.body}</p>{/if}
      <div class="buttons">
        <button class="btn ghost" onclick={() => (app.confirm = null)}>Cancel</button>
        <button
          class="btn {app.confirm.danger ? 'danger' : 'primary'}"
          onclick={() => {
            const c = app.confirm;
            app.confirm = null;
            c?.ok();
          }}>{app.confirm.okLabel}</button
        >
      </div>
    </div>
  </div>
{/if}

{#if app.toast}
  <div class="toast" class:error={app.toast.kind === "error"} role="status">
    <span>{app.toast.text}</span>
    {#if app.toast.action}
      <button
        onclick={() => {
          const a = app.toast?.action;
          app.toast = null;
          a?.run();
        }}>{app.toast.action.label}</button
      >
    {/if}
  </div>
{/if}

{#if dnd.s && !dnd.s.native}
  <div class="ghost" style:left="{dnd.s.x + 14}px" style:top="{dnd.s.y + 10}px">{dnd.s.label}</div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: rgba(15, 23, 42, 0.25);
    display: grid;
    place-items: start center;
    padding-top: 18vh;
  }
  .scrim.clear {
    background: none;
  }
  .menu {
    position: fixed;
    z-index: 51;
    min-width: 210px;
    max-width: 320px;
    max-height: 70vh;
    overflow-y: auto;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 10px;
    box-shadow: var(--shadow);
    padding: 5px;
    display: flex;
    flex-direction: column;
  }
  .mi {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 7px 10px;
    border-radius: 6px;
    text-align: left;
    font-size: 13.5px;
    color: var(--text);
  }
  .mi :global(.icon) {
    color: var(--text2);
  }
  .mi:hover {
    background: var(--hover);
  }
  .mi.danger,
  .mi.danger :global(.icon) {
    color: var(--red);
  }
  .mi.disabled {
    opacity: 0.45;
    cursor: default;
  }
  .mi.disabled:hover {
    background: none;
  }
  .mi.back {
    color: var(--text2);
    font-size: 12.5px;
  }
  .lbl {
    flex: 1;
    display: flex;
    flex-direction: column;
  }
  .hint {
    font-size: 11.5px;
    color: var(--text2);
  }
  .sc {
    font-size: 11.5px;
    color: var(--text3);
  }
  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    margin: 0 1.5px;
    flex: none;
  }
  .swatch.none {
    background: none;
  }
  .sep {
    flex: none;
    height: 1px;
    background: var(--line);
    margin: 4px 6px;
  }
  .dialog {
    width: 400px;
    background: var(--bg);
    border-radius: 14px;
    box-shadow: var(--shadow);
    padding: 20px 20px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .dialog h3 {
    margin: 0;
    font-size: 16px;
  }
  .dialog p {
    margin: 0;
  }
  .dialog input {
    height: 38px;
    border-radius: 8px;
    border: 1px solid var(--line-strong);
    background: var(--bg);
    padding: 0 12px;
    outline: none;
  }
  .dialog input:focus {
    border-color: var(--accent);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .toast {
    position: fixed;
    left: 50%;
    bottom: 24px;
    transform: translateX(-50%);
    z-index: 60;
    display: flex;
    align-items: center;
    gap: 18px;
    padding: 11px 16px;
    border-radius: 10px;
    background: #1f2328;
    color: #fff;
    box-shadow: var(--shadow);
    font-size: 13.5px;
    max-width: 70vw;
  }
  .toast.error {
    background: #7f1d1d;
  }
  .toast button {
    color: #8fb3ff;
    font-weight: 600;
  }
  .ghost {
    position: fixed;
    z-index: 70;
    pointer-events: none;
    background: var(--bg);
    border: 1px solid var(--line-strong);
    box-shadow: var(--shadow);
    border-radius: 8px;
    padding: 6px 12px;
    font-size: 13.5px;
    max-width: 320px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    opacity: 0.95;
  }
</style>
