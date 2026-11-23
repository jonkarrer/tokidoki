<script lang="ts">
  import { tick } from "svelte";
  import Lines from "./Lines.svelte";
  import { render } from "./markdown";
  import Presence from "./Presence.svelte";
  import { answering, app, items, loadEarlier, send, stop, type Presence as Doing } from "./state.svelte";

  // The one conversation: everything said, whatever thread it went to, the replies being
  // written, and where to write the next message. Tokidoki files each message with the
  // thread it belongs to; the user never picks one.
  let draft = $state("");
  let sending = $state(false);
  let scroller = $state<HTMLElement>();
  let input = $state<HTMLTextAreaElement>();

  const lines = $derived(items(app.log, app.memories));
  const busy = $derived(answering(app.log));
  /** Every thread at work: what it's doing and what it has written so far. */
  const working = $derived(
    busy.map((thread) => {
      const now = app.live[thread];
      // Between the commit that starts a turn and the first sign of work, it is thinking.
      return { thread, text: now?.text ?? "", doing: (now?.doing ?? { kind: "thinking" }) as Doing };
    }),
  );

  function label(doing: Doing) {
    switch (doing?.kind) {
      case "remembering":
        return "Remembering";
      case "thinking":
        return "Thinking";
      case "tool":
        return `Using ${doing.name}`;
      default:
        return "";
    }
  }

  function greeting() {
    const hour = new Date().getHours();
    return hour < 5 ? "Still up." : hour < 12 ? "Good morning." : hour < 18 ? "Good afternoon." : "Good evening.";
  }

  /** Why an input got no answer, in the user's terms. */
  function explain(reason: string) {
    if (/\b401\b|\b403\b|authentication/i.test(reason))
      return "The provider didn't accept the key. Replace it under Model and key, in the sidebar.";
    if (reason.startsWith("model_error")) return `The model couldn't be reached. ${reason.replace(/^model_error:\s*/, "")}`;
    if (reason === "turn_limit") return "Tokidoki stopped after too many steps without an answer.";
    return `No answer: ${reason}`;
  }

  // Follow the conversation down as it grows, unless the user has scrolled up to read.
  let pinned = true;
  function onscroll() {
    if (scroller) pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 80;
  }
  $effect(() => {
    void [lines.length, working.map((w) => w.text + w.doing?.kind).join(), app.queued.length];
    if (pinned) tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
  });
  $effect(() => {
    input?.focus();
  });

  /** Earlier turns, keeping what is on screen where it is. */
  async function earlier() {
    if (!scroller) return;
    const below = scroller.scrollHeight - scroller.scrollTop;
    await loadEarlier();
    await tick();
    scroller.scrollTop = scroller.scrollHeight - below;
  }

  // The box grows with what is written in it (`field-sizing` does this alone where the
  // system's WebKit has it).
  $effect(() => {
    void draft;
    if (!input || CSS.supports("field-sizing", "content")) return;
    input.style.height = "auto";
    input.style.height = `${input.scrollHeight}px`;
  });

  async function submit() {
    const text = draft.trim();
    if (!text || sending) return;
    sending = true;
    pinned = true;
    if (await send(text)) draft = "";
    sending = false;
    input?.focus();
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      submit();
    }
  }
</script>

<section>
  <header data-tauri-drag-region></header>

  <div class="scroll" bind:this={scroller} {onscroll}>
    <div class="column">
      {#if app.earlier}
        <button class="earlier" onclick={earlier}>Earlier</button>
      {/if}
      {#if lines.length === 0 && working.length === 0}
        <div class="hello">
          <Presence doing={sending ? { kind: "thinking" } : null} size={56} />
          <h1>{greeting()}</h1>
          <p>Say what's on your mind. Tokidoki keeps track of what you talk about, and what matters for next time.</p>
        </div>
      {/if}

      <Lines {lines} />

      {#each working as now (now.thread)}
        {#if now.text}
          <div class="reply selectable">{@html render(now.text)}</div>
        {/if}
        {#if now.doing && now.doing.kind !== "writing"}
          <p class="working"><Presence doing={now.doing} size={18} /> {label(now.doing)}</p>
        {/if}
      {/each}
      {#each app.queued as waiting (waiting.id)}
        <p class="said waiting">{waiting.text} <span>Sends after this answer</span></p>
      {/each}
      {#if app.unanswered !== null && working.length === 0}
        <p class="problem selectable" role="alert">{explain(app.unanswered)}</p>
      {/if}
    </div>
  </div>

  <form class="composer" onsubmit={(event) => (event.preventDefault(), submit())}>
    {#if app.sendError}<p class="problem selectable" role="alert">{app.sendError}</p>{/if}
    <div class="box">
      <textarea
        bind:this={input}
        bind:value={draft}
        {onkeydown}
        rows="1"
        placeholder="Message Tokidoki"
        aria-label="Message"
      ></textarea>
      {#if working.length}
        <button type="button" class="stop" onclick={stop}>Stop</button>
      {/if}
      <button type="submit" class="send" disabled={!draft.trim() || sending}>Send</button>
    </div>
  </form>
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
    padding: 8px 28px 28px;
  }
  .hello {
    padding-top: 14vh;
  }
  .hello h1 {
    margin: 22px 0 8px;
    font: 500 34px/1.1 var(--voice);
    letter-spacing: -0.02em;
  }
  .hello p {
    margin: 0;
    max-width: 44ch;
    color: var(--muted);
    font-size: 15px;
  }
  .earlier {
    display: block;
    margin: 4px auto 12px;
    color: var(--muted);
    font-size: 12.5px;
  }
  .earlier:hover {
    color: var(--ink);
  }
  .working {
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 12px 0 0;
    color: var(--muted);
    font-size: 13px;
  }
  .problem {
    margin: 12px 0 0;
    color: var(--danger);
    font-size: 13px;
  }

  .composer {
    flex: none;
    width: min(680px, 100%);
    margin: 0 auto;
    padding: 0 28px 22px;
  }
  .composer .problem {
    margin: 0 0 8px;
  }
  .box {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    padding: 8px 8px 8px 14px;
    border: 1px solid var(--line);
    border-radius: 14px;
    background: var(--raised);
  }
  .box:focus-within {
    border-color: var(--indigo);
  }
  textarea {
    flex: 1;
    min-width: 0;
    max-height: 40vh;
    padding: 5px 0;
    border: 0;
    outline: 0;
    resize: none;
    background: none;
    font-size: 15px;
    line-height: 1.45;
    field-sizing: content;
  }
  textarea::placeholder {
    color: var(--muted);
  }
  .box button {
    height: 30px;
    padding: 0 13px;
    border-radius: 8px;
    font-weight: 600;
  }
  .send {
    background: var(--indigo);
    color: var(--paper);
  }
  .send:disabled {
    opacity: 0.4;
  }
  .stop {
    background: var(--side);
  }
</style>
