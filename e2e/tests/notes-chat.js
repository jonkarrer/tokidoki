// With a key: asked in a conversation, Tokidoki saves a note (it shows in Notes, linked to
// the conversation) and later answers from it. Calls a model (the provider's cheapest).
const ask = async (text) => {
  // Done when a new reply is in and nothing is being answered: earlier replies don't count.
  const replies = document.querySelectorAll(".reply").length;
  await t.type(".composer textarea", text);
  await t.click(".composer .send");
  await t.until("the answer to end", () => !document.querySelector(".composer .stop") && document.querySelectorAll(".reply").length > replies, 90000);
};
await t.see(".composer textarea");
await ask("Save these notes: plumber coming Tuesday 9am, fix kitchen sink, costs 80 euros, call Marco if late");
await t.until("the note to be listed", () => /[1-9]/.test(document.querySelector("nav .notes .count")?.textContent ?? ""), 30000);
await t.shot("notes-chat-1-saved");
await t.click("nav .notes");
await t.see("li .name");
await t.check("one note was saved", document.querySelectorAll("li .name").length === 1);
await t.check("it kept the details", /tuesday/i.test(t.text()) && /80/.test(t.text()) && /marco/i.test(t.text()));
await t.click("li button");
await t.see("article");
await t.check("it links to the thread it was saved in", document.querySelector("article .link"));
await t.shot("notes-chat-2-note");
await t.click("nav .conversation");
await ask("Check my notes: what time is the plumber coming?");
const answer = [...document.querySelectorAll(".reply")].at(-1).textContent;
await t.check("it answers from the note", /9/.test(answer));
await t.shot("notes-chat-3-answered");
