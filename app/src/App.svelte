<script lang="ts">
  import { onMount } from "svelte";
  import { core } from "./lib/core";
  import Conversation from "./lib/Conversation.svelte";
  import Keys from "./lib/Keys.svelte";
  import Memory from "./lib/Memory.svelte";
  import Notes from "./lib/Notes.svelte";
  import Presence from "./lib/Presence.svelte";
  import Threads from "./lib/Threads.svelte";
  import UpdateCard from "./lib/UpdateCard.svelte";
  import { answering, app, hasHistory, show, start } from "./lib/state.svelte";

  let version = $state("");

  onMount(() => {
    // The page is up once `start` has asked the core where things stand.
    start().then(async () => {
      if (import.meta.env.DEV) (await import("./lib/debug")).startDebug();
    });
    core.version().then((v) => (version = v));
  });

  // What Tokidoki is doing in the conversation, shown beside it from anywhere in the app.
  const doing = $derived.by(() => {
    const thread = answering(app.log)[0];
    return thread ? (app.live[thread]?.doing ?? { kind: "thinking" as const }) : null;
  });
</script>

{#if app.status === null}
  <div class="center" data-tauri-drag-region><Presence doing={{ kind: "thinking" }} size={44} /></div>
{:else if app.status.state === "needs_keys" && !hasHistory()}
  <div class="center" data-tauri-drag-region><Keys first ondone={start} /></div>
{:else if app.status.state === "failed"}
  <div class="center" data-tauri-drag-region>
    <div class="failed">
      <h1>Tokidoki couldn't start</h1>
      <p class="selectable">{app.status.message}</p>
      <button onclick={start}>Try again</button>
    </div>
  </div>
{:else}
  <div class="window">
    <nav>
      <div class="titlebar" data-tauri-drag-region></div>
      <!-- One conversation; the threads it is filed into are under the hood (Threads). -->
      <button class="place conversation" class:on={app.view === "conversation"} onclick={() => show("conversation")}>
        <span class="mark">
          {#if doing}<Presence {doing} size={12} />{:else}<span class="talk"></span>{/if}
        </span>
        <span>Conversation</span>
        <span class="count"></span>
      </button>
      <button class="place memory" class:on={app.view === "memory"} onclick={() => show("memory")}>
        <span class="mark"><span class="mote" class:fresh={app.unseen > 0}></span></span>
        <span>Memory</span>
        <span class="count" class:fresh={app.unseen > 0}>{app.unseen > 0 ? `${app.unseen} new` : app.memories.length || ""}</span>
      </button>
      <button class="place notes" class:on={app.view === "notes"} onclick={() => show("notes")}>
        <span class="mark"><span class="leaf"></span></span>
        <span>Notes</span>
        <span class="count">{app.notes.length || ""}</span>
      </button>
      <footer>
        <UpdateCard />
        <div class="foot">
          <button class:on={app.view === "threads"} onclick={() => show("threads")}>Threads</button>
          <button onclick={() => show("keys")}>Model and key</button>
          <button onclick={() => core.checkForUpdates()} title="Check for updates">v{version}</button>
        </div>
      </footer>
    </nav>
    <main>
      {#if app.view === "keys"}
        <div class="center"><Keys
            ondone={async () => {
              await start();
              show("conversation");
            }}
            oncancel={app.status.state === "ready" ? () => show("conversation") : undefined}
          /></div>
      {:else if app.view === "memory"}
        <Memory />
      {:else if app.view === "notes"}
        <Notes />
      {:else if app.view === "threads"}
        <Threads />
      {:else}
        <Conversation />
      {/if}
    </main>
  </div>
{/if}

<style>
  .center {
    height: 100%;
    display: grid;
    place-items: center;
    padding: 40px;
  }
  .failed {
    max-width: 460px;
  }
  .failed h1 {
    margin: 0 0 8px;
    font: 600 22px/1.25 var(--voice);
  }
  .failed p {
    color: var(--muted);
  }
  .failed button {
    height: 32px;
    padding: 0 14px;
    border-radius: 8px;
    background: var(--indigo);
    color: var(--paper);
    font-weight: 600;
  }
  .window {
    height: 100%;
    display: grid;
    grid-template-columns: 236px 1fr;
  }
  nav {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--side);
    border-right: 1px solid var(--line);
  }
  .titlebar {
    height: 52px;
    flex: none;
  }
  .place {
    display: grid;
    grid-template-columns: 14px 1fr auto;
    align-items: center;
    gap: 6px;
    margin: 0 6px 2px;
    padding: 7px 8px 7px 4px;
    border-radius: 7px;
    text-align: left;
  }
  .place:hover {
    background: color-mix(in srgb, var(--raised) 55%, transparent);
  }
  .place.on {
    background: var(--raised);
  }
  .mark {
    display: grid;
    place-items: center;
  }
  /* The conversation: a speech mark, indigo like what the user says. */
  .talk {
    width: 8px;
    height: 8px;
    border-radius: 50% 50% 50% 0;
    border: 1.5px solid var(--indigo);
  }
  .mote {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    border: 1.5px solid var(--plum);
  }
  .mote.fresh {
    background: var(--plum);
  }
  /* A note: a page with a folded corner, small. */
  .leaf {
    width: 8px;
    height: 10px;
    border: 1.5px solid var(--indigo);
    border-radius: 1px 4px 1px 1px;
  }
  .count {
    color: var(--muted);
    font-size: 11.5px;
    font-variant-numeric: tabular-nums;
  }
  .count.fresh {
    color: var(--plum);
    font-weight: 600;
  }
  footer {
    flex: none;
    margin-top: auto;
    padding: 8px 10px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .foot {
    display: flex;
    justify-content: space-between;
    color: var(--muted);
    font-size: 12px;
  }
  .foot button:hover,
  .foot button.on {
    color: var(--ink);
  }
  main {
    min-width: 0;
    min-height: 0;
  }
  @media (max-width: 640px) {
    .window {
      grid-template-columns: 1fr;
    }
    nav {
      display: none;
    }
  }
</style>
