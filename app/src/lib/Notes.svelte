<script lang="ts">
  import { core, type Note } from "./core";
  import { render } from "./markdown";
  import { app, showThread } from "./state.svelte";

  // The user's notes: kept as written, apart from what Tokidoki remembers. Written here, or
  // saved by Tokidoki when asked in a conversation. The list follows the core's note events
  // (state.svelte.ts), so a note saved in a conversation shows up here as it happens.
  let query = $state("");
  let tag = $state<string | null>(null);
  /** What the core found for `query`; the list itself when there is no query. */
  let found = $state<Note[] | null>(null);
  let reading = $state<number | null>(null);
  /** The note being written: `id` null for a new one. Lists are typed as comma-separated. */
  let editing = $state<{ id: number | null; title: string; body: string; tags: string; project: string; people: string } | null>(
    null,
  );
  let deleting = $state(false);
  let error = $state<string | null>(null);

  // Search runs in the core, and again whenever the notes change.
  $effect(() => {
    const words = query.trim();
    const within = tag;
    void app.notes;
    if (!words) {
      found = null;
      return;
    }
    const timer = setTimeout(async () => {
      const result = await core.notes({ query: words, tag: within, project: null });
      if (query.trim() === words && tag === within) found = result;
    }, 120);
    return () => clearTimeout(timer);
  });

  const shown = $derived(found ?? app.notes.filter((note) => !tag || note.tags.some((t) => same(t, tag!))));
  const open = $derived(app.notes.find((note) => note.id === reading) ?? null);

  /** Every tag in use, the most used first. */
  const tags = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const note of app.notes)
      for (const t of note.tags) {
        const known = [...counts.keys()].find((k) => same(k, t)) ?? t;
        counts.set(known, (counts.get(known) ?? 0) + 1);
      }
    return [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([t]) => t);
  });

  function same(a: string, b: string) {
    return a.localeCompare(b, undefined, { sensitivity: "accent" }) === 0;
  }

  function write(note: Note | null) {
    error = null;
    deleting = false;
    editing = note
      ? {
          id: note.id,
          title: note.title,
          body: note.body,
          tags: note.tags.join(", "),
          project: note.project ?? "",
          people: note.people.join(", "),
        }
      : { id: null, title: "", body: "", tags: tag ?? "", project: "", people: "" };
  }

  async function save(event: SubmitEvent) {
    event.preventDefault();
    if (!editing) return;
    const split = (text: string) => text.split(",").map((part) => part.trim()).filter(Boolean);
    error = null;
    try {
      const note = await core.writeNote(editing.id, {
        title: editing.title,
        body: editing.body,
        tags: split(editing.tags),
        project: editing.project.trim() || null,
        people: split(editing.people),
      });
      editing = null;
      reading = note.id;
    } catch (problem) {
      error = String(problem);
    }
  }

  async function remove(id: number) {
    error = null;
    try {
      await core.deleteNote(id);
      deleting = false;
      reading = null;
    } catch (problem) {
      error = String(problem);
    }
  }

  /** Leaves the editor: to the note, or for a new one, to the list. */
  function cancel() {
    if (editing?.id === null) back();
    else editing = null;
  }

  function back() {
    editing = null;
    reading = null;
    deleting = false;
    error = null;
  }

  /** The body as one plain line for the list, without the title when the title is its first line. */
  function excerpt(note: Note) {
    const lines = note.body
      .split("\n")
      .map((line) =>
        line
          .replace(/^\s*(?:[#>*+-]+|\d+[.)])\s*/, "")
          .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
          .replace(/[*_`~]/g, "")
          .trim(),
      )
      .filter(Boolean);
    if (lines[0] === note.title) lines.shift();
    return lines.join(" ");
  }

  function when(ms: number) {
    const days = Math.floor((Date.now() - ms) / 86_400_000);
    if (days < 1) return "today";
    if (days < 2) return "yesterday";
    if (days < 7) return `${days} days ago`;
    return new Date(ms).toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" });
  }

  function from(note: Note) {
    return note.thread ? app.threads.find((thread) => thread.id === note.thread) : undefined;
  }
</script>

<section>
  <header data-tauri-drag-region></header>
  <div class="scroll">
    <div class="column">
      {#if editing}
        <form class="editor" onsubmit={save}>
          <button type="button" class="back" onclick={cancel}>
            ← {editing.id === null ? "All notes" : "Back to the note"}
          </button>
          <input class="title" bind:value={editing.title} placeholder="Title, or leave it to the first line" aria-label="Title" />
          <div class="fields">
            <label>Tags <input bind:value={editing.tags} placeholder="work, ideas" /></label>
            <label>Project <input bind:value={editing.project} placeholder="None" /></label>
            <label>People <input bind:value={editing.people} placeholder="Ana, Sam" /></label>
          </div>
          <!-- svelte-ignore a11y_autofocus -->
          <textarea bind:value={editing.body} placeholder="Write anything. Markdown works." aria-label="Note" autofocus></textarea>
          {#if error}<p class="error selectable" role="alert">{error}</p>{/if}
          <div class="row end">
            <button type="button" onclick={cancel}>Cancel</button>
            <button type="submit" class="primary" disabled={!editing.title.trim() && !editing.body.trim()}>Save</button>
          </div>
        </form>
      {:else if open}
        {@const thread = from(open)}
        <article>
          <button class="back" onclick={back}>← All notes</button>
          <h1 class="selectable">{open.title}</h1>
          <p class="meta">
            {open.created_at === open.updated_at ? "Written" : "Changed"}
            {when(open.updated_at)}{#if thread}, saved in
              <button class="link" onclick={() => showThread(thread.id)}>{thread.title}</button>{/if}{#if open.project}
              · {open.project}{/if}{#if open.people.length}
              · with {open.people.join(", ")}{/if}
          </p>
          {#if open.tags.length}
            <div class="tags">
              {#each open.tags as t (t)}<span class="tag">{t}</span>{/each}
            </div>
          {/if}
          <div class="body selectable">{@html render(open.body)}</div>
          {#if error}<p class="error selectable" role="alert">{error}</p>{/if}
          <div class="row">
            {#if deleting}
              <span class="ask">Delete this note for good?</span>
              <button onclick={() => (deleting = false)}>Keep it</button>
              <button class="danger" onclick={() => remove(open.id)}>Delete</button>
            {:else}
              <button onclick={() => write(open)}>Edit</button>
              <button onclick={() => (deleting = true)}>Delete…</button>
            {/if}
          </div>
        </article>
      {:else}
        <h1>Notes</h1>
        <p class="lede">
          Yours, kept as you wrote them and apart from what Tokidoki remembers. Write one here, or tell Tokidoki “save
          these notes: …” in a conversation.
        </p>
        <div class="row">
          <input class="search" type="search" bind:value={query} placeholder="Search notes" aria-label="Search notes" />
          <button class="primary" onclick={() => write(null)}>New note</button>
        </div>
        {#if tags.length}
          <div class="tags filter">
            {#each tags as t (t)}
              <button class="tag" class:on={tag !== null && same(tag, t)} onclick={() => (tag = tag !== null && same(tag, t) ? null : t)}
                >{t}</button
              >
            {/each}
          </div>
        {/if}
        {#if app.notes.length === 0}
          <p class="empty">No notes yet.</p>
        {:else}
          <ul>
            {#each shown as note (note.id)}
              <li>
                <button onclick={() => (reading = note.id)}>
                  <span class="name">{note.title}</span>
                  {#if excerpt(note)}<span class="excerpt">{excerpt(note)}</span>{/if}
                  <span class="meta">
                    {[when(note.updated_at), note.tags.join(", "), note.thread ? "from a conversation" : ""]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                </button>
              </li>
            {:else}
              <li class="empty">No note matches{query.trim() ? ` “${query.trim()}”` : ""}{tag ? ` in ${tag}` : ""}.</li>
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
  }
  .lede {
    margin: 0 0 22px;
    max-width: 52ch;
    color: var(--muted);
    font-size: 15px;
  }
  input,
  textarea {
    border: 1px solid var(--line);
    border-radius: 9px;
    background: var(--raised);
    padding: 0 12px;
    font-size: 14.5px;
    min-width: 0;
  }
  input {
    height: 36px;
  }
  input:focus,
  textarea:focus {
    outline: 2px solid var(--indigo);
    outline-offset: -1px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row.end {
    justify-content: flex-end;
  }
  .search {
    flex: 1;
  }
  .row button:not(.primary) {
    height: 30px;
    padding: 0 12px;
    border-radius: 8px;
    background: var(--side);
    font-weight: 600;
    font-size: 13px;
  }
  .row button.danger {
    background: var(--danger);
    color: var(--paper);
  }
  .primary {
    height: 36px;
    padding: 0 14px;
    border-radius: 9px;
    background: var(--plum);
    color: var(--paper);
    font-weight: 600;
    flex: none;
  }
  .primary:disabled {
    opacity: 0.4;
  }
  .ask {
    color: var(--muted);
    font-size: 13px;
    margin-right: auto;
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 10px 0 0;
  }
  .tag {
    padding: 2px 9px;
    border-radius: 999px;
    border: 1px solid var(--line);
    color: var(--muted);
    font-size: 12px;
  }
  button.tag:hover {
    color: var(--ink);
  }
  .tag.on {
    border-color: var(--indigo);
    color: var(--indigo);
    font-weight: 600;
  }
  ul {
    margin: 14px 0 0;
    padding: 0;
    list-style: none;
  }
  li:not(.empty) {
    border-bottom: 1px solid var(--line);
  }
  li button {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 13px 0 11px;
    text-align: left;
  }
  li button:hover .name {
    color: var(--indigo);
  }
  .name {
    font: 16.5px/1.4 var(--voice);
    overflow-wrap: anywhere;
  }
  .excerpt {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--muted);
    font-size: 13.5px;
  }
  .meta {
    color: var(--muted);
    font-size: 12.5px;
  }
  article .meta {
    margin: 0;
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
  .body {
    margin: 22px 0 26px;
    font: 16px/1.6 var(--voice);
    overflow-wrap: anywhere;
  }
  .body :global(:first-child) {
    margin-top: 0;
  }
  .body :global(h1),
  .body :global(h2),
  .body :global(h3) {
    font-family: var(--ui);
    font-size: 15px;
    font-weight: 650;
    margin: 1.4em 0 0.4em;
  }
  .body :global(ul),
  .body :global(ol) {
    padding-left: 1.3em;
  }
  .body :global(code) {
    font: 0.88em var(--code);
  }
  .editor {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .editor .back {
    align-self: flex-start;
  }
  .editor .title {
    height: 46px;
    font: 500 21px var(--voice);
  }
  .fields {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }
  .fields label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--muted);
    font-size: 12px;
    font-weight: 600;
  }
  .fields input {
    height: 32px;
    font-size: 13.5px;
    color: var(--ink);
    font-weight: 400;
  }
  .editor textarea {
    min-height: 46vh;
    padding: 12px;
    resize: vertical;
    font: 15px/1.55 var(--ui);
  }
  .error {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
  .empty {
    margin: 28px 0 0;
    color: var(--muted);
  }
  @media (max-width: 560px) {
    .fields {
      grid-template-columns: 1fr;
    }
  }
</style>
