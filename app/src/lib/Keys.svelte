<script lang="ts">
  import { onMount } from "svelte";
  import { core, type Chosen } from "./core";
  import Presence from "./Presence.svelte";

  // Whose models Tokidoki runs on, and the user's own key for them: at first launch
  // (`first`), and later from the sidebar. A key field left empty keeps the key already
  // stored for that provider, so switching back to one never asks again.
  let { first = false, ondone, oncancel }: { first?: boolean; ondone: () => void; oncancel?: () => void } = $props();

  let chosen = $state<Chosen | null>(null);
  let provider = $state("anthropic");
  let key = $state("");
  let model = $state("");
  let saving = $state(false);
  let error = $state<string | null>(null);

  onMount(async () => {
    chosen = await core.chosen();
    provider = chosen.provider;
    model = chosen.model ?? "";
  });

  const picked = $derived(chosen?.providers.find((p) => p.id === provider));
  const ready = $derived(key.trim() !== "" || picked?.has_key === true);

  function pick(id: string) {
    if (id === provider) return;
    provider = id;
    key = "";
    // A model name belongs to the provider it was typed for.
    model = id === chosen?.provider ? (chosen.model ?? "") : "";
    error = null;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!ready || saving) return;
    saving = true;
    error = null;
    try {
      await core.choose(provider, key.trim() || null, model.trim() || null);
      ondone();
    } catch (problem) {
      error = String(problem);
    } finally {
      saving = false;
    }
  }
</script>

<form class="keys" onsubmit={save}>
  {#if first}
    <Presence doing={saving ? { kind: "thinking" } : null} size={44} />
    <h1>Tokidoki runs on your own key</h1>
    <p class="lede">
      It talks to the models directly from this Mac. Your key is kept in the Keychain and sent nowhere else.
    </p>
  {:else}
    <h1>Model and key</h1>
  {/if}

  {#if chosen && picked}
    <fieldset>
      <legend>Provider</legend>
      <div class="providers">
        {#each chosen.providers as option (option.id)}
          <label class="provider" class:on={option.id === provider}>
            <input type="radio" name="provider" value={option.id} checked={option.id === provider} onchange={() => pick(option.id)} />
            <span>{option.name}</span>
            {#if option.has_key}<small>key saved</small>{/if}
          </label>
        {/each}
      </div>
    </fieldset>

    <label class="field">
      <span>{picked.name} API key</span>
      <input
        type="password"
        bind:value={key}
        placeholder={picked.has_key ? "Saved. Paste a new key to replace it" : "Paste your key"}
        autocomplete="off"
        spellcheck="false"
      />
      <small>Create one at {picked.keys_at}. It answers, remembers and files your messages.</small>
    </label>

    <label class="field">
      <span>Model <em>optional</em></span>
      <input type="text" bind:value={model} placeholder={picked.model} autocomplete="off" spellcheck="false" />
      <small>Leave empty for {picked.name}'s usual model, shown above.</small>
    </label>
  {/if}

  {#if error}<p class="error selectable" role="alert">{error}</p>{/if}

  <div class="actions">
    {#if oncancel}<button type="button" class="quiet" onclick={oncancel}>Cancel</button>{/if}
    <button type="submit" class="primary" disabled={!ready || saving}>
      {saving ? "Checking the key…" : first ? "Save and start" : "Save"}
    </button>
  </div>
</form>

<style>
  .keys {
    width: min(440px, 100%);
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  h1 {
    margin: 6px 0 0;
    font: 600 22px/1.25 var(--voice);
    letter-spacing: -0.01em;
  }
  .lede {
    margin: -8px 0 4px;
    color: var(--muted);
  }
  fieldset {
    margin: 0;
    padding: 0;
    border: 0;
    min-width: 0;
  }
  legend,
  .field span {
    padding: 0;
    font-weight: 600;
  }
  .providers {
    margin-top: 6px;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .provider {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 9px 11px;
    min-height: 54px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--raised);
    font-weight: 600;
  }
  .provider.on {
    border-color: var(--indigo);
    box-shadow: inset 0 0 0 1px var(--indigo);
  }
  .provider input {
    position: absolute;
    inset: 0;
    opacity: 0;
    margin: 0;
  }
  .provider:has(input:focus-visible) {
    outline: 2px solid var(--indigo);
    outline-offset: 2px;
  }
  .provider small {
    font-weight: 400;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field em {
    font-style: normal;
    font-weight: 400;
    color: var(--muted);
    margin-left: 4px;
  }
  .field input {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--raised);
    font-family: var(--code);
    font-size: 13px;
  }
  .field input:focus {
    outline: 2px solid var(--indigo);
    outline-offset: -1px;
  }
  small {
    color: var(--muted);
    font-size: 12px;
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .actions button {
    height: 32px;
    padding: 0 14px;
    border-radius: 8px;
    font-weight: 600;
  }
  .primary {
    background: var(--indigo);
    color: var(--paper);
  }
  .primary:disabled {
    opacity: 0.45;
  }
  .quiet {
    background: var(--side);
  }
</style>
