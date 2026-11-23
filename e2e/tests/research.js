// With keys (a model, Jev and Exa): research is handed to a worker. Tokidoki says it's on it,
// a "Looking into" line shows while the worker searches, then its report comes back as a card
// (with its sources as links) and Tokidoki tells what it found. Calls a model (the cheapest).
await t.see(".composer textarea");
await t.type(".composer textarea", "Could you look into what the current population of Lisbon is, with a source?");
await t.click(".composer .send");
await t.until("the worker to start", () => document.querySelector(".looking"), 60000);
await t.check("it says what it's looking into", /Looking into: .*Lisbon/i.test(document.querySelector(".looking").textContent));
await t.shot("research-1-looking");
await t.until("the report to come back", () => document.querySelector(".report"), 180000);
await t.until("the answer after it", () => !document.querySelector(".composer .stop") && document.querySelectorAll(".reply").length >= 2, 90000);
await t.check("nothing is still being looked into", !document.querySelector(".looking"));
await t.check("the report says it came back", document.querySelector(".report summary").textContent.includes("Research came back"));
await t.check("Tokidoki tells what it found", /\d{3}[,.\s]?\d{3}/.test([...document.querySelectorAll(".reply")].at(-1).textContent));
await t.shot("research-2-answered");
await t.click(".report summary");
await t.sleep(200);
await t.check("the report cites its sources as links", document.querySelectorAll(".report a[href^='http']").length >= 1);
await t.shot("research-3-report");
