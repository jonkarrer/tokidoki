// With a key: one conversation, filed into threads under the hood. Two subjects become two
// threads, both shown as one conversation with a gap where the subject changed; coming back
// to the first goes back to its thread. Threads shows them. Calls a model (the cheapest).
const ask = async (text) => {
  await t.type(".composer textarea", text);
  await t.click(".composer .send");
  await t.until("the answer to end", () => !document.querySelector(".composer .stop") && document.querySelectorAll(".reply").length, 90000);
  await t.sleep(300);
};
const threadOf = (said) => [...document.querySelectorAll(".said")].find((e) => e.textContent.includes(said))?.dataset.thread;
await t.see(".composer textarea");
await ask("I'm planning a trip to Kyoto in April. Which neighbourhood should I stay in? One line.");
await ask("Which of those is quietest at night? One line.");
await ask("New topic: what's a good beginner sourdough hydration? One line.");
await ask("Back to the Kyoto trip: is April busy there? One line.");
await t.shot("one-conversation-1");
const kyoto = threadOf("planning a trip to Kyoto");
await t.check("a follow-up stays in its thread", threadOf("quietest at night") === kyoto);
await t.check("a new subject gets its own thread", threadOf("sourdough") && threadOf("sourdough") !== kyoto);
await t.check("coming back to a subject goes back to its thread", threadOf("Back to the Kyoto trip") === kyoto);
await t.check("a change of subject leaves a gap", document.querySelectorAll(".said.moved").length === 2);
await t.check("every message is in the one conversation", document.querySelectorAll(".said").length === 4);
await t.click("Threads");
await t.see(".list li");
await t.check("Threads lists both", document.querySelectorAll(".list li").length === 2);
await t.check("both are hot", document.querySelectorAll(".list .phase.hot").length === 2);
await t.shot("one-conversation-2-threads");
await t.click(".list li button");
await t.see(".history .said");
await t.check("a thread shows its own history", document.querySelectorAll(".history .said").length >= 1);
await t.shot("one-conversation-3-thread");
