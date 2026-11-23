// A first launch with no key: the provider form, then switching provider on it.
await t.see("form.keys");
await t.see(".provider");
await t.check("it asks for a key first", t.text().includes("Tokidoki runs on your own key"));
await t.check("Anthropic is chosen to begin with", document.querySelector(".provider.on")?.textContent.includes("Anthropic"));
await t.shot("first-run-1-anthropic");
await t.click("Fireworks");
await t.sleep(100);
await t.check("the key field follows the provider", t.text().includes("Fireworks API key"));
await t.type("input[type=password]", "not-a-real-key");
await t.shot("first-run-2-fireworks");
await t.click("Save and start");
await t.until("the key to be refused", () => document.querySelector(".error"), 30000);
await t.check("a wrong key is refused in words", t.text().includes("didn't accept that key"));
await t.shot("first-run-3-refused");
