// The voice, on models already downloaded (voice.env), with no key: Tokidoki speaks a sentence
// and hears it back through the app (bytes both ways, as the microphone's recording goes), and
// the mic is in the composer.
const invoke = window.__TAURI_INTERNALS__.invoke;
await t.see("form.keys");
const status = await invoke("voice_status");
await t.check("both models are ready", status?.listening.state === "ready" && status?.speaking.state === "ready");
const said = "Remind me to water the plants on Sunday morning.";
let started = Date.now();
const wav = await invoke("speak", { text: said, language: "en" });
const spoke = Date.now() - started;
await t.check("it speaks: WAV bytes come back", wav instanceof ArrayBuffer && new TextDecoder().decode(wav.slice(0, 4)) === "RIFF");
started = Date.now();
const heard = await invoke("listen", new Uint8Array(wav));
await t.log(`spoke in ${spoke} ms, heard ${JSON.stringify(heard)} in ${Date.now() - started} ms`);
await t.check("it hears what was said", heard.text.replace(/[^a-z ]/gi, "").toLowerCase() === said.replace(/[^a-z ]/gi, "").toLowerCase());
await t.check("in English, which it can answer in", heard.language === "en" && heard.speakable);
const refused = await invoke("speak", { text: "Guten Tag", language: "de" }).then(() => null, (e) => String(e));
await t.check("a language it can't speak is refused in words", /can't speak de/.test(refused ?? ""));
await invoke("remember", { text: "I'm vegetarian" });
await t.click("nav .conversation");
await t.see(".composer .mic");
await t.check("the mic is in the composer", document.querySelector(".composer .mic").title.includes("⌥Space"));
await t.shot("voice-1-composer");
