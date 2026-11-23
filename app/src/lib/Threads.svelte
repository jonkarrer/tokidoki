<script lang="ts">
  import { core, type Checkpoint, type ThreadPhase } from "./core";
  import Lines from "./Lines.svelte";
  import { app, items, show } from "./state.svelte";

  // Under the hood: the threads the conversation is filed into, for looking into how it was
  // filed. Each with how warm it is, what Tokidoki thinks it's about, where it left off when
  // it settled, and its own history, as stored. Read-only.
  let log = $state<Checkpoint[]>([]);

  const open = $derived(app.threads.find((thread) => thread.id === app.thread));
  const routes = $derived(
    log.flatMap(({ seq, change }) => (change.kind === "routed" ? [{ seq, by: change.by, reason: change.reason }] : [])),
  );

  $effect(() => {
    const thread = app.thread;
    log = [];
    if (thread) core.history(thread, 0).then((read) => app.thread === thread && (log = read));
  });

  const phases: Record<ThreadPhase, string> = {
    hot: "Hot: being talked in",
    cool: "Cool: idle, and dreamed",
    settled: "Settled: reset to where it left off",
  };

  function when(ms: number) {
    return new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit" });
  }
</script>

<section>
  <header data-tauri-drag-region></header>
  <div class="scroll">
    <div class="column">
      {#if open}
        <button class="back" onclick={() => (app.thread = null)}>← All threads</button>
        <h1 class="selectable">{open.title}</h1>
        <p class="meta">
          <span class="phase {open.phase}"></span>{phases[open.phase]} · {open.turns}
          {open.turns === 1 ? "turn" : "turns"} · last {when(open.active_at)} · <span class="selectable">{open.id}</span>
        </p>
        {#if open.about}<p class="card selectable"><b>About</b> {open.about}</p>{/if}
        {#if open.resume}<p class="card selectable"><b>Where it left off</b> {open.resume}</p>{/if}
        {#if routes.length}
          <ul class="routes">
            {#each routes as route (route.seq)}<li class="selectable">Filed here by {route.by}: {route.reason}</li>{/each}
          </ul>
        {/if}
        <div class="history"><Lines lines={items(log, app.memories)} /></div>
      {:else}
        <button class="back" onclick={() => show("conversation")}>← The conversation</button>
        <h1>Threads</h1>
        <p class="lede">
          The conversation, as Tokidoki files it: one thread per subject. A thread is hot while you talk in it, cools
          when it goes quiet, and settles into a short note of where it left off.
        </p>
        {#if app.threads.length === 0}
          <p class="empty">None yet.</p>
        {:else}
          <ul class="list">
            {#each app.threads as thread (thread.id)}
              <li>
                <button onclick={() => (app.thread = thread.id)}>
                  <span class="phase {thread.phase}" title={phases[thread.phase]}></span>
                  <span class="name">{thread.title}</span>
                  <span class="facts">{thread.turns} · {when(thread.active_at)}</span>
                  {#if thread.about}<span class="about">{thread.about}</span>{/if}
                </button>
              </li>
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
  .back {
    margin-top: 18px;
    color: var(--muted);
    font-size: 13px;
  }
  .back:hover {
    color: var(--ink);
  }
  h1 {
    margin: 14px 0 8px;
    font: 500 30px/1.15 var(--voice);
    letter-spacing: -0.02em;
    overflow-wrap: anywhere;
  }
  .lede {
    margin: 0 0 22px;
    max-width: 54ch;
    color: var(--muted);
    font-size: 15px;
  }
  .meta {
    margin: 0 0 16px;
    color: var(--muted);
    font-size: 12.5px;
  }
  /* Warmth: filled while hot, an outline once cool, faint once settled. */
  .phase {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 7px;
    border-radius: 50%;
    border: 1.5px solid var(--indigo);
    vertical-align: 0;
  }
  .phase.hot {
    background: var(--indigo);
  }
  .phase.settled {
    border-color: var(--line);
  }
  .card {
    margin: 0 0 10px;
    padding: 9px 12px;
    border-radius: 9px;
    background: var(--raised);
    font: 14px/1.5 var(--voice);
  }
  .card b {
    display: block;
    font: 600 11.5px var(--ui);
    color: var(--muted);
  }
  .routes {
    margin: 0 0 8px;
    padding: 0;
    list-style: none;
    color: var(--muted);
    font-size: 12px;
  }
  .history {
    margin-top: 18px;
    padding-top: 6px;
    border-top: 1px solid var(--line);
  }
  .list {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .list li {
    border-bottom: 1px solid var(--line);
  }
  .list button {
    width: 100%;
    display: grid;
    grid-template-columns: 15px 1fr auto;
    align-items: baseline;
    gap: 2px 4px;
    padding: 11px 0;
    text-align: left;
  }
  .list button:hover .name {
    color: var(--indigo);
  }
  .name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .facts {
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .about {
    grid-column: 2 / 4;
    color: var(--muted);
    font-size: 13px;
  }
  .empty {
    color: var(--muted);
  }
</style>
