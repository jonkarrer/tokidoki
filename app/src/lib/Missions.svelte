<script lang="ts">
  import type { Checkpoint, Worker } from "./core";
  import { render } from "./markdown";
  import Presence from "./Presence.svelte";
  import { app, cancelWorker, showThread, trace } from "./state.svelte";

  // Missions: jobs Tokidoki sent a worker to go and do, the newest first. A mission runs on
  // a thread of its own, as durable as the conversation and with no memory; opened, it shows
  // what came of it and every step the worker took, read from that thread (state.svelte.ts,
  // `trace`) and followed live while it is still out.
  let reading = $state<string | null>(null);

  const missions = $derived(app.workers.filter((worker) => worker.kind === "mission"));
  const open = $derived(missions.find((worker) => worker.id === reading) ?? null);

  $effect(() => {
    if (reading) trace(reading);
  });

  /** One thing the worker did: something it said, or a tool it used and what came back. */
  type Step =
    | { kind: "said"; seq: number; at: number; text: string }
    | { kind: "tool"; seq: number; at: number; name: string; input: unknown; result?: string; failed: boolean; took?: number };

  const steps = $derived.by(() => {
    const steps: Step[] = [];
    const calls = new Map<string, Extract<Step, { kind: "tool" }>>();
    for (const { seq, at, change } of (reading && app.traces[reading]) || []) {
      if (change.kind !== "message") continue;
      for (const part of change.content) {
        if (part.type === "text" && change.role === "assistant" && part.text.trim()) {
          steps.push({ kind: "said", seq, at, text: part.text });
        } else if (part.type === "tool_call") {
          const step = { kind: "tool" as const, seq, at, name: part.name, input: part.input, failed: false };
          calls.set(part.id, step);
          steps.push(step);
        } else if (part.type === "tool_result") {
          const call = calls.get(part.call_id);
          if (call) Object.assign(call, { result: part.content, failed: part.is_error, took: at - call.at });
        }
      }
    }
    // The last thing it said is its report, shown above as what came of the mission.
    if (open?.status === "done" && steps.at(-1)?.kind === "said") steps.pop();
    return steps;
  });

  function standing(worker: Worker) {
    switch (worker.status) {
      case "running":
        return "Out on it";
      case "done":
        return "Came back";
      case "failed":
        return "Couldn't be finished";
      case "cancelled":
        return "Stopped";
    }
  }

  /** A tool's name as words: `search_notes` is "search notes". */
  const words = (name: string) => name.replaceAll("_", " ");

  /** What a tool was given, on one line: `query: groceries`. */
  function given(input: unknown) {
    if (!input || typeof input !== "object") return "";
    return Object.entries(input)
      .map(([key, value]) => `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`)
      .join(" · ");
  }

  function when(ms: number) {
    const days = Math.floor((Date.now() - ms) / 86_400_000);
    if (days < 1) return new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    if (days < 2) return "yesterday";
    if (days < 7) return `${days} days ago`;
    return new Date(ms).toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" });
  }

  const clock = (ms: number) =>
    new Date(ms).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });

  function lasted(ms: number) {
    if (ms < 1000) return `${ms} ms`;
    if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
    return `${Math.floor(ms / 60_000)} min ${Math.round((ms % 60_000) / 1000)} s`;
  }

  function from(worker: Worker) {
    return app.threads.find((thread) => thread.id === worker.parent);
  }

  /** What the worker is doing this moment, from its thread's live activity. */
  function now(worker: Worker) {
    const doing = app.live[worker.id]?.doing;
    if (doing?.kind === "tool") return `Using ${words(doing.name)}`;
    return "Working out the next step";
  }

  function first(checkpoints: Checkpoint[] | undefined) {
    return checkpoints?.[0]?.at;
  }
</script>

<section>
  <header data-tauri-drag-region></header>
  <div class="scroll">
    <div class="column">
      {#if open}
        {@const thread = from(open)}
        <article>
          <button class="back" onclick={() => (reading = null)}>← All missions</button>
          <h1 class="selectable">{open.title || open.task}</h1>
          <p class="meta" class:failed={open.status === "failed"}>
            {standing(open)} · {when(open.started_at)}{#if open.ended_at}
              · took {lasted(open.ended_at - open.started_at)}{/if}{#if thread}, asked in
              <button class="link" onclick={() => showThread(thread.id)}>{thread.title}</button>{/if}
          </p>

          <h2>The job</h2>
          <p class="job selectable">{open.task}</p>

          {#if open.status === "running"}
            <p class="out">
              <Presence doing={app.live[open.id]?.doing ?? { kind: "thinking" }} size={18} />
              <span>{now(open)}. Its report comes back to the conversation by itself.</span>
              <button onclick={() => cancelWorker(open.id)}>Stop</button>
            </p>
          {:else if open.report}
            <h2>What came of it</h2>
            <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
            <div class="reply selectable" class:failed={open.status !== "done"}>{@html render(open.report)}</div>
          {/if}

          <h2>What it did</h2>
          {#if steps.length === 0}
            <p class="none">
              {open.status === "running" ? "Nothing yet." : "It took no steps: it answered from the job alone."}
            </p>
          {:else}
            <ol class="trace">
              {#each steps as step (`${step.seq}:${step.kind}`)}
                <li class={step.kind} class:failed={step.kind === "tool" && step.failed}>
                  <time>{clock(step.at)}</time>
                  {#if step.kind === "said"}
                    <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
                    <div class="words selectable">{@html render(step.text)}</div>
                  {:else}
                    <details>
                      <summary>
                        <span class="tool">{words(step.name)}</span>
                        <span class="given">{given(step.input)}</span>
                        <span class="came">
                          {#if step.result === undefined}{open.status === "running" ? "under way" : "cut off"}
                          {:else if step.failed}failed{:else if step.took !== undefined}{lasted(step.took)}{/if}
                        </span>
                      </summary>
                      {#if step.result !== undefined}
                        <pre class="selectable">{step.result}</pre>
                      {/if}
                    </details>
                  {/if}
                </li>
              {/each}
            </ol>
          {/if}
          <p class="foot selectable">
            {open.id}{#if first(app.traces[open.id])}, started {clock(first(app.traces[open.id])!)}{/if}
          </p>
        </article>
      {:else}
        <h1>Missions</h1>
        <p class="lede">
          Jobs Tokidoki went and did for you. Ask for something to be done, not talked through, and it sends a worker
          on a mission; every step the worker takes is kept here.
        </p>
        {#if missions.length === 0}
          <p class="empty">None yet.</p>
        {:else}
          <ul>
            {#each missions as worker (worker.id)}
              <li>
                <button onclick={() => (reading = worker.id)}>
                  <span class="name">
                    {#if worker.status === "running"}<Presence doing={{ kind: "tool", name: worker.kind }} size={12} />{/if}
                    {worker.title || worker.task}
                  </span>
                  <span class="meta" class:failed={worker.status === "failed"}
                    >{[standing(worker), when(worker.started_at)].join(" · ")}</span
                  >
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
  h2 {
    margin: 26px 0 8px;
    font-size: 13px;
    font-weight: 600;
    color: var(--muted);
  }
  .lede {
    margin: 0 0 22px;
    max-width: 54ch;
    color: var(--muted);
    font-size: 15px;
  }
  ul {
    margin: 14px 0 0;
    padding: 0;
    list-style: none;
  }
  ul li {
    border-bottom: 1px solid var(--line);
  }
  ul li button {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 13px 0 11px;
    text-align: left;
  }
  ul li button:hover .name {
    color: var(--indigo);
  }
  .name {
    display: flex;
    align-items: center;
    gap: 8px;
    font: 16.5px/1.4 var(--voice);
    overflow-wrap: anywhere;
  }
  .meta {
    margin: 0;
    color: var(--muted);
    font-size: 12.5px;
  }
  .meta.failed,
  .reply.failed {
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
  .job {
    margin: 0;
    max-width: 62ch;
    white-space: pre-wrap;
  }
  .out {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 22px 0 0;
    color: var(--muted);
    font-size: 13.5px;
  }
  .out span {
    flex: 1;
  }
  .out button {
    font-size: 12px;
    font-weight: 600;
  }
  .out button:hover {
    color: var(--danger);
  }
  /* The trace: one row per step, the time it was taken down the left. */
  .trace {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  .trace li {
    display: grid;
    grid-template-columns: 78px minmax(0, 1fr);
    gap: 0 12px;
    padding: 9px 0;
    border-bottom: 1px solid var(--line);
  }
  time {
    padding-top: 1px;
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .words {
    color: var(--muted);
    font-size: 13.5px;
  }
  .words :global(p) {
    margin: 0 0 6px;
  }
  .words :global(p:last-child) {
    margin-bottom: 0;
  }
  summary {
    display: flex;
    align-items: baseline;
    gap: 8px;
    cursor: default;
    font-size: 13.5px;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary:hover .tool {
    color: var(--indigo);
  }
  .tool {
    flex: none;
    font-weight: 600;
  }
  .given {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
    font-family: var(--code);
    font-size: 12px;
  }
  .came {
    flex: none;
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .failed .came,
  .failed .tool {
    color: var(--danger);
  }
  pre {
    margin: 8px 0 2px;
    padding: 10px 12px;
    max-height: 320px;
    overflow: auto;
    border-radius: 8px;
    background: var(--raised);
    font: 12px/1.5 var(--code);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .none,
  .empty {
    margin: 0;
    color: var(--muted);
  }
  .empty {
    margin-top: 28px;
  }
  .foot {
    margin: 16px 0 0;
    color: var(--muted);
    font-size: 12px;
  }
</style>
