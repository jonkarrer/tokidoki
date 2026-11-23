// Push-to-talk recording: the microphone, as 16 kHz mono 16-bit WAV (what the core's Whisper
// hears; sending it at that rate keeps the bytes few). After Lo's recorder.

const RATE = 16_000;

export interface Recording {
  /** Stops and returns the WAV, or null when nothing worth hearing was recorded. */
  stop(): Promise<Uint8Array | null>;
  /** Stops and throws it away. */
  cancel(): void;
}

/** Starts recording; `onLevel` hears the input level (0–1) about twenty times a second. */
export async function record(onLevel?: (level: number) => void): Promise<Recording> {
  const stream = await navigator.mediaDevices.getUserMedia({
    audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true },
  });
  const audio = new AudioContext();
  const source = audio.createMediaStreamSource(stream);
  // Deprecated, but it needs no separate worklet module, and WebKit has it.
  const processor = audio.createScriptProcessor(4096, 1, 1);
  const chunks: Float32Array[] = [];
  processor.onaudioprocess = (event) => {
    const input = event.inputBuffer.getChannelData(0);
    chunks.push(new Float32Array(input));
    onLevel?.(level(input));
  };
  source.connect(processor);
  processor.connect(audio.destination);

  const release = () => {
    processor.disconnect();
    source.disconnect();
    for (const track of stream.getTracks()) track.stop();
    void audio.close();
  };

  return {
    async stop() {
      release();
      const length = chunks.reduce((n, chunk) => n + chunk.length, 0);
      // Under a third of a second: a click, not something said.
      if (length < audio.sampleRate * 0.3) return null;
      const all = new Float32Array(length);
      let at = 0;
      for (const chunk of chunks) {
        all.set(chunk, at);
        at += chunk.length;
      }
      return wav(downsample(all, audio.sampleRate, RATE), RATE);
    },
    cancel: release,
  };
}

/** How loud `samples` are, 0–1 (root mean square, scaled so speech reaches most of it). */
function level(samples: Float32Array) {
  let sum = 0;
  for (const s of samples) sum += s * s;
  return Math.min(1, Math.sqrt(sum / samples.length) * 6);
}

/** `samples` at `from` per second, at `to` instead, averaging what each new sample covers. */
function downsample(samples: Float32Array, from: number, to: number): Float32Array {
  if (from === to) return samples;
  const ratio = from / to;
  const out = new Float32Array(Math.floor(samples.length / ratio));
  for (let i = 0; i < out.length; i++) {
    const start = Math.floor(i * ratio);
    const end = Math.min(samples.length, Math.floor((i + 1) * ratio));
    let sum = 0;
    for (let j = start; j < end; j++) sum += samples[j];
    out[i] = sum / Math.max(1, end - start);
  }
  return out;
}

/** Mono samples as 16-bit WAV bytes. */
function wav(samples: Float32Array, rate: number): Uint8Array {
  const bytes = new Uint8Array(44 + samples.length * 2);
  const view = new DataView(bytes.buffer);
  const text = (at: number, s: string) => [...s].forEach((c, i) => view.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  view.setUint32(4, 36 + samples.length * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true); // PCM
  view.setUint16(22, 1, true); // mono
  view.setUint32(24, rate, true);
  view.setUint32(28, rate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, samples.length * 2, true);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    view.setInt16(44 + i * 2, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  return bytes;
}
