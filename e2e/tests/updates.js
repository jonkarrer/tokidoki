// Checking for updates by hand, from the settings and from the version in the sidebar. A
// development build doesn't update itself, and says so in both places.
const invoke = window.__TAURI_INTERNALS__.invoke;
await t.see("form.keys");
// Something to show, so the window opens on more than the first-launch form.
await invoke("remember", { text: "I live in Lisbon" });
await t.click("button.memory");
await t.click("Model and key");
await t.see(".update");
await t.until("the settings to say which version this is", () => /Version \d+\.\d+\.\d+/.test(t.text()), 5000);
await t.check("and how updates arrive", t.text().includes("checks for updates by itself"));
await t.shot("updates-1-settings");
await t.click("Check for updates");
await t.until("the answer", () => t.text().includes("Development builds don't update themselves"), 10000);
await t.see("nav .card");
await t.check("the sidebar says it too", document.querySelector("nav .card").textContent.includes("didn't go through"));
await t.shot("updates-2-answered");
await t.click("Close");
await t.check("the card closes", !document.querySelector("nav .card"));
