<script lang="ts">
  import type { Checkpoint, Worker } from "./core";
  import { render } from "./markdown";
  import Presence from "./Presence.svelte";
  import { app, cancelWorker, showThread, trace } from "./state.svelte";

  // Missions: pieces of real work (sustained, many steps, something to show at the end)
  // Tokidoki sent a worker away to do, the newest first. A mission runs on
  // a thread of its own, as durable as the conversation and with no memory; opened, it shows
  // what came of it and every step the worker took, read from that thread (state.svelte.ts,
  // `trace`) and followed live while it is still out.
  let reading = $state<string | null>(null);

  const missions = $derived(app.workers.filter((worker) => worker.kind === "mission"));
  const open = $derived(missions.find((worker) => worker.id === reading) ?? null);

  $effect(() => {
    if (reading) trace(reading);
  });

  /** A tool the worker used, what it was given, and what came back. */
  type Use = { id: string; name: string; given: [string, string][]; result?: string; failed: boolean; took?: number };
  /** One turn of the worker's model: what it said, and the tools it used. */
  type Step = { seq: number; at: number; said: string; uses: Use[] };

  const loaded = $derived(reading !== null && app.traces[reading] !== undefined);
  const steps = $derived.by(() => {
    const steps: Step[] = [];
    const calls = new Map<string, { use: Use; at: number }>();
    for (const { seq, at, change } of (reading && app.traces[reading]) || []) {
      if (change.kind !== "message") continue;
      if (change.role === "assistant") {
        const step: Step = { seq, at, said: "", uses: [] };
        for (const part of change.content) {
          if (part.type === "text") step.said = [step.said, part.text.trim()].filter(Boolean).join("\n\n");
          if (part.type === "tool_call") {
            const use: Use = { id: part.id, name: part.name, given: given(part.input), failed: false };
            calls.set(part.id, { use, at });
            step.uses.push(use);
          }
        }
        if (step.said || step.uses.length) steps.push(step);
      }
      for (const part of change.content) {
        if (part.type !== "tool_result") continue;
        const call = calls.get(part.call_id);
        if (call) Object.assign(call.use, { result: part.content, failed: part.is_error, took: at - call.at });
      }
    }
    return steps;
  });
  // The last thing it said, with no tool after, is its report: shown as such, at the end.
  const reported = $derived(open?.status === "done" && steps.at(-1)?.uses.length === 0);
  const taken = $derived(reported ? steps.slice(0, -1) : steps);

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

  /** What a tool was given, argument by argument, each as text. */
  function given(input: unknown): [string, string][] {
    if (!input || typeof input !== "object") return [];
    return Object.entries(input).map(([key, value]) => [key, typeof value === "string" ? value : JSON.stringify(value)]);
  }

  /** A result's first lines, for telling at a glance what came back. */
  function glance(result: string) {
    const lines = result.split("\n").filter((line) => line.trim());
    return lines.slice(0, 2).join("\n");
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

          <details class="brief">
            <summary>The job, as the worker was given it</summary>
            <p class="job selectable">{open.task}</p>
          </details>

          <h2>Trace</h2>
          {#if app.traceErrors[open.id]}
            <p class="none failed selectable">Its steps couldn't be read: {app.traceErrors[open.id]}</p>
          {:else if !loaded}
            <p class="none">Reading what it did…</p>
          {:else if taken.length === 0 && open.status !== "running"}
            <p class="none selectable">
              No steps to show: {app.traces[open.id]?.length ?? 0} entries were read from {open.id}, none of them a
              tool being used.
            </p>
          {/if}
          <ol class="trace">
            {#each taken as step, index (step.seq)}
              <li>
                <p class="at"><b>Step {index + 1}</b><time>{clock(step.at)}</time></p>
                {#if step.said}
                  <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
                  <div class="words selectable">{@html render(step.said)}</div>
                {/if}
                {#each step.uses as use (use.id)}
                  <div class="use" class:failed={use.failed}>
                    <p class="tool">
                      <code class="selectable">{use.name}</code>
                      <span>
                        {#if use.result === undefined}{open.status === "running" ? "under way" : "cut off"}
                        {:else}{use.failed ? "refused or failed" : "done"}{#if use.took !== undefined}
                            · {lasted(use.took)}{/if}{/if}
                      </span>
                    </p>
                    {#if use.given.length}
                      <dl>
                        {#each use.given as [key, value] (key)}
                          <dt>{key}</dt>
                          <dd class="selectable">{value}</dd>
                        {/each}
                      </dl>
                    {/if}
                    {#if use.result !== undefined}
                      <details class="result">
                        <summary><span class="selectable">{glance(use.result) || "(nothing)"}</span></summary>
                        <pre class="selectable">{use.result}</pre>
                      </details>
                    {/if}
                  </div>
                {/each}
              </li>
            {/each}
            {#if open.status === "running"}
              <li class="now">
                <Presence doing={app.live[open.id]?.doing ?? { kind: "thinking" }} size={18} />
                <span>{now(open)}</span>
                <button onclick={() => cancelWorker(open.id)}>Stop</button>
              </li>
            {/if}
          </ol>

          {#if open.status !== "running" && open.report}
            <h2>{open.status === "done" ? "Report" : "Why it stopped"}</h2>
            <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
            <div class="reply selectable" class:failed={open.status !== "done"}>{@html render(open.report)}</div>
          {/if}
          <p class="foot selectable">{open.id}</p>
        </article>
      {:else}
        <h1>Missions</h1>
        <p class="lede">
          Real work Tokidoki went away to do for you: writing and publishing a post, going through your recent
          email, placing an order. It sends a worker on a mission, and every step the worker takes is kept here.
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
  .none.failed,
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
  .job {
    margin: 6px 0 0;
    max-width: 62ch;
    white-space: pre-wrap;
  }
  /* The trace: one block per turn of the worker's model, in order, each with what it said
     and every tool it used, its arguments in full. */
  .trace {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .trace > li {
    padding: 12px 0 14px;
    border-top: 1px solid var(--line);
  }
  .at {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin: 0 0 6px;
    font-size: 12.5px;
  }
  time {
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .words {
    margin-bottom: 8px;
    color: var(--muted);
    font-size: 13.5px;
  }
  .words :global(p) {
    margin: 0 0 6px;
  }
  .words :global(p:last-child) {
    margin-bottom: 0;
  }
  .use {
    margin-top: 6px;
    padding: 9px 12px 10px;
    border-radius: 8px;
    background: var(--raised);
    border: 1px solid var(--line);
  }
  .use.failed {
    border-color: var(--danger);
  }
  .tool {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 10px;
    margin: 0;
  }
  .tool code {
    font: 600 13px var(--code);
  }
  .tool span {
    color: var(--muted);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .failed .tool span {
    color: var(--danger);
  }
  dl {
    display: grid;
    grid-template-columns: max-content minmax(0, 1fr);
    gap: 3px 12px;
    margin: 7px 0 0;
    font: 12.5px/1.5 var(--code);
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .result {
    margin-top: 8px;
    padding-top: 7px;
    border-top: 1px solid var(--line);
  }
  .result summary {
    cursor: default;
    color: var(--muted);
    font: 12px/1.5 var(--code);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .result summary:hover {
    color: var(--ink);
  }
  .result[open] summary span {
    display: none;
  }
  .result[open] summary::after {
    content: "What came back";
  }
  pre {
    margin: 8px 0 0;
    max-height: 420px;
    overflow: auto;
    font: 12px/1.5 var(--code);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .trace > li.now {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--muted);
    font-size: 13.5px;
  }
  .now span {
    flex: 1;
  }
  .now button {
    font-size: 12px;
    font-weight: 600;
  }
  .now button:hover {
    color: var(--danger);
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
