//! Voice, for whatever embeds tokidoki: what the models' state looks like in the protocol,
//! and the blocking speech work moved off the async runtime.

use std::sync::Arc;

use anyhow::Result;
use interface::{Heard, ModelState, VoiceStatus};
use voice::{Model, State, Voice};

pub(crate) fn status(voice: &Voice) -> VoiceStatus {
    let state = |model: Model| match voice.state(model) {
        State::Missing => ModelState::Missing {
            megabytes: model.megabytes(),
        },
        State::Fetching(done) => ModelState::Fetching { done },
        State::Ready => ModelState::Ready,
    };
    VoiceStatus {
        listening: state(Model::Listening),
        speaking: state(Model::Speaking),
    }
}

/// What `wav` says.
pub(crate) async fn listen(voice: Arc<Voice>, wav: Vec<u8>) -> Result<Heard> {
    let heard = tokio::task::spawn_blocking(move || voice.listen(&wav)).await??;
    Ok(Heard {
        speakable: Voice::speaks(&heard.language),
        text: heard.text,
        language: heard.language,
    })
}

/// A reply, read out in `language`, as WAV.
pub(crate) async fn speak(voice: Arc<Voice>, reply: String, language: String) -> Result<Vec<u8>> {
    let text = voice::speakable(&reply);
    tokio::task::spawn_blocking(move || voice.speak(&text, &language, 1.0)).await?
}
