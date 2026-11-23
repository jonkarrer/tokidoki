<script lang="ts">
  import type { Worker } from "./core";
  import { render } from "./markdown";
  import Presence from "./Presence.svelte";
  import { app, cancelWorker, showThread } from "./state.svelte";

  // What Tokidoki looked into: every piece of research it handed to a worker, the newest
  // first, with the report each brought back (its sources as links). Running ones can be
  // stopped. The list follows the core's worker events (state.svelte.ts), so research started
  // in a conversation shows up here as it happens.
  let query = $state("");
  let reading = $state<string | null>(null);

  const research = $derived(app.workers.filter((worker) => worker.kind === "research"));
  const shown = $derived.by(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return research.filter((worker) => {
      const text = `${worker.title}\n${worker.task}\n${worker.report ?? ""}`.toLowerCase();
      return words.every((word) => text.includes(word));
    });
  });
  const open = $derived(research.find((worker) => worker.id === reading) ?? null);

  function standing(worker: Worker) {
    switch (worker.status) {
      case "running":
        return "Looking into it";
      case "done":
        return worker.delivery === "held" ? "Kept for when you come back to it" : "Came back";
      case "failed":
        return "Couldn't be done";
      case "cancelled":
        return "Stopped";
    }
  }

  /** The report's first lines, as plain text, for the list. */
  function excerpt(worker: Worker) {
    if (!worker.report || worker.status !== "done") return "";
    return worker.report
      .split("\n")
      .map((line) =>
        line
          .replace(/^\s*(?:[#>*+-]+|\d+[.)])\s*/, "")
          .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
          .replace(/[*_`~]/g, "")
          .trim(),
      )
      .filter((line) => line && !/^(answer|summary):?$/i.test(line))
      .join(" ");
  }

  function when(ms: number) {
    const days = Math.floor((Date.now() - ms) / 86_400_000);
    if (days < 1) return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    if (days < 2) return "yesterday";
    if (days < 7) return `${days} days ago`;
    return new Date(ms).toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" });
  }

  function from(worker: Worker) {
    return app.threads.find((thread) => thread.id === worker.parent);
  }
</script>

<section>
  <header data-tauri-drag-region></header>
  <div class="scroll">
    <div class="column">
      {#if open}
        {@const thread = from(open)}
        <article>
          <button class="back" onclick={() => (reading = null)}>← All research</button>
          <h1 class="selectable">{open.title || open.task}</h1>
          <p class="meta">
            {standing(open)} · {when(open.started_at)}{#if thread}, asked in
              <button class="link" onclick={() => showThread(thread.id)}>{thread.title}</button>{/if}
          </p>
          <details class="brief">
            <summary>What the worker was asked</summary>
            <p class="selectable">{open.task}</p>
          </details>
          {#if open.status === "running"}
            <p class="looking">
              <Presence doing={{ kind: "tool", name: open.kind }} size={18} />
              <span>Looking into it. The report comes back to the conversation by itself.</span>
              <button onclick={() => cancelWorker(open.id)}>Stop</button>
            </p>
          {:else if open.report}
            <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
            <div class="reply selectable" class:failed={open.status !== "done"}>{@html render(open.report)}</div>
          {/if}
        </article>
      {:else}
        <h1>Research</h1>
        <p class="lede">
          What Tokidoki looked into for you. Ask it to look something up in a conversation and it hands the work to a
          worker that searches the web; its report lands here, with its sources.
        </p>
        {#if research.length > 4}
          <input class="search" type="search" bind:value={query} placeholder="Search research" aria-label="Search research" />
        {/if}
        {#if research.length === 0}
          <p class="empty">Nothing yet. Research needs an Exa key, under Model and key.</p>
        {:else}
          <ul>
            {#each shown as worker (worker.id)}
              <li>
                <button onclick={() => (reading = worker.id)}>
                  <span class="name">
                    {#if worker.status === "running"}<Presence doing={{ kind: "tool", name: worker.kind }} size={12} />{/if}
                    {worker.title || worker.task}
                  </span>
                  {#if excerpt(worker)}<span class="excerpt">{excerpt(worker)}</span>{/if}
                  <span class="meta" class:failed={worker.status === "failed"}
                    >{[standing(worker), when(worker.started_at)].join(" · ")}</span
                  >
                </button>
              </li>
            {:else}
              <li class="empty">No research matches “{query.trim()}”.</li>
            {/each}
          </ul>
        {/if}
      {/if}
    </div>
  </div>
</section>

<style>
  section {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  header {
    height: 38px;
    flex: none;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .column {
    width: min(680px, 100%);
    margin: 0 auto;
    padding: 8px 28px 40px;
  }
  h1 {
    margin: 4vh 0 8px;
    font: 500 30px/1.15 var(--voice);
    letter-spacing: -0.02em;
    overflow-wrap: anywhere;
  }
  article h1 {
    margin-top: 14px;
    font-size: 24px;
    line-height: 1.25;
  }
  .lede {
    margin: 0 0 22px;
    max-width: 54ch;
    color: var(--muted);
    font-size: 15px;
  }
  .search {
    width: 100%;
    height: 36px;
    padding: 0 12px;
    border: 1px solid var(--line);
    border-radius: 9px;
    background: var(--raised);
    font-size: 14.5px;
  }
  .search:focus {
    outline: 2px solid var(--indigo);
    outline-offset: -1px;
  }
  ul {
    margin: 14px 0 0;
    padding: 0;
    list-style: none;
  }
  li:not(.empty) {
    border-bottom: 1px solid var(--line);
  }
  li button {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 13px 0 11px;
    text-align: left;
  }
  li button:hover .name {
    color: var(--indigo);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 16.5px/1.4 var(--voice);
    overflow-wrap: anywhere;
  }
  .excerpt {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--muted);
    font-size: 13.5px;
  }
  .meta {
    margin: 0;
    color: var(--muted);
    font-size: 12.5px;
  }
  .meta.failed {
    color: var(--danger);
  }
  .link {
    color: var(--indigo);
  }
  .link:hover {
    text-decoration: underline;
  }
  .back {
    margin-top: 18px;
    color: var(--muted);
    font-size: 13px;
  }
  .back:hover {
    color: var(--ink);
  }
  .brief {
    margin-top: 10px;
    color: var(--muted);
    font-size: 13px;
  }
  .brief summary {
    cursor: default;
  }
  .brief summary:hover {
    color: var(--ink);
  }
  .brief p {
    margin: 6px 0 0;
    max-width: 60ch;
  }
  article .reply {
    margin-top: 22px;
  }
  .reply.failed {
    color: var(--danger);
  }
  .looking {
    display: flex;
    align-items: center;
    gap: 9px;
    margin-top: 22px;
    color: var(--muted);
    font-size: 13.5px;
  }
  .looking span {
    flex: 1;
  }
  .looking button {
    font-size: 12px;
    font-weight: 600;
  }
  .looking button:hover {
    color: var(--danger);
  }
  .empty {
    margin: 28px 0 0;
    color: var(--muted);
  }
</style>
