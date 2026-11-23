//! Voice, on this Mac: listening (speech to text, Whisper large-v3-turbo) and speaking (text to
//! speech, Kokoro v1.0, full precision), both run here by sherpa-onnx, nothing sent anywhere. The models are
//! downloaded once, on first use, into a folder of their own (`Voice::new`); until then
//! `status` says what is missing and `prepare` fetches it, reporting progress.
//!
//! Audio goes in and out as WAV: what the user said (any rate, mono or stereo) is turned into
//! the 16 kHz mono Whisper hears; what is spoken comes back at Kokoro's own rate.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use sherpa_rs::OnnxConfig;
use sherpa_rs::tts::{CommonTtsConfig, KokoroTts, KokoroTtsConfig};
use sherpa_rs::whisper::{WhisperConfig, WhisperRecognizer};

/// A model: what it is for, where it comes from, and the file that shows it is unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Listening,
    Speaking,
}

impl Model {
    pub const ALL: [Model; 2] = [Model::Listening, Model::Speaking];

    /// The folder its archive unpacks to.
    fn dir(self) -> &'static str {
        match self {
            Model::Listening => "sherpa-onnx-whisper-turbo",
            Model::Speaking => "kokoro-multi-lang-v1_0",
        }
    }

    fn url(self) -> String {
        let base = "https://github.com/k2-fsa/sherpa-onnx/releases/download";
        match self {
            Model::Listening => format!("{base}/asr-models/{}.tar.bz2", self.dir()),
            Model::Speaking => format!("{base}/tts-models/{}.tar.bz2", self.dir()),
        }
    }

    /// Written once the archive is wholly unpacked: a folder without it is a broken download.
    fn marker(self) -> &'static str {
        ".unpacked"
    }

    /// Roughly how much it downloads, for telling the user before they start.
    pub fn megabytes(self) -> u32 {
        match self {
            Model::Listening => 563,
            Model::Speaking => 349,
        }
    }
}

/// Where a model stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum State {
    Missing,
    /// Downloading or unpacking, this far (0–1).
    Fetching(f32),
    Ready,
}

/// Something said, as text.
#[derive(Debug, Clone, PartialEq)]
pub struct Heard {
    pub text: String,
    /// The language Whisper heard it in (`en`, `pt`, …).
    pub language: String,
}

/// Sound: samples (-1–1, mono) at `rate` per second.
#[derive(Debug, Clone, PartialEq)]
pub struct Audio {
    pub samples: Vec<f32>,
    pub rate: u32,
}

/// Listening and speaking, on models in `dir`. Each model is loaded on first use and kept.
pub struct Voice {
    dir: PathBuf,
    listening: Mutex<Option<WhisperRecognizer>>,
    /// Kokoro, as loaded for one language: switching reloads it (each copy is 330 MB).
    speaking: Mutex<Option<(String, KokoroTts)>>,
}

/// The downloads under way, by where they unpack to, and how far each is (0–1). One list for
/// the process: a second `Voice` on the same folder (the app makes one each time it reopens
/// its core) sees a download the first started, and doesn't start it again over the same file.
static FETCHING: Mutex<Vec<(PathBuf, f32)>> = Mutex::new(Vec::new());

/// How often a `prepare` that found a download under way looks whether it is done.
const FETCH_POLL: Duration = Duration::from_millis(250);

/// The Kokoro voice (a speaker of its v1.0 table) and the language setting for each language
/// it speaks: English from its own lexicon, the others through espeak. `None` for one it can't.
fn voice_for(language: &str) -> Option<(i32, &'static str)> {
    Some(match language {
        "en" | "" => (3, ""),  // af_heart
        "es" => (28, "es"),    // ef_dora
        "fr" => (30, "fr"),    // ff_siwis
        "it" => (35, "it"),    // if_sara
        "pt" => (42, "pt-br"), // pf_dora
        _ => return None,
    })
}

/// The rate Whisper hears at.
const LISTENING_RATE: u32 = 16_000;

/// A download claimed in `FETCHING`, given up when dropped.
struct Claim(PathBuf);

impl Drop for Claim {
    fn drop(&mut self) {
        FETCHING.lock().unwrap().retain(|(path, _)| *path != self.0);
    }
}

/// The most Whisper is given at once. It hears thirty seconds and drops the rest without a
/// word (sherpa-onnx's offline Whisper), so anything longer is heard in pieces.
const PIECE_SECONDS: usize = 28;
/// How far back from that a piece may end, to end in a pause instead of a word.
const PAUSE_SECONDS: usize = 8;

/// `samples` (at `rate`) in pieces Whisper hears whole, each cut at the quietest tenth of a
/// second near its end.
fn pieces(samples: &[f32], rate: usize) -> Vec<&[f32]> {
    let (most, back, frame) = (PIECE_SECONDS * rate, PAUSE_SECONDS * rate, rate / 10);
    let mut pieces = Vec::new();
    let mut rest = samples;
    while rest.len() > most {
        let loudness = |at: usize| rest[at..at + frame].iter().map(|s| s * s).sum::<f32>();
        let quietest = (most - back..=most - frame)
            .step_by(frame)
            .min_by(|a, b| loudness(*a).total_cmp(&loudness(*b)))
            .unwrap_or(most - frame);
        let (piece, after) = rest.split_at(quietest + frame / 2);
        pieces.push(piece);
        rest = after;
    }
    pieces.push(rest);
    pieces
}

impl Voice {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            listening: Mutex::new(None),
            speaking: Mutex::new(None),
        }
    }

    fn path(&self, model: Model) -> PathBuf {
        self.dir.join(model.dir())
    }

    pub fn state(&self, model: Model) -> State {
        let path = self.path(model);
        if let Some((_, done)) = FETCHING.lock().unwrap().iter().find(|(p, _)| *p == path) {
            return State::Fetching(*done);
        }
        match self.path(model).join(model.marker()).exists() {
            true => State::Ready,
            false => State::Missing,
        }
    }

    /// Downloads and unpacks what is missing, calling `progress` with each model and how far
    /// along it is (0–1), and returns when both are ready. Slow: minutes on a slow connection.
    /// A model someone else is already downloading is waited for, not downloaded twice.
    pub async fn prepare(&self, progress: impl Fn(Model, f32)) -> Result<()> {
        for model in Model::ALL {
            loop {
                match self.state(model) {
                    State::Ready => break,
                    State::Fetching(_) => tokio::time::sleep(FETCH_POLL).await,
                    // Lost to another caller between looking and claiming: look again.
                    State::Missing if !self.claim(model) => {}
                    State::Missing => {
                        // Released however this ends, also when the caller stops waiting.
                        let _claimed = Claim(self.path(model));
                        (self.fetch(model, &progress).await)
                            .with_context(|| format!("fetching the {model:?} model"))?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Marks `model` as being downloaded by this caller, unless it already is by another.
    fn claim(&self, model: Model) -> bool {
        let path = self.path(model);
        let mut fetching = FETCHING.lock().unwrap();
        if fetching.iter().any(|(p, _)| *p == path) {
            return false;
        }
        fetching.push((path, 0.0));
        true
    }

    async fn fetch(&self, model: Model, progress: &impl Fn(Model, f32)) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let archive = self.dir.join(format!("{}.tar.bz2.part", model.dir()));
        let mut response = reqwest::get(model.url()).await?.error_for_status()?;
        let total = response.content_length().unwrap_or(0) as f32;
        let mut file = tokio::fs::File::create(&archive).await?;
        let mut got = 0f32;
        while let Some(chunk) = response.chunk().await? {
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
            got += chunk.len() as f32;
            // Downloading is most of the work; unpacking the rest.
            let done = if total > 0.0 { 0.9 * got / total } else { 0.0 };
            self.set_progress(model, done);
            progress(model, done);
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        drop(file);
        let (dir, path) = (self.dir.clone(), archive.clone());
        tokio::task::spawn_blocking(move || unpack(&path, &dir)).await??;
        std::fs::remove_file(&archive)?;
        std::fs::write(self.path(model).join(model.marker()), "")?;
        progress(model, 1.0);
        tracing::info!(?model, megabytes = got / 1e6, "voice model ready");
        Ok(())
    }

    fn set_progress(&self, model: Model, done: f32) {
        let path = self.path(model);
        let mut fetching = FETCHING.lock().unwrap();
        if let Some(entry) = fetching.iter_mut().find(|(p, _)| *p == path) {
            entry.1 = done;
        }
    }

    /// Loads both models (speaking in English) if they aren't yet, so the first `listen` and
    /// `speak` don't wait seconds for it: call it as the user starts talking. Blocking.
    pub fn warm(&self) -> Result<()> {
        let mut listening = self.listening.lock().unwrap();
        if listening.is_none() {
            *listening = Some(self.load_listening()?);
        }
        drop(listening);
        let mut speaking = self.speaking.lock().unwrap();
        if speaking.is_none() {
            *speaking = Some((String::new(), self.load_speaking("")?));
        }
        Ok(())
    }

    /// What `wav` says. Blocking, and seconds of work: call it off the async runtime.
    pub fn listen(&self, wav: &[u8]) -> Result<Heard> {
        let audio = read_wav(wav)?;
        let samples = resample(&audio.samples, audio.rate, LISTENING_RATE);
        let mut listening = self.listening.lock().unwrap();
        if listening.is_none() {
            *listening = Some(self.load_listening()?);
        }
        let started = std::time::Instant::now();
        let whisper = listening.as_mut().unwrap();
        let mut said = Vec::new();
        // The language of the whole is the language of how it starts.
        let mut language = None;
        for piece in pieces(&samples, LISTENING_RATE as usize) {
            let result = whisper.transcribe(LISTENING_RATE, piece);
            language.get_or_insert_with(|| {
                (result.lang)
                    .trim_matches(|c| c == '<' || c == '>' || c == '|')
                    .to_string()
            });
            said.push(result.text.trim().to_string());
        }
        said.retain(|text| !text.is_empty());
        let (text, language) = (said.join(" "), language.unwrap_or_default());
        tracing::info!(
            seconds = samples.len() as f32 / LISTENING_RATE as f32,
            ms = started.elapsed().as_millis() as u64,
            language,
            text,
            "heard"
        );
        Ok(Heard { text, language })
    }

    /// Whether it can speak `language` (as Whisper names it: `en`, `es`, …).
    pub fn speaks(language: &str) -> bool {
        voice_for(language).is_some()
    }

    /// `text`, spoken in `language` (`en` when unknown), as WAV; an error for a language it
    /// can't speak (`speaks`). Blocking, like `listen`. Switching language reloads the model.
    pub fn speak(&self, text: &str, language: &str, speed: f32) -> Result<Vec<u8>> {
        let Some((voice, lang)) = voice_for(language) else {
            bail!("can't speak {language} yet");
        };
        let mut speaking = self.speaking.lock().unwrap();
        if speaking.as_ref().is_none_or(|(loaded, _)| loaded != lang) {
            *speaking = None;
            *speaking = Some((lang.to_string(), self.load_speaking(lang)?));
        }
        let started = std::time::Instant::now();
        let (_, kokoro) = speaking.as_mut().unwrap();
        let audio =
            (kokoro.create(text, voice, speed)).map_err(|error| anyhow!("speaking: {error}"))?;
        tracing::info!(
            chars = text.chars().count(),
            seconds = audio.samples.len() as f32 / audio.sample_rate as f32,
            ms = started.elapsed().as_millis() as u64,
            "spoke"
        );
        write_wav(&Audio {
            samples: audio.samples,
            rate: audio.sample_rate,
        })
    }

    fn load_listening(&self) -> Result<WhisperRecognizer> {
        let dir = self.ready(Model::Listening)?;
        let file = |name: &str| dir.join(name).to_string_lossy().into_owned();
        let config = WhisperConfig {
            encoder: file("turbo-encoder.int8.onnx"),
            decoder: file("turbo-decoder.int8.onnx"),
            tokens: file("turbo-tokens.txt"),
            // Empty: Whisper finds the language itself.
            language: String::new(),
            num_threads: Some(threads()),
            ..WhisperConfig::default()
        };
        WhisperRecognizer::new(config).map_err(|error| anyhow!("loading Whisper: {error}"))
    }

    fn load_speaking(&self, lang: &str) -> Result<KokoroTts> {
        let dir = self.ready(Model::Speaking)?;
        let file = |name: &str| dir.join(name).to_string_lossy().into_owned();
        let config = KokoroTtsConfig {
            // Full precision: on Apple Silicon int8 is five times slower to speak.
            model: file("model.onnx"),
            voices: file("voices.bin"),
            tokens: file("tokens.txt"),
            data_dir: file("espeak-ng-data"),
            dict_dir: file("dict"),
            lexicon: format!("{},{}", file("lexicon-us-en.txt"), file("lexicon-zh.txt")),
            length_scale: 1.0,
            onnx_config: OnnxConfig {
                provider: "cpu".into(),
                debug: false,
                num_threads: threads(),
            },
            common_config: CommonTtsConfig {
                max_num_sentences: 1,
                silence_scale: 0.2,
                ..CommonTtsConfig::default()
            },
            lang: lang.to_string(),
        };
        Ok(KokoroTts::new(config))
    }

    fn ready(&self, model: Model) -> Result<PathBuf> {
        match self.state(model) {
            State::Ready => Ok(self.path(model)),
            _ => bail!("the {model:?} model isn't downloaded yet"),
        }
    }
}

/// Threads for a model: most of this Mac's cores, leaving the rest for everything else.
fn threads() -> i32 {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cores.saturating_sub(2)).clamp(2, 10) as i32
}

fn unpack(archive: &Path, into: &Path) -> Result<()> {
    let file = std::fs::File::open(archive)?;
    tar::Archive::new(bzip2::read::BzDecoder::new(file)).unpack(into)?;
    Ok(())
}

/// A reply (Markdown) as it should be read out: code isn't read, a link is read as its text
/// and a bare address as "a link", and marks (emphasis, list bullets, headings) go.
pub fn speakable(markdown: &str) -> String {
    let mut out = Vec::new();
    let mut in_code = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || line.trim_start().starts_with('|') {
            continue;
        }
        let mut line = line.trim().trim_start_matches(['#', '>', ' ']).to_string();
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            line = rest.to_string();
        }
        out.push(read_inline(&line));
    }
    out.join("\n").trim().to_string()
}

/// One line's links, addresses, code and emphasis, as read.
fn read_inline(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while !rest.is_empty() {
        // [text](address) → text
        if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find("](")
            && let Some(end) = after[close + 2..].find(')')
        {
            out.push_str(&after[..close]);
            rest = &after[close + 2 + end + 1..];
            continue;
        }
        if rest.starts_with("http://") || rest.starts_with("https://") {
            out.push_str("a link");
            rest = &rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..];
            continue;
        }
        let c = rest.chars().next().unwrap();
        if !matches!(c, '*' | '_' | '`' | '~') {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

/// WAV bytes as mono samples, whatever their format.
pub fn read_wav(wav: &[u8]) -> Result<Audio> {
    let mut reader = hound::WavReader::new(Cursor::new(wav)).context("reading WAV")?;
    let spec = reader.spec();
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            (reader.samples::<i32>())
                .map(|s| s.map(|s| s as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    let channels = usize::from(spec.channels.max(1));
    let mono = samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok(Audio {
        samples: mono,
        rate: spec.sample_rate,
    })
}

/// Mono samples as 16-bit WAV bytes.
pub fn write_wav(audio: &Audio) -> Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: audio.rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut bytes, spec)?;
    for sample in &audio.samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    Ok(bytes.into_inner())
}

/// `samples` at `from` per second, at `to` instead: linear interpolation, which speech
/// recognition doesn't mind.
fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let len = (samples.len() as f64 / ratio) as usize;
    (0..len)
        .map(|i| {
            let at = i as f64 * ratio;
            let (j, frac) = (at as usize, (at.fract()) as f32);
            let a = samples[j];
            let b = *samples.get(j + 1).unwrap_or(&a);
            a + (b - a) * frac
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_goes_out_and_comes_back() {
        let audio = Audio {
            samples: (0..1600).map(|i| (i as f32 / 10.0).sin() * 0.5).collect(),
            rate: 16_000,
        };
        let back = read_wav(&write_wav(&audio).unwrap()).unwrap();
        assert_eq!(back.rate, 16_000);
        assert_eq!(back.samples.len(), 1600);
        assert!((back.samples[100] - audio.samples[100]).abs() < 0.001);
    }

    #[test]
    fn resampling_keeps_the_duration() {
        let one_second = vec![0.0; 24_000];
        assert_eq!(resample(&one_second, 24_000, 16_000).len(), 16_000);
        assert_eq!(resample(&one_second, 16_000, 16_000).len(), 24_000);
    }

    /// A long recording is heard in pieces of under thirty seconds, none of it left out, each
    /// ending in the pause nearest its end.
    #[test]
    fn a_long_recording_is_heard_in_pieces() {
        let rate = 100;
        // Seventy seconds of talk, silent in the 24th second and the 50th.
        let mut samples = vec![0.5; 70 * rate];
        samples[23 * rate..24 * rate].fill(0.0);
        samples[49 * rate..50 * rate].fill(0.0);
        let pieces = pieces(&samples, rate);
        let lengths: Vec<usize> = pieces.iter().map(|p| p.len()).collect();
        assert_eq!(lengths.iter().sum::<usize>(), samples.len());
        assert!(lengths.iter().all(|n| *n <= PIECE_SECONDS * rate));
        assert_eq!(lengths.len(), 3);
        // Each cut falls in a silence.
        assert!((23 * rate..24 * rate).contains(&lengths[0]));
        assert!((49 * rate..50 * rate).contains(&(lengths[0] + lengths[1])));
        assert_eq!(super::pieces(&samples[..10 * rate], rate).len(), 1);
    }

    /// A download under way is seen by another `Voice` on the same folder, which doesn't
    /// start it again.
    #[test]
    fn a_download_is_claimed_once_for_a_folder() {
        let dir = std::env::temp_dir().join("tokidoki-voice-claimed");
        let (one, other) = (Voice::new(&dir), Voice::new(&dir));
        assert!(one.claim(Model::Listening));
        let claimed = Claim(one.path(Model::Listening));
        assert_eq!(other.state(Model::Listening), State::Fetching(0.0));
        assert!(!other.claim(Model::Listening));
        drop(claimed);
        assert_eq!(other.state(Model::Listening), State::Missing);
    }

    #[test]
    fn a_missing_model_says_so() {
        let voice = Voice::new(std::env::temp_dir().join("tokidoki-voice-none"));
        assert_eq!(voice.state(Model::Listening), State::Missing);
        assert!(
            voice
                .listen(
                    &write_wav(&Audio {
                        samples: vec![0.0; 160],
                        rate: 16_000
                    })
                    .unwrap()
                )
                .is_err()
        );
    }

    /// Kokoro says a sentence and Whisper hears it back, on the models in
    /// `TOKIDOKI_VOICE_MODELS` (downloaded there if missing), timing both.
    #[test]
    #[ignore = "needs the voice models (~700 MB)"]
    fn what_is_spoken_is_heard() {
        let dir = std::env::var("TOKIDOKI_VOICE_MODELS").expect("set TOKIDOKI_VOICE_MODELS");
        let voice = Voice::new(dir);
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(voice.prepare(|model, done| eprintln!("{model:?} {:.0}%", done * 100.0)))
            .unwrap();
        let said = "Remind me to call my sister when I get home tonight.";
        for round in 1..=2 {
            let started = std::time::Instant::now();
            let wav = voice.speak(said, "en", 1.0).unwrap();
            let spoke = started.elapsed();
            let audio = read_wav(&wav).unwrap();
            let started = std::time::Instant::now();
            let heard = voice.listen(&wav).unwrap();
            println!(
                "round {round}: spoke {:.1}s of audio in {spoke:?}; heard {:?} ({}) in {:?}",
                audio.samples.len() as f32 / audio.rate as f32,
                heard.text,
                heard.language,
                started.elapsed()
            );
            let words = |s: &str| {
                s.to_lowercase()
                    .replace(|c: char| !c.is_alphanumeric() && c != ' ', "")
            };
            assert_eq!(words(&heard.text), words(said));
            assert_eq!(heard.language, "en");
        }
    }
}

#[cfg(test)]
mod languages {
    use super::*;

    /// Spanish and French come back as they were said, in that language; a language it
    /// can't speak is refused rather than read in the wrong accent.
    #[test]
    #[ignore = "needs the voice models (~900 MB)"]
    fn other_languages_are_spoken_in_their_own_voice() {
        let voice =
            Voice::new(std::env::var("TOKIDOKI_VOICE_MODELS").expect("set TOKIDOKI_VOICE_MODELS"));
        for (language, said) in [
            ("es", "Mañana voy a comprar pan y leche en el mercado."),
            ("fr", "Je voudrais réserver une table pour deux ce soir."),
        ] {
            let heard = voice
                .listen(&voice.speak(said, language, 1.0).unwrap())
                .unwrap();
            println!(
                "{language}: {said:?} -> {:?} ({})",
                heard.text, heard.language
            );
            assert_eq!(heard.language, language);
            assert_eq!(heard.text, said);
        }
        assert!(!Voice::speaks("de"));
        assert!(voice.speak("Guten Tag", "de", 1.0).is_err());
    }
}

#[cfg(test)]
mod reading {
    use super::*;

    #[test]
    fn a_reply_is_read_without_its_marks() {
        let reply = "## Plan\n\n- **Book** the [ryokan](https://example.com/a) early\n- See https://x.y/z first\n\n```sh\nrm -rf /\n```\nThat's `it`.";
        assert_eq!(
            speakable(reply),
            "Plan\n\nBook the ryokan early\nSee a link first\n\nThat's it."
        );
    }
}
