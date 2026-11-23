// Talking to Tokidoki, after Lo: push-to-talk (the mic button, or ⌥Space, to start and stop),
// and what was said is sent as a message. A reply to something said is read out; to something
// typed, it is only shown. Starting to talk, or Esc, stops the reading; any reply can be read
// again. Listening and speaking both run in the core, on this Mac (crates/voice).
import { core, type VoiceStatus } from "./core";
import { record, type Recording } from "./recorder";
import { send, whenReplied } from "./state.svelte";

export const voice = $state({
  /** Where the models stand; `null` when there is no voice at all. */
  status: null as VoiceStatus | null,
  /** The one-time download is under way. */
  preparing: false,
  /** Asked to talk before the models were there: the download offer is showing. */
  offering: false,
  recording: false,
  /** How loud the microphone is now, 0–1. */
  level: 0,
  /** Turning what was said into text. */
  hearing: false,
  /** Answer out loud: the last message was spoken, in a language Tokidoki can speak. */
  aloud: false,
  /** The language of the last thing said, which replies are read out in. */
  language: "en",
  /** The reply being read out (its line's key). */
  playing: null as string | null,
  /** Something said that couldn't be sent: the composer takes it, so it isn't lost. */
  unsent: null as string | null,
  error: null as string | null,
});

let recording: Recording | null = null;
/** The microphone was asked for and hasn't answered yet; `abandoned` when that was called off. */
let starting = false;
let abandoned = false;
let started = false;

/** Reads where the models stand, and follows it; reads replies out when talked to. */
export async function startVoice() {
  if (started) return;
  started = true;
  await core.onEvent((event) => {
    if (event.type === "voice") voice.status = event.status;
  });
  whenReplied((key, text) => {
    // Not over the user talking: the microphone would hear it as theirs.
    if (!voice.aloud || starting || voice.recording || voice.hearing) return;
    // One turn can bring several replies: each is read after the one before, not over it.
    if (voice.playing !== null) waiting.push({ key, text });
    else read(key, text);
  });
  voice.status = await core.voiceStatus().catch(() => null);
}

export function ready() {
  return voice.status?.listening.state === "ready" && voice.status?.speaking.state === "ready";
}

/** Megabytes still to download, when something is missing. */
export function toDownload() {
  const parts = [voice.status?.listening, voice.status?.speaking];
  return parts.reduce((sum, part) => sum + (part?.state === "missing" ? part.megabytes : 0), 0);
}

/** How far the download is, 0–1, across both models. */
export function downloaded() {
  const part = (state: VoiceStatus["listening"] | undefined) =>
    state?.state === "ready" ? 1 : state?.state === "fetching" ? state.done : 0;
  return (part(voice.status?.listening) + part(voice.status?.speaking)) / 2;
}

/** Downloads the models (once); when done, the talking can start. */
export async function prepare() {
  voice.preparing = true;
  voice.error = null;
  try {
    await core.prepareVoice();
    voice.offering = false;
  } catch (problem) {
    voice.error = String(problem);
  } finally {
    voice.preparing = false;
    voice.status = await core.voiceStatus().catch(() => voice.status);
  }
}

/** Starts talking, or stops and sends what was said. */
export async function toggleTalking() {
  if (voice.recording) return stopTalking();
  if (voice.hearing || starting) return;
  if (!ready()) {
    voice.offering = true;
    return;
  }
  stopSpeaking();
  voice.error = null;
  // Loads the models while the user talks, so hearing them doesn't wait for it.
  core.warmVoice().catch(() => {});
  starting = true;
  abandoned = false;
  try {
    const begun = await record((level) => (voice.level = level));
    // Called off while the microphone was being asked for.
    if (abandoned) return begun.cancel();
    recording = begun;
    voice.recording = true;
  } catch (problem) {
    voice.error = `The microphone isn't available: ${problem}`;
  } finally {
    starting = false;
  }
}

async function stopTalking() {
  const ended = recording;
  recording = null;
  voice.recording = false;
  voice.level = 0;
  const wav = await ended?.stop();
  if (!wav) return;
  voice.hearing = true;
  try {
    const heard = await core.listen(wav);
    if (!heard.text.trim()) return;
    // Replies are read in the language last said that it can speak.
    if (heard.speakable) voice.language = heard.language;
    voice.aloud = heard.speakable;
    if (!(await send(heard.text))) {
      voice.aloud = false;
      voice.unsent = heard.text;
    }
  } catch (problem) {
    voice.error = String(problem);
  } finally {
    voice.hearing = false;
  }
}

/** Stops recording without sending, also when it was only about to start. */
export function cancelTalking() {
  if (starting) abandoned = true;
  recording?.cancel();
  recording = null;
  voice.recording = false;
  voice.level = 0;
}

/** Something was typed: replies go back to being only shown. */
export function typed() {
  voice.aloud = false;
}

/** Esc: calls off a recording, or else the reading out. */
export function hush() {
  if (voice.recording || starting) cancelTalking();
  else stopSpeaking();
}

let playing: { token: number; audio: HTMLAudioElement | null } = { token: 0, audio: null };
/** Replies to read once the one being read is done. */
let waiting: { key: string; text: string }[] = [];

/** Reads `text` (a reply, Markdown) out now, in place of whatever was being read. */
export function speak(key: string, text: string) {
  stopSpeaking();
  return read(key, text);
}

/** `text` in the parts it is read in: its paragraphs, a code block whole however many blank
    lines it has (the core leaves code out, and has to see where it starts and ends). */
export function paragraphs(text: string): string[] {
  const parts: string[] = [];
  let part: string[] = [];
  let code = false;
  for (const line of text.split("\n")) {
    if (line.trimStart().startsWith("```")) code = !code;
    if (line.trim() || code) part.push(line);
    else if (part.length) parts.push(part.splice(0).join("\n"));
  }
  if (part.length) parts.push(part.join("\n"));
  return parts;
}

/** Reads `text` out, paragraph by paragraph: the next is made while the one before plays, so
    a long reply starts as soon as its first paragraph is ready. */
async function read(key: string, text: string) {
  const token = ++playing.token;
  voice.playing = key;
  const parts = paragraphs(text);
  const make = (part: string) =>
    core.speak(part, voice.language).catch((problem) => {
      if (token === playing.token) voice.error = String(problem);
      return null;
    });
  try {
    let next = parts.length ? make(parts[0]) : null;
    for (let i = 0; i < parts.length; i++) {
      const wav = await next;
      if (token !== playing.token) return;
      if (i + 1 < parts.length) next = make(parts[i + 1]);
      if (wav && wav.byteLength > 44) await play(wav, token);
    }
  } finally {
    if (token === playing.token) {
      voice.playing = null;
      const then = waiting.shift();
      if (then) read(then.key, then.text);
    }
  }
}

function play(wav: ArrayBuffer, token: number) {
  return new Promise<void>((done) => {
    if (token !== playing.token) return done();
    const url = URL.createObjectURL(new Blob([wav], { type: "audio/wav" }));
    const audio = new Audio(url);
    playing.audio = audio;
    const end = () => {
      URL.revokeObjectURL(url);
      done();
    };
    audio.onended = end;
    audio.onerror = end;
    audio.onpause = end;
    audio.play().catch(end);
  });
}

/** Stops reading out. */
export function stopSpeaking() {
  waiting = [];
  playing.token++;
  playing.audio?.pause();
  playing.audio = null;
  voice.playing = null;
}
