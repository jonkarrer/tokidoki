// The memory view with no key: memory can still be read and edited while asleep. Two facts
// are told outright (no model needed), then everything is forgotten at once.
const invoke = window.__TAURI_INTERNALS__.invoke;
await t.see("form.keys");
await invoke("remember", { text: "I'm vegetarian" });
await invoke("remember", { text: "I live in Lisbon" });
await t.click("button.memory");
await t.see("Forget everything…");
await t.check("both memories are listed", t.text().includes("I'm vegetarian") && t.text().includes("I live in Lisbon"));
await t.shot("memory-1-listed");
await t.click("Forget everything…");
await t.see(".wipe .danger");
await t.check("it asks first, saying how many", t.text().includes("Forget all 2 memories for good?"));
await t.shot("memory-2-asking");
await t.click("Keep them");
await t.check("keeping them changes nothing", t.text().includes("I'm vegetarian"));
await t.click("Forget everything…");
await t.click(".wipe .danger");
// With no key and no conversations, nothing is left to show: it is a first launch again.
await t.see("form.keys");
await t.check("nothing is left", (await invoke("memories")).length === 0);
await t.check("with nothing left and no key, it asks for one again", t.text().includes("Tokidoki runs on your own key"));
