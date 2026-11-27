<script lang="ts">
  import { core, type Usage } from "./core";
  import { app } from "./state.svelte";

  // How much AI Tokidoki used: every call it made to a model and to Jev, and every tool it
  // ran, as its journal recorded them. Counted, not guessed: tokens for what a model or Jev
  // was sent and wrote, calls for a tool. Shown by who provided it, then by what the work
  // was (the conversation, or a mission, a learning run, a piece of research), then by day.
  const ranges = [
    { name: "Today", days: 0 },
    { name: "7 days", days: 7 },
    { name: "30 days", days: 30 },
  ];
  let days = $state(7);
  let lines = $state<Usage[]>([]);
  let error = $state<string | null>(null);
  let loaded = $state(false);

  function since(days: number) {
    const start = new Date();
    start.setHours(0, 0, 0, 0);
    return start.getTime() - days * 86_400_000;
  }

  $effect(() => {
    const from = since(days);
    error = null;
    core
      .usage(from)
      .then((read) => (lines = read))
      .catch((problem) => (error = String(problem)))
      .finally(() => (loaded = true));
  });

  type Sum = { calls: number; input: number; output: number };
  const tokens = (sum: Sum) => sum.input + sum.output;

  /** The lines added up by `key`, in the order the keys first appear. */
  function grouped(from: Usage[], key: (line: Usage) => string) {
    const groups = new Map<string, Sum>();
    for (const line of from) {
      const k = key(line);
      const sum = groups.get(k) ?? { calls: 0, input: 0, output: 0 };
      sum.calls += line.calls;
      sum.input += line.input_tokens;
      sum.output += line.output_tokens;
      groups.set(k, sum);
    }
    return [...groups].map(([key, sum]) => ({ key, ...sum }));
  }

  /** A tool is counted in calls; a model and Jev in tokens too. */
  const tool = (service: string) => service === "exa" || service === "tools";
  const ai = $derived(lines.filter((line) => !tool(line.service)));
  const total = $derived(grouped(ai, () => "")[0] ?? { calls: 0, input: 0, output: 0 });
  const toolCalls = $derived(lines.filter((line) => tool(line.service)).reduce((n, line) => n + line.calls, 0));

  // One block per provider: the model providers by how much they were used, then Jev, then
  // what the tools did (Exa's, then the ones run on this Mac).
  const last = ["jev", "exa", "tools"];
  const providers = $derived(
    grouped(lines, (line) => line.service)
      .sort((a, b) => last.indexOf(a.key) - last.indexOf(b.key) || tokens(b) - tokens(a))
      .map((provider) => ({
        ...provider,
        rows: grouped(
          lines.filter((line) => line.service === provider.key),
          (line) => line.model,
        ).sort((a, b) => tokens(b) - tokens(a) || b.calls - a.calls),
      })),
  );

  const names: Record<string, string> = {
    anthropic: "Anthropic",
    fireworks: "Fireworks",
    baseten: "Baseten",
    jev: "Jev (TypeSafe)",
    exa: "Exa",
    tools: "Tools on this Mac",
  };
  const named = (service: string) => names[service.toLowerCase()] ?? service;
  const about: Record<string, string> = {
    jev: "The quick decisions: where a message belongs, what to recall and keep. Counted in tokens sent.",
    exa: "Searching the web and reading public pages, for research.",
    tools: "What the models asked Tokidoki to do here: notes, the browser, reading a page, handing work to a worker.",
  };

  /** What a thread's calls were for: a worker's job by its name, anything else the conversation. */
  function purpose(thread: string) {
    const worker = app.workers.find((worker) => worker.id === thread);
    if (!worker) return "The conversation, with memory and filing";
    const kind = { mission: "Mission", learning: "Learning", research: "Research" }[worker.kind] ?? worker.kind;
    return `${kind}: ${worker.title || worker.task}`;
  }
  const purposes = $derived(
    grouped(ai, (line) => (line.thread?.startsWith("worker-") ? line.thread : ""))
      .map((group) => ({
        ...group,
        name: purpose(group.key),
        tools: lines
          .filter((line) => tool(line.service) && (line.thread?.startsWith("worker-") ? line.thread : "") === group.key)
          .reduce((n, line) => n + line.calls, 0),
      }))
      .sort((a, b) => tokens(b) - tokens(a)),
  );
  const day = (at: number) => new Date(at).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
  const byDay = $derived(
    grouped(ai, (line) => day(line.at))
      .map((group) => ({
        ...group,
        tools: lines.filter((line) => tool(line.service) && day(line.at) === group.key).reduce((n, line) => n + line.calls, 0),
      }))
      .reverse(),
  );

  const count = (n: number) => n.toLocaleString();
</script>

<section>
  <header data-tauri-drag-region></header>
  <div class="scroll">
    <div class="column">
      <h1>Usage</h1>
      <p class="lede">
        How much AI Tokidoki used on your keys: every call to a model and every tool it ran, counted as it recorded
        them.
      </p>
      <div class="ranges">
        {#each ranges as range (range.days)}
          <button class:on={days === range.days} onclick={() => (days = range.days)}>{range.name}</button>
        {/each}
      </div>
      {#if error}<p class="error selectable" role="alert">{error}</p>{/if}

      {#if loaded && lines.length === 0}
        <p class="empty">Nothing recorded in this time.</p>
      {:else if lines.length}
        <p class="total selectable">
          {count(tokens(total))} tokens in {count(total.calls)} model calls ({count(total.input)} in, {count(total.output)}
          out) · {count(toolCalls)} tool calls
        </p>

        {#each providers as provider (provider.key)}
          <h2>{named(provider.key)}</h2>
          {#if about[provider.key]}<p class="about">{about[provider.key]}</p>{/if}
          <table>
            <thead>
              {#if tool(provider.key)}
                <tr><th>Tool</th><th>Calls</th></tr>
              {:else}
                <tr><th>Model</th><th>Calls</th><th>Tokens in</th><th>Tokens out</th><th>Tokens</th></tr>
              {/if}
            </thead>
            <tbody>
              {#each provider.rows as row (row.key)}
                <tr>
                  <td class="selectable"><code>{row.key}</code></td>
                  <td>{count(row.calls)}</td>
                  {#if !tool(provider.key)}
                    <td>{count(row.input)}</td>
                    <td>{row.output ? count(row.output) : ""}</td>
                    <td>{count(tokens(row))}</td>
                  {/if}
                </tr>
              {/each}
              {#if provider.rows.length > 1}
                <tr class="sum">
                  <td>All</td>
                  <td>{count(provider.calls)}</td>
                  {#if !tool(provider.key)}
                    <td>{count(provider.input)}</td>
                    <td>{provider.output ? count(provider.output) : ""}</td>
                    <td>{count(tokens(provider))}</td>
                  {/if}
                </tr>
              {/if}
            </tbody>
          </table>
        {/each}

        <h2>By what it was for</h2>
        <table>
          <thead><tr><th>Work</th><th>Model calls</th><th>Tokens in</th><th>Tokens out</th><th>Tool calls</th></tr></thead>
          <tbody>
            {#each purposes as line (line.key)}
              <tr>
                <td class="selectable">{line.name}</td>
                <td>{count(line.calls)}</td>
                <td>{count(line.input)}</td>
                <td>{count(line.output)}</td>
                <td>{line.tools ? count(line.tools) : ""}</td>
              </tr>
            {/each}
          </tbody>
        </table>

        <h2>By day</h2>
        <table>
          <thead><tr><th>Day</th><th>Model calls</th><th>Tokens in</th><th>Tokens out</th><th>Tool calls</th></tr></thead>
          <tbody>
            {#each byDay as line (line.key)}
              <tr>
                <td>{line.key}</td>
                <td>{count(line.calls)}</td>
                <td>{count(line.input)}</td>
                <td>{count(line.output)}</td>
                <td>{line.tools ? count(line.tools) : ""}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <p class="note">Counted from Tokidoki's own journal on this Mac. Voice runs here and uses none of this.</p>
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
    width: min(760px, 100%);
    margin: 0 auto;
    padding: 8px 28px 40px;
  }
  h1 {
    margin: 4vh 0 8px;
    font: 500 30px/1.15 var(--voice);
    letter-spacing: -0.02em;
  }
  h2 {
    margin: 30px 0 6px;
    font: 500 18px/1.3 var(--voice);
  }
  .about {
    margin: 0 0 8px;
    max-width: 62ch;
    color: var(--muted);
    font-size: 12.5px;
  }
  .lede {
    margin: 0 0 18px;
    max-width: 60ch;
    color: var(--muted);
    font-size: 15px;
  }
  .ranges {
    display: flex;
    gap: 6px;
  }
  .ranges button {
    height: 28px;
    padding: 0 12px;
    border-radius: 8px;
    background: var(--side);
    color: var(--muted);
    font-size: 12.5px;
    font-weight: 600;
  }
  .ranges button.on {
    color: var(--ink);
    box-shadow: inset 0 0 0 1px var(--indigo);
  }
  .total {
    margin: 20px 0 0;
    font: 17px/1.45 var(--voice);
    font-variant-numeric: tabular-nums;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  th {
    padding: 0 0 6px 14px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 400;
    text-align: right;
    white-space: nowrap;
  }
  td {
    padding: 8px 0 8px 14px;
    border-top: 1px solid var(--line);
    text-align: right;
    white-space: nowrap;
  }
  th:first-child,
  td:first-child {
    padding-left: 0;
    text-align: left;
    white-space: normal;
    overflow-wrap: anywhere;
  }
  td code {
    font: 12.5px var(--code);
  }
  .sum td {
    font-weight: 600;
  }
  .note,
  .empty {
    margin: 22px 0 0;
    max-width: 64ch;
    color: var(--muted);
    font-size: 12.5px;
  }
  .error {
    margin: 14px 0 0;
    color: var(--danger);
  }
</style>
