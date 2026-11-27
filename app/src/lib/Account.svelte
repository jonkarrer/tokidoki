<script lang="ts">
  import Keys from "./Keys.svelte";
  import { app, show } from "./state.svelte";
  import Threads from "./Threads.svelte";
  import Usage from "./Usage.svelte";

  // Account, from the foot of the sidebar: what Tokidoki spent, whose models it runs on and
  // the keys for them, and the threads under the hood. One page, three parts; which shows
  // is the app's view (`usage`, `keys`, `threads`), so a link to any of them still lands.
  let { ondone }: { ondone: () => Promise<void> } = $props();

  const parts = [
    { view: "usage", name: "Usage" },
    { view: "keys", name: "Model and keys" },
    { view: "threads", name: "Threads" },
  ] as const;
</script>

<div class="account">
  <div class="tabs" data-tauri-drag-region>
    {#each parts as part (part.view)}
      <button class:on={app.view === part.view} onclick={() => show(part.view)}>{part.name}</button>
    {/each}
  </div>
  <div class="part">
    {#if app.view === "keys"}
      <div class="center">
        <Keys
          ondone={async () => {
            await ondone();
            show("conversation");
          }}
          oncancel={app.status?.state === "ready" ? () => show("conversation") : undefined}
        />
      </div>
    {:else if app.view === "threads"}
      <Threads />
    {:else}
      <Usage />
    {/if}
  </div>
</div>

<style>
  .account {
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .tabs {
    flex: none;
    display: flex;
    align-items: flex-end;
    gap: 22px;
    height: 46px;
    padding: 0 28px 9px;
    border-bottom: 1px solid var(--line);
    color: var(--muted);
    font-size: 13px;
    font-weight: 600;
  }
  .tabs button:hover,
  .tabs button.on {
    color: var(--ink);
  }
  .tabs button.on {
    box-shadow: 0 10px 0 -8px var(--indigo);
  }
  .part {
    flex: 1;
    min-height: 0;
  }
  /* The parts are pages of their own elsewhere, each with room at the top for the title
     bar: here the tabs are that room. */
  .part :global(section > header) {
    display: none;
  }
  .center {
    height: 100%;
    overflow-y: auto;
    display: grid;
    place-items: start center;
    padding: 28px 28px 40px;
  }
</style>
