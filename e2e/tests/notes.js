// Notes with no key: written, read, searched, filtered by tag, changed and deleted, all in the
// app (no model needed). One note comes from the core directly, as if Tokidoki had saved it.
const invoke = window.__TAURI_INTERNALS__.invoke;
const draft = (title, body, tags = []) => ({ title, body, tags, project: null, people: [] });
await t.see("form.keys");
await invoke("write_note", { id: null, draft: draft("Lisbon trip", "Flights on the **3rd**.\n\n- book the hotel\n- ask Ana", ["travel"]) });
await t.click("button.notes");
await t.see("New note");
await t.check("a note made elsewhere is listed", t.text().includes("Lisbon trip"));

await t.click("New note");
await t.type("textarea", "Groceries\n\n- oat milk\n- lentils");
await t.type(".fields label:first-child input", "shopping, weekly");
await t.shot("notes-1-writing");
await t.click("button[type=submit]");
await t.see("article");
await t.check("an untitled note takes its first line", document.querySelector("article h1")?.textContent === "Groceries");
await t.check("its body is shown as Markdown", document.querySelectorAll("article .body li").length === 2);
await t.shot("notes-2-reading");

await t.click("← All notes");
await t.see("input.search");
await t.check("both notes are listed, newest first", [...document.querySelectorAll("li .name")].map((e) => e.textContent).join() === "Groceries,Lisbon trip");
await t.shot("notes-3-list");
await t.type("input.search", "lentils");
await t.until("the search to narrow the list", () => document.querySelectorAll("li .name").length === 1);
await t.check("search finds words in the body", document.querySelector("li .name")?.textContent === "Groceries");
await t.type("input.search", "");
await t.click("button.tag:not(.on)");
await t.until("the tag to filter", () => document.querySelector("button.tag.on"));
await t.check("a tag shows only its notes", document.querySelectorAll("li .name").length === 1);
await t.click("button.tag.on");

await t.click("Lisbon trip");
await t.click("Edit");
await t.type("input.title", "Lisbon, in March");
await t.click("button[type=submit]");
await t.until("the change to show", () => document.querySelector("article h1")?.textContent === "Lisbon, in March");
await t.check("a change keeps the note", (await invoke("notes", { search: { query: "", tag: null, project: null } })).length === 2);
await t.click("Delete…");
await t.click("article .danger");
await t.see("input.search");
await t.check("a deleted note is gone", !t.text().includes("Lisbon"));
await t.shot("notes-4-after");
