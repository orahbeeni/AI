<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { app, ago, dirOf, shortPath } from "./state.svelte";
  import { chooseFileFor, linkIcon } from "./linkActions";
  import Icon from "./Icon.svelte";

  const missing = $derived(app.missingLinks());
  const total = $derived(app.linkTotal());

  function users(id: string) {
    return app.usersOf(id).map((u) => (u.node.id === u.list.id ? u.list.text : `${u.list.text} › ${u.node.text}`)).join(", ");
  }

  function searchAgain() {
    invoke("search_again");
    app.say("Checking every link…");
  }
</script>

<div class="view">
  <div class="content">
    <div class="head">
      <h1>Links</h1>
      <button class="btn" onclick={searchAgain}><Icon name="refresh" size={14} /> Check again</button>
    </div>
    <p class="muted lead">
      {total} linked {total === 1 ? "file, folder or page" : "files, folders and pages"}. When a file moves, its links follow it.
    </p>

    <div class="section-label">Needs attention</div>
    {#if !missing.length}
      <div class="ok"><Icon name="check" size={16} /> Every linked file is where its links say.</div>
    {/if}
    {#each missing as l (l.id)}
      <div class="card">
        <div class="row1">
          {#if l.status === "searching"}
            <span class="spinner big"></span>
          {:else}
            <span class="warn"><Icon name="warning" size={20} /></span>
          {/if}
          <div class="what">
            <strong>{l.status === "searching" ? `Looking for ${l.name}…` : `${l.name} can't be found`}</strong>
            <span class="muted small">
              Linked from {users(l.id)}{#if l.path} · last seen in {shortPath(dirOf(l.path), app.home)}{:else} · linked on another computer{/if}
            </span>
          </div>
        </div>
        {#if l.candidates?.length}
          <div class="cands">
            {#each l.candidates as c (c.path)}
              <div class="cand">
                <Icon name="search" size={15} />
                <div class="c-text">
                  <span>{shortPath(c.path, app.home)}</span>
                  <span class="faint small">{c.reason}</span>
                </div>
                <button class="btn primary" onclick={() => app.relink(l.id, c.path)}>Use this</button>
              </div>
            {/each}
          </div>
        {/if}
        {#if l.status !== "searching"}
          <div class="actions">
            <button class="btn" onclick={() => chooseFileFor(l)}><Icon name="folder" size={14} /> Choose its new location…</button>
            <button
              class="btn ghost"
              onclick={(e) =>
                app.openMenu(e, [
                  { label: "Look again", icon: "refresh", run: searchAgain },
                  { label: "Remove this link everywhere", icon: "trash", danger: true, run: () => app.removeLinkEverywhere(l.id) },
                ])}><Icon name="more" size={16} /></button
            >
          </div>
        {/if}
      </div>
    {/each}

    <div class="section-label gap">Recently moved</div>
    {#if !app.moves.length}
      <p class="faint small">Nothing has moved yet. Try it: move or rename a linked file in your file manager.</p>
    {/if}
    {#each app.moves.slice(0, 20) as m (m.at + m.linkId + m.to)}
      {@const l = app.links[m.linkId]}
      <div class="move">
        <Icon name={linkIcon(l)} size={17} />
        <div class="m-text">
          <span>{m.name}</span>
          <span class="muted small">
            {#if dirOf(m.from) === dirOf(m.to)}
              renamed from {m.from.split("/").pop()}
            {:else}
              {shortPath(dirOf(m.from), app.home)} → {shortPath(dirOf(m.to), app.home)}
            {/if}
            {#if m.how !== "watcher"}<span class="faint"> · {m.how === "you" ? "chosen by you" : m.how === "inode" ? "found by file identity" : "found by content"}</span>{/if}
          </span>
        </div>
        <span class="faint small when">{ago(m.at)}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  .view {
    height: 100%;
    overflow-y: auto;
  }
  .content {
    max-width: 820px;
    margin: 0 auto;
    padding: 40px 48px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h1 {
    font-size: 28px;
    margin: 0;
  }
  .lead {
    margin: 6px 0 28px;
  }
  .small {
    font-size: 12.5px;
  }
  .gap {
    margin-top: 32px;
  }
  .section-label {
    margin-bottom: 10px;
  }
  .ok {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--green);
    padding: 12px 14px;
    border-radius: 10px;
    background: var(--green-bg);
  }
  .card {
    border: 1px solid var(--line);
    border-radius: 12px;
    padding: 16px 18px;
    margin-bottom: 10px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .row1 {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .warn {
    color: var(--amber);
  }
  .big {
    width: 18px;
    height: 18px;
    color: var(--text3);
    margin-top: 2px;
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .cands {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-left: 32px;
  }
  .cand {
    display: flex;
    align-items: center;
    gap: 10px;
    color: var(--text2);
  }
  .c-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    color: var(--text);
  }
  .c-text span:first-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    gap: 8px;
    padding-left: 32px;
  }
  .move {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 2px;
    border-bottom: 1px solid var(--line);
    color: var(--text2);
  }
  .m-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    color: var(--text);
  }
  .m-text span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .when {
    flex: none;
  }
</style>
