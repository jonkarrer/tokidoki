// With a model key: a job is handed to a worker on a mission. Tokidoki says it's on it, an
// "On a mission" line shows while the worker is out, its report comes back as a card, and
// Missions keeps what came of it with every step the worker took. The job here is not what a
// mission is for (reading a note is the conversation's own to do): it is asked for outright,
// as the one job a mission can do with no browser and no key but the model's. Calls a model (the cheapest).
const invoke = window.__TAURI_INTERNALS__.invoke;
await t.see(".composer textarea");
await invoke("write_note", {
  id: null,
  draft: { title: "Grocery list", body: "- oat milk\n- eggs\n- sourdough\n- tomatoes", tags: ["shopping"], project: null, people: [] },
});
await t.type(".composer textarea", "Send a worker on a mission: find my grocery list in my notes and report how many items are on it and what they are.");
await t.click(".composer .send");
await t.until("the mission to start", () => document.querySelector(".looking"), 60000);
await t.check("it says it's on a mission", /On a mission: /.test(document.querySelector(".looking").textContent));
await t.shot("missions-1-out");
await t.until("the report to come back", () => document.querySelector(".report"), 180000);
await t.until("the answer after it", () => !document.querySelector(".composer .stop") && document.querySelectorAll(".reply").length >= 2, 90000);
await t.check("the card says a mission came back", document.querySelector(".report summary").textContent.includes("A mission came back"));
await t.shot("missions-2-answered");
await t.click("nav .missions");
await t.see("section li button");
await t.check("Missions lists it", document.querySelectorAll("section li button").length === 1);
await t.shot("missions-3-list");
await t.click("section li button");
await t.until("its trace", () => document.querySelector(".trace .use"), 10000);
await t.check("the trace names the tool it used", /search_notes|get_note/.test(document.querySelector(".trace").textContent));
await t.check("with what it was given", document.querySelector(".trace dl dd")?.textContent.trim().length > 0);
await t.check("and its report after", /4|four/i.test(document.querySelector("article .reply").textContent));
await t.shot("missions-4-trace");
await t.click(".trace .result summary");
await t.sleep(200);
await t.check("a result opens to all that came back", /oat milk|Grocery list/i.test(document.querySelector(".trace pre")?.textContent ?? ""));
await t.shot("missions-5-step");
