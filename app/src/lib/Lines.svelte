<script lang="ts">
  import { render } from "./markdown";
  import { app, show, type Item } from "./state.svelte";

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
    <div class="reply selectable">{@html render(line.text)}</div>
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
  {/if}
{/each}

<style>
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
</style>
