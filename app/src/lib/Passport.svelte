<script lang="ts">
  import { onMount } from "svelte";
  import { core, type Account } from "./core";

  // Passport: the sites the user is signed in to, for missions that need to be. Tokidoki
  // holds no passwords: signing in is done by the user, in the browser's own window (Sign in
  // opens it), and the session is kept there (src-tauri/src/passport.rs).
  const known = [
    { name: "Amazon", site: "amazon.com", sign_in_url: "https://www.amazon.com/ap/signin" },
    { name: "Reddit", site: "reddit.com", sign_in_url: "https://www.reddit.com/login" },
    { name: "X", site: "x.com", sign_in_url: "https://x.com/i/flow/login" },
    { name: "GitHub", site: "github.com", sign_in_url: "https://github.com/login" },
    { name: "Notion", site: "notion.com", sign_in_url: "https://www.notion.com/login" },
    { name: "Fly.io", site: "fly.io", sign_in_url: "https://fly.io/app/sign-in" },
  ];

  let accounts = $state<Account[]>([]);
  /** The account being added or changed: its site as first saved, or "" for a new one. */
  let editing = $state<string | null>(null);
  let draft = $state({ name: "", site: "", sign_in_url: "" });
  let error = $state<string | null>(null);
  let removing = $state<string | null>(null);

  const offered = $derived(known.filter((site) => !accounts.some((account) => account.site === site.site)));
  const current = $derived(accounts.find((account) => account.site === editing));

  async function load() {
    accounts = await core.passport();
  }
  onMount(() => {
    load();
    // Back from signing in, in the browser's window: what it holds may have changed.
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  });

  function add(from?: (typeof known)[number]) {
    draft = { name: from?.name ?? "", site: from?.site ?? "", sign_in_url: from?.sign_in_url ?? "" };
    editing = "";
    error = null;
  }

  function change(account: Account) {
    draft = { name: account.name, site: account.site, sign_in_url: account.sign_in_url };
    editing = account.site;
    error = null;
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    error = null;
    try {
      await core.saveAccount(draft);
      editing = null;
      await load();
    } catch (problem) {
      error = String(problem);
    }
  }

  async function remove(site: string) {
    await core.deleteAccount(site);
    removing = null;
    editing = null;
    await load();
  }

  async function signIn(site: string) {
    error = null;
    try {
      await core.signInAccount(site);
    } catch (problem) {
      error = String(problem);
    }
  }
</script>

<section>
  <header data-tauri-drag-region></header>
  <div class="scroll">
    <div class="column">
      {#if editing !== null}
        <button class="back" onclick={() => (editing = null)}>← Passport</button>
        <h1>{current ? current.name : "Add an account"}</h1>
        <form onsubmit={save}>
          <label>
            <span>Name</span>
            <input type="text" bind:value={draft.name} placeholder="Amazon" autocomplete="off" spellcheck="false" />
          </label>
          <label>
            <span>Site</span>
            <input type="text" bind:value={draft.site} placeholder="amazon.com" autocomplete="off" spellcheck="false" disabled={!!current} required />
          </label>
          <label>
            <span>Sign-in page <em>optional</em></span>
            <input type="text" bind:value={draft.sign_in_url} placeholder="The site's front page" autocomplete="off" spellcheck="false" />
            <small>Where Sign in takes you. It has to be on the site itself.</small>
          </label>
          {#if error}<p class="error selectable" role="alert">{error}</p>{/if}
          <div class="actions">
            {#if current}
              {#if removing === current.site}
                <span class="ask">Remove {current.name}?</span>
                <button type="button" class="quiet" onclick={() => (removing = null)}>Keep it</button>
                <button type="button" class="danger" onclick={() => remove(current.site)}>Remove</button>
              {:else}
                <button type="button" class="quiet left" onclick={() => (removing = current.site)}>Remove account</button>
              {/if}
            {/if}
            {#if removing === null}
              <button type="button" class="quiet" onclick={() => (editing = null)}>Cancel</button>
              <button type="submit" class="primary">Save</button>
            {/if}
          </div>
        </form>
      {:else}
        <h1>Passport</h1>
        <p class="lede">
          The sites you are signed in to, for missions that need to be. Sign in to each yourself, in Tokidoki's own
          browser, and it stays signed in there. Tokidoki never holds a password.
        </p>
        {#if error}<p class="error selectable" role="alert">{error}</p>{/if}
        {#if accounts.length}
          <ul>
            {#each accounts as account (account.site)}
              <li>
                <button class="who" onclick={() => change(account)}>
                  <span class="name">{account.name}</span>
                  <span class="meta">
                    {account.site} · {account.visited ? "the browser has been there" : "not signed in yet"}
                  </span>
                </button>
                <button class="go" onclick={() => signIn(account.site)}>Sign in</button>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="add">
          {#each offered as site (site.site)}
            <button onclick={() => add(site)}>{site.name}</button>
          {/each}
          <button onclick={() => add()}>Another site…</button>
        </div>
        <p class="note">
          “The browser has been there” means it holds something from a visit, not that the session is still good: a
          site can sign you out at any time. If a mission says it needs you to sign in, do it here.
        </p>
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
  }
  form {
    width: min(440px, 100%);
    display: flex;
    flex-direction: column;
    gap: 16px;
    margin-top: 18px;
  }
  .lede {
    margin: 0 0 22px;
    max-width: 56ch;
    color: var(--muted);
    font-size: 15px;
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--line);
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    border-bottom: 1px solid var(--line);
  }
  .who {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 13px 0 11px;
    text-align: left;
  }
  .who:hover .name {
    color: var(--indigo);
  }
  .name {
    font: 16.5px/1.4 var(--voice);
  }
  .meta {
    color: var(--muted);
    font-size: 12.5px;
    overflow-wrap: anywhere;
  }
  .go,
  .add button,
  .actions button {
    flex: none;
    height: 30px;
    padding: 0 13px;
    border-radius: 8px;
    background: var(--side);
    font-size: 13px;
    font-weight: 600;
  }
  .go:hover,
  .add button:hover {
    color: var(--indigo);
  }
  .add {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 18px;
  }
  .add::before {
    content: "Add";
    align-self: center;
    margin-right: 2px;
    color: var(--muted);
    font-size: 13px;
  }
  .note {
    margin: 22px 0 0;
    max-width: 60ch;
    color: var(--muted);
    font-size: 12.5px;
  }
  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  label span {
    font-weight: 600;
  }
  label em {
    margin-left: 4px;
    font-style: normal;
    font-weight: 400;
    color: var(--muted);
  }
  input {
    height: 34px;
    padding: 0 10px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: var(--raised);
    font-size: 14px;
  }
  input:focus {
    outline: 2px solid var(--indigo);
    outline-offset: -1px;
  }
  input:disabled {
    opacity: 0.6;
  }
  small {
    color: var(--muted);
    font-size: 12px;
  }
  .back {
    margin-top: 18px;
    color: var(--muted);
    font-size: 13px;
  }
  .back:hover {
    color: var(--ink);
  }
  .error {
    margin: 0;
    color: var(--danger);
  }
  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .actions .left {
    margin-right: auto;
  }
  .ask {
    margin-right: auto;
    font-size: 13px;
  }
  .actions .primary {
    background: var(--indigo);
    color: var(--paper);
  }
  .actions .danger {
    background: var(--danger);
    color: var(--paper);
  }
</style>
