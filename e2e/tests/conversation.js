// With a key: a message is answered as it is written, the thread is listed, and what was
// worth keeping shows up in memory. Calls a model (the provider's cheapest).
await t.see(".composer textarea");
await t.check("it opens on a new conversation", !t.text().includes("runs on your own key"));
await t.shot("conversation-1-empty");
await t.type(".composer textarea", "I'm vegetarian and I live in Lisbon. What should I cook tonight? Two short ideas.");
await t.click(".composer .send");
await t.until("a reply to start", () => document.querySelector(".reply")?.textContent.trim(), 60000);
await t.shot("conversation-2-writing");
await t.until("the answer to end", () => !document.querySelector(".composer .stop") && document.querySelector(".reply"), 90000);
await t.check("the reply has words", document.querySelector(".reply").textContent.trim().length > 20);
await t.until("the thread to be listed", () => document.querySelector("nav li button"), 10000);
await t.shot("conversation-3-answered");
// What it learned arrives once the turn has settled.
let learned = true;
try {
  await t.until("a memory", () => /\d+ new/.test(document.querySelector("nav .memory")?.textContent ?? ""), 60000);
} catch {
  learned = false;
}
await t.check("it learned something from the message", learned);
await t.shot("conversation-4-learned");
await t.click("nav .memory");
await t.see("section h1");
await t.shot("conversation-5-memory");
await t.click("Model and key");
await t.see(".provider");
await t.shot("conversation-6-settings");
