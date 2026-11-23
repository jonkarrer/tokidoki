<script lang="ts">
  import { render } from "./markdown";
  import { app, show, type Item } from "./state.svelte";
  import { ready, speak, stopSpeaking, voice } from "./voice.svelte";

  // The lines of a conversation: what the user said, what Tokidoki answered, and around them
  // what it remembered and learned. Where the talk moves to another thread there is a quiet
  // gap, and hovering what the user said there shows which thread it went to.
  let { lines }: { lines: Item[] } = $props();

  function title(thread: string) {
    return app.threads.find((t) => t.id === thread)?.title ?? thread;
  }
</script>

{#each lines as line (line.key)}
  {#if line.kind === "user"}
    <p class="said selectable" class:moved={line.moved} title={title(line.thread)} data-thread={line.thread}>
      {line.text}
    </p>
  {:else if line.kind === "reply"}
    <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
    <div class="answer">
      <div class="reply selectable">{@html render(line.text)}</div>
      {#if ready()}
        <button
          class="aloud"
          class:on={voice.playing === line.key}
          onclick={() => (voice.playing === line.key ? stopSpeaking() : speak(line.key, line.text))}
          >{voice.playing === line.key ? "Stop reading" : "Read aloud"}</button
        >
      {/if}
    </div>
  {:else if line.kind === "recalled"}
    <ul class="recalled">
      {#each line.memories as memory}<li class="selectable">{memory}</li>{/each}
    </ul>
  {:else if line.kind === "learned"}
    <button class="learned" onclick={() => show("memory")} title="Open memory">
      <span>{line.memory.version > 1 ? "Updated what it knows" : "Learned"}</span>
      {line.memory.text}
    </button>
  {:else if line.kind === "tool"}
    <p class="aside">Used {line.name}</p>
  {:else if line.kind === "compacted"}
    <p class="aside rule">Earlier messages are summarized from here</p>
  {:else if line.kind === "report"}
    {@const worker = app.workers.find((w) => w.id === line.report.worker)}
    {@const held = worker?.delivery === "held"}
    <details class="report" class:failed={line.report.status !== "done"}>
      <summary>
        <span class="what"
          >{line.report.status === "done" ? "Research came back" : "Research couldn't be done"}{#if held}{" · kept for when you come back to it"}{/if}</span
        >
        <span class="task">{worker?.title || line.report.task}</span>
      </summary>
      <!-- eslint-disable-next-line svelte/no-at-html-tags (sanitized in render) -->
      <div class="reply selectable">{@html render(line.report.body)}</div>
    </details>
  {/if}
{/each}

<style>
  /* Read aloud: under a reply, shown when the pointer is over it, or while it plays. */
  .aloud {
    margin-top: 4px;
    color: var(--muted);
    font-size: 12px;
    opacity: 0;
  }
  .answer:hover .aloud,
  .aloud:focus-visible,
  .aloud.on {
    opacity: 1;
  }
  .aloud:hover,
  .aloud.on {
    color: var(--indigo);
  }
  /* The talk changed subject: room, nothing more. */
  .moved {
    margin-top: 64px;
  }
  /* What it remembered, coming into the conversation. */
  .recalled {
    margin: 0 0 12px;
    padding: 0;
    list-style: none;
    color: var(--plum);
    font-size: 13px;
  }
  .recalled li {
    padding-left: 16px;
    position: relative;
  }
  .recalled li::before {
    content: "";
    position: absolute;
    left: 2px;
    top: 0.55em;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--plum);
  }
  /* What it learned here: the same plum, set apart from what it recalled by its outline. */
  .learned {
    display: block;
    margin: 10px 0 12px;
    padding: 7px 12px;
    border: 1px solid color-mix(in srgb, var(--plum) 45%, transparent);
    border-radius: 9px;
    color: var(--plum);
    font-size: 13px;
    text-align: left;
  }
  .learned span {
    display: block;
    font-weight: 650;
  }
  .learned:hover {
    background: color-mix(in srgb, var(--plum) 8%, transparent);
  }
  .aside {
    margin: 10px 0;
    color: var(--muted);
    font-size: 13px;
  }
  .rule {
    padding-bottom: 10px;
    border-bottom: 1px solid var(--line);
  }
  /* What a worker brought back: indigo, like what Tokidoki does; opened for the whole report. */
  .report {
    margin: 14px 0;
    border: 1px solid color-mix(in srgb, var(--indigo) 45%, transparent);
    border-radius: 10px;
  }
  .report summary {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 8px 12px;
    list-style: none;
    cursor: default;
  }
  .report summary::-webkit-details-marker {
    display: none;
  }
  .report summary:hover {
    background: color-mix(in srgb, var(--indigo) 7%, transparent);
  }
  .report .what {
    color: var(--indigo);
    font-size: 12.5px;
    font-weight: 650;
  }
  .report .what::after {
    content: " ›";
  }
  .report[open] .what::after {
    content: " ⌄";
  }
  .report .task {
    color: var(--muted);
    font-size: 13px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .report .reply {
    padding: 4px 14px 14px;
    font-size: 15px;
  }
  .report.failed {
    border-color: color-mix(in srgb, var(--danger) 45%, transparent);
  }
  .report.failed .what {
    color: var(--danger);
  }
</style>
