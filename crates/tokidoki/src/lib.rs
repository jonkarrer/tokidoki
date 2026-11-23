//! The whole system as one library: input in, output out.
//!
//! `Tokidoki::open` takes a `Config` (`Config::new`; `Config::from_env` for `chat`) and wires the state
//! database, memory, the router, Experience, the engine and its agent tasks, and the journal.
//! It resumes whatever a previous process left unfinished. Whatever embeds it calls its
//! methods with `interface` types and gets `interface` types back: `send` an input, follow
//! `events` (or `wait`), read `threads` and `history`, `abort`, give `feedback`; and see what
//! it remembers (`memories`), tell it something (`remember`), correct it (`revise`), have it
//! `forget` one thing or `wipe` all of it; and keep the user's `notes` (`write_note`,
//! `delete_note`), which the model saves and reads too, when asked. Where inputs come from
//! and where outputs go is the embedding code's business, not tokidoki's.
//!
//! Who may speak is decided here: every input names its `Speaker`, and only the configured
//! speakers are let in, all as the one user whose memory scope this instance reads and
//! writes. (Several users, each with their own scope, is a later step.)

mod events;
mod note_tools;
mod speech;
mod worker_tools;

use std::path::PathBuf;
use std::sync::Arc;

use agent::{Agent, Live, Mind, Model, ProviderModel, ThreadSetup};
use anyhow::{Context as _, Result};
use engine::{Engine, Registry};
use experience::{Experience, Scopes};
use inference::providers::AnyApi;
use inference::{Provider, Request};
use interface::{Feedback, Input, Rejected, Sent, Speaker, Submission, ThreadPhase, ThreadSummary};
use memory::{Body, Memory, Scope, Write};
use notes::Notes;
use router::Router;
use state::{Checkpointer, Sqlite};
use tokio::sync::mpsc;
use tracing::Instrument;

// Everything an embedding program names, so this crate is the only one it depends on.
pub use agent::{Settings, Tool, ToolFuture, Tools};
pub use inference::{Effort, Vendor};
pub use interface;

/// Events as they happen, until dropped.
pub type Events = mpsc::Receiver<interface::Event>;

pub const DEFAULT_SYSTEM: &str = "You are tokidoki, a personal assistant. Memory is automatic: \
what matters is remembered across conversations without any tool, and memories relevant to \
the conversation are added to it for you. Be direct and concise.";

#[derive(Clone)]
pub struct Config {
    /// Holds `tokidoki.db` (state), `tokidoki.journal.db`, `memory.db` and `notes.db`.
    pub data: PathBuf,
    /// Whose memory this is: the scope every speaker reads and writes.
    pub user: String,
    /// The `Speaker` IDs let in.
    pub speakers: Vec<String>,
    /// What every new thread starts with.
    pub system: Option<String>,
    /// Whose models it runs on, all of them, and the user's key for it.
    pub provider: Vendor,
    pub key: String,
    /// Answers, and Experience's memory work.
    pub model: String,
    pub effort: Effort,
    /// The router's fast model, its last resort.
    pub router_model: String,
    /// The user's TypeSafe key, for Jev: when there is one, Jev makes the quick decisions:
    /// where an input goes (`router::JevClassifier`), and what to recall and keep for it
    /// (`Experience::jev`).
    pub jev_key: Option<String>,
    /// Which Jev; pinned, since its confidence thresholds are tuned to it.
    pub jev_model: String,
    /// The user's Exa key, for research workers: with one, the model can hand research to a
    /// worker that searches the web (`delegate`).
    pub exa_key: Option<String>,
    /// Where the voice models are kept, downloaded on first use (`prepare_voice`): with a
    /// folder, tokidoki can listen and speak (`listen`, `speak`), all on this Mac.
    pub voice_models: Option<PathBuf>,
    pub agent: agent::Settings,
    /// Record to the journal (`telemetry`), for `just inspect`.
    pub journal: bool,
}

/// Never the key: a `{config:?}` anywhere would otherwise put it in the journal.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("data", &self.data)
            .field("user", &self.user)
            .field("provider", &self.provider)
            .field(
                "key",
                &if self.key.is_empty() {
                    "<empty>"
                } else {
                    "<set>"
                },
            )
            .field("model", &self.model)
            .field("router_model", &self.router_model)
            .field("jev_key", &self.jev_key.as_ref().map(|_| "<set>"))
            .field("jev_model", &self.jev_model)
            .field("exa_key", &self.exa_key.as_ref().map(|_| "<set>"))
            .field("voice_models", &self.voice_models)
            .finish_non_exhaustive()
    }
}

impl Config {
    /// From the environment: `TOKIDOKI_PROVIDER` (`anthropic`, the default, `fireworks` or
    /// `baseten`) and that provider's key (`ANTHROPIC_API_KEY`, `FIREWORKS_API_KEY`,
    /// `BASETEN_API_KEY`: required); `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` (default:
    /// the provider's, `Config::models`); `TOKIDOKI_DATA` (default `data`), `TOKIDOKI_USER`
    /// (default `owner`), `TOKIDOKI_SPEAKERS` (comma-separated IDs, default `owner`),
    /// `TOKIDOKI_SYSTEM`; `TYPESAFE_API_KEY` for Jev, with `TOKIDOKI_JEV_MODEL`; and
    /// `EXA_API_KEY` for research workers.
    pub fn from_env() -> Result<Self> {
        let provider = Self::provider_from_env()?;
        let key = var(provider.key_var())
            .with_context(|| format!("{} is not set", provider.key_var()))?;
        Ok(Self::new(telemetry::data_dir(), provider, key).with_env())
    }

    /// The provider `TOKIDOKI_PROVIDER` names: Anthropic when it names none.
    pub fn provider_from_env() -> Result<Vendor> {
        match var("TOKIDOKI_PROVIDER") {
            Some(provider) => provider.parse().context("TOKIDOKI_PROVIDER"),
            None => Ok(Vendor::Anthropic),
        }
    }

    /// With what the environment says of the models, the user, the speakers and the system
    /// prompt (see `from_env`).
    pub fn with_env(mut self) -> Self {
        self = self.with_models_from_env();
        let config = &mut self;
        if let Some(user) = var("TOKIDOKI_USER") {
            config.user = user;
        }
        if let Some(speakers) = var("TOKIDOKI_SPEAKERS") {
            config.speakers = speakers
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(system) = var("TOKIDOKI_SYSTEM") {
            config.system = Some(system);
        }
        if let Some(key) = var("TYPESAFE_API_KEY") {
            config.jev_key = Some(key);
        }
        if let Some(key) = var("EXA_API_KEY") {
            config.exa_key = Some(key);
        }
        if let Some(dir) = var("TOKIDOKI_VOICE_MODELS") {
            config.voice_models = Some(dir.into());
        }
        if let Some(model) = var("TOKIDOKI_JEV_MODEL") {
            config.jev_model = model;
        }
        self
    }

    /// With the models the environment names: `TOKIDOKI_MODELS=cheap` for the provider's
    /// cheapest (tests, always), then `TOKIDOKI_MODEL` and `TOKIDOKI_ROUTER_MODEL` by name.
    pub fn with_models_from_env(mut self) -> Self {
        if var("TOKIDOKI_MODELS").as_deref() == Some("cheap") {
            let (model, router_model) = Self::cheap_models(self.provider);
            self.model = model.into();
            self.router_model = router_model.into();
        }
        if let Some(model) = var("TOKIDOKI_MODEL") {
            self.model = model;
        }
        if let Some(model) = var("TOKIDOKI_ROUTER_MODEL") {
            self.router_model = model;
        }
        self
    }

    /// The models a provider runs unless told otherwise: the one that answers, and the
    /// router's fast one.
    pub fn models(provider: Vendor) -> (&'static str, &'static str) {
        match provider {
            Vendor::Anthropic => ("claude-opus-5-5", "claude-haiku-5-5"),
            // Cheap by choice (the owner's): the flash model answers and routes. Checked
            // against the account's /v1/models on 2026-11-21; deepseek-v4-pro isn't served
            // there, kimi-k3p1-pro is, if something stronger is ever wanted.
            Vendor::Fireworks => (
                "accounts/fireworks/models/deepseek-v4p1-flash",
                "accounts/fireworks/models/deepseek-v4p1-flash",
            ),
            Vendor::Baseten => (
                "deepseek-ai/DeepSeek-V4-Pro",
                "deepseek-ai/DeepSeek-V4.1-Flash",
            ),
        }
    }

    /// The provider's cheapest models that still do the job: what every test that calls a
    /// model runs on (`TOKIDOKI_MODELS=cheap`), never the defaults above.
    pub fn cheap_models(provider: Vendor) -> (&'static str, &'static str) {
        match provider {
            Vendor::Anthropic => ("claude-haiku-5-5", "claude-haiku-5-5"),
            Vendor::Fireworks => (
                "accounts/fireworks/models/deepseek-v4p1-flash",
                "accounts/fireworks/models/deepseek-v4p1-flash",
            ),
            Vendor::Baseten => (
                "deepseek-ai/DeepSeek-V4.1-Flash",
                "deepseek-ai/DeepSeek-V4.1-Flash",
            ),
        }
    }

    /// The owner's own tokidoki, with its data in `data`, running on `provider` with the
    /// user's `key` for it: what a program that holds the key itself (an app) starts from.
    pub fn new(data: impl Into<PathBuf>, provider: Vendor, key: String) -> Self {
        let (model, router_model) = Self::models(provider);
        Self {
            data: data.into(),
            user: "owner".into(),
            speakers: vec!["owner".into()],
            system: Some(DEFAULT_SYSTEM.into()),
            provider,
            key,
            model: model.into(),
            effort: Effort::Low,
            router_model: router_model.into(),
            jev_key: None,
            jev_model: jev::MODEL.into(),
            exa_key: None,
            voice_models: None,
            agent: agent::Settings::default(),
            journal: true,
        }
    }

    pub fn state_path(&self) -> PathBuf {
        self.data.join(telemetry::STATE_FILE)
    }

    pub fn memory_path(&self) -> PathBuf {
        self.data.join(telemetry::MEMORY_FILE)
    }

    pub fn notes_path(&self) -> PathBuf {
        self.data.join(telemetry::NOTES_FILE)
    }
}

/// An environment variable that says something.
fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The pieces `Tokidoki::build` puts together; `open` makes them from a `Config`.
pub struct Parts<M> {
    pub store: Arc<Sqlite>,
    pub model: Arc<M>,
    pub mind: Option<Arc<dyn Mind>>,
    pub router: Router<AnyApi>,
    /// What it remembers, and whose memories those are.
    pub memory: Arc<Memory>,
    pub scope: Scope,
    /// The user's notes; `build` adds the tools for them to `tools`.
    pub notes: Arc<Notes>,
    pub tools: Tools,
    /// What every new thread starts with (with `tools` registered).
    pub system: Option<String>,
    pub settings: agent::Settings,
    pub speakers: Vec<String>,
    /// The journal being recorded to (`telemetry::record`), flushed on `close`. Everything
    /// the system does happens in its span.
    pub journal: Option<telemetry::Recording>,
    /// Exa, for research workers: without it there are no workers.
    pub exa: Option<exa::Exa>,
    /// What decides whether a worker's report is worth interrupting the user for, when they
    /// have moved on from the thread that asked (Jev).
    pub review: Option<Arc<dyn agent::Review>>,
    /// Listening and speaking, when there is a folder for the models.
    pub voice: Option<Arc<voice::Voice>>,
}

/// The running system; cheap to clone.
#[derive(Clone)]
pub struct Tokidoki {
    inner: Arc<Inner>,
}

struct Inner {
    agent: Agent,
    live: Live,
    router: Router<AnyApi>,
    memory: Arc<Memory>,
    scope: Scope,
    notes: Arc<Notes>,
    voice: Option<Arc<voice::Voice>>,
    store: Arc<Sqlite>,
    speakers: Vec<String>,
    journal: Option<telemetry::Recording>,
    /// Why it runs nothing, when it doesn't (`open_asleep`).
    asleep: Option<String>,
}

impl Tokidoki {
    /// Opens the system from `config`, with `tools` for the model, and resumes unfinished
    /// work. Call inside a Tokio runtime.
    pub fn open(config: Config, tools: Tools) -> Result<Self> {
        Self::open_as(config, tools, None)
    }

    /// Opens what `config`'s folder holds without running any of it, for when it can't be
    /// run (no key for the model, say): its threads, history and memory can be read and its
    /// memory changed, unfinished work stays unfinished for the next `open`, and an input is
    /// rejected with `why`. `config`'s keys are not used.
    pub fn open_asleep(config: Config, why: impl Into<String>) -> Result<Self> {
        Self::open_as(config, Tools::new(), Some(why.into()))
    }

    fn open_as(config: Config, tools: Tools, asleep: Option<String>) -> Result<Self> {
        std::fs::create_dir_all(&config.data)
            .with_context(|| format!("data directory {}", config.data.display()))?;
        let journal = match config.journal {
            // Fails if the program installed its own subscriber, which then gets the events.
            true => telemetry::record(&telemetry::Journal::beside(config.state_path()))
                .map(Some)
                .unwrap_or_else(|error| {
                    eprintln!("tokidoki: not recording the journal: {error:#}");
                    None
                }),
            false => None,
        };
        if cfg!(panic = "abort") {
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                eprintln!(
                    "tokidoki: built with panic = \"abort\": a task that panics ends the \
                     process instead of being faulted"
                )
            });
        }
        // Opening is traced in the system's own span too.
        let _recorded = span_of(&journal).entered();
        let store = Arc::new(Sqlite::open(config.state_path())?);
        let memory = Arc::new(Memory::open(config.memory_path())?);
        let notes = Arc::new(Notes::open(config.notes_path())?);
        let scope = Scope::user(&config.user);
        let answers = || Request::new(config.model.clone()).effort(config.effort);
        let provider = || Provider::of(config.provider, &config.key);
        let mut router = Router::new(
            provider(),
            Request::new(config.router_model.clone()).thinking(false),
            memory.clone(),
            vec![scope.clone()],
        );
        // A thread cools when it dreams.
        router.dream_after = std::time::Duration::from_millis(config.agent.dream_after_ms);
        // With a TypeSafe key, Jev makes the quick decisions: where an input goes, and what to
        // recall and keep for it.
        let jev =
            (config.jev_key.as_ref()).map(|key| jev::Jev::new(key).model(config.jev_model.clone()));
        if let Some(jev) = &jev {
            router.classifier = Some(Box::new(router::JevClassifier::new(jev.clone())));
        }
        let mut experience = Experience::new(
            provider(),
            answers(),
            memory.clone(),
            Scopes {
                read: vec![scope.clone()],
                write: scope.clone(),
            },
        );
        // With an Exa key there are research workers: Jev also judges what to hand them, and
        // whether a report is worth interrupting the user for.
        let exa = config.exa_key.as_ref().map(exa::Exa::new);
        experience.delegation = exa.is_some();
        let review = (jev.clone())
            .map(|jev| Arc::new(worker_tools::JevReview(jev)) as Arc<dyn agent::Review>);
        experience.jev = jev;
        let model = Arc::new(ProviderModel {
            provider: provider(),
            request: answers(),
        });
        let parts = Parts {
            store,
            model,
            mind: Some(Arc::new(experience)),
            router,
            memory,
            scope,
            notes,
            tools,
            system: config.system,
            settings: config.agent,
            speakers: config.speakers,
            journal,
            exa,
            review,
            voice: (config.voice_models.clone()).map(|dir| Arc::new(voice::Voice::new(dir))),
        };
        Self::assemble(parts, asleep)
    }

    /// Puts given parts together (another model, a test double), and resumes.
    pub fn build<M: Model>(parts: Parts<M>) -> Result<Self> {
        Self::assemble(parts, None)
    }

    fn assemble<M: Model>(parts: Parts<M>, asleep: Option<String>) -> Result<Self> {
        // The engine keeps the span it is opened in, for everything it runs.
        let _recorded = span_of(&parts.journal).entered();
        // Every thread can keep the user's notes, and is told when to; and, with Exa, hand
        // research to workers.
        let mut tools = parts.tools;
        note_tools::add(&mut tools, &parts.notes);
        let mut system = match parts.system {
            Some(system) => format!("{system}\n\n{}", note_tools::PROMPT),
            None => note_tools::PROMPT.to_string(),
        };
        let late = worker_tools::Late::default();
        if parts.exa.is_some() {
            worker_tools::add(&mut tools, &late);
            system = format!("{system}\n\n{}", worker_tools::PROMPT);
        }
        let setup = ThreadSetup {
            system: Some(system),
            tools: tools.specs(),
        };
        // A research worker searches with its own tools, which the conversation doesn't have.
        let mut kinds = std::collections::HashMap::new();
        if let Some(exa) = &parts.exa {
            let research = worker_tools::research(exa);
            let setup = ThreadSetup {
                system: Some(worker_tools::RESEARCH_PROMPT.into()),
                tools: research.specs(),
            };
            kinds.insert(worker_tools::RESEARCH.to_string(), setup);
            tools.extend(research);
        }
        let live = Live::default();
        // Whatever changes memory (answering, dreaming, the user) or a note is shown as it
        // happens.
        let shown = live.clone();
        parts
            .memory
            .watch(move |change| shown.show(events::memory_change(change)));
        let shown = live.clone();
        parts
            .notes
            .watch(move |change| shown.show(note_tools::change(change)));
        let mut registry = Registry::new();
        agent::register_with_review(
            &mut registry,
            parts.model,
            tools,
            parts.mind,
            parts.settings,
            live.clone(),
            parts.review,
        );
        let engine = Engine::open(parts.store.clone(), registry, engine::Settings::default())?;
        if asleep.is_none() {
            engine.resume();
        }
        let agent = Agent::new(engine, setup).with_workers(kinds);
        let _ = late.set(agent.clone());
        Ok(Self {
            inner: Arc::new(Inner {
                agent,
                live,
                router: parts.router,
                memory: parts.memory,
                scope: parts.scope,
                notes: parts.notes,
                voice: parts.voice,
                store: parts.store,
                speakers: parts.speakers,
                journal: parts.journal,
                asleep,
            }),
        })
    }

    /// Stops running work (it resumes when the system is opened again), waits until it has
    /// stopped, and finishes writing the journal: nothing of this system touches its files
    /// afterwards, so they can be opened again, or deleted.
    pub async fn close(&self) -> Result<()> {
        let engine = self.inner.agent.engine();
        let closed = self.span().in_scope(|| engine.close());
        // Work that never yields can't be stopped: go on without it rather than hang.
        if tokio::time::timeout(STOP_WITHIN, engine.stopped())
            .await
            .is_err()
        {
            self.span()
                .in_scope(|| tracing::warn!("closed while some work was still running"));
        }
        if let Some(recording) = &self.inner.journal {
            recording.journal.flush();
        }
        closed
    }

    /// Whether `close` was called: it takes nothing more, and `open` gives a new one.
    pub fn is_closed(&self) -> bool {
        self.inner.agent.engine().is_closed()
    }

    /// The span this system's own calls are traced in (see `span_of`).
    fn span(&self) -> tracing::Span {
        span_of(&self.inner.journal)
    }

    fn admit(&self, speaker: &interface::Speaker) -> Result<()> {
        if !self.inner.speakers.contains(&speaker.id) {
            let id = &speaker.id;
            return Err(Rejected(format!("{id} may not speak to this tokidoki")).into());
        }
        Ok(())
    }
}

/// What an embedding app calls. Each takes and returns `interface` types, ready to pass on as
/// JSON; an input it won't take fails with an `interface::Rejected`.
impl Tokidoki {
    /// Admits an input, to its thread or wherever the router sends it, and returns at once:
    /// follow it with `events` or `wait`.
    pub async fn send(&self, input: Input) -> Result<Sent> {
        self.admit(&input.from)?;
        if let Some(why) = &self.inner.asleep {
            return Err(Rejected(why.clone()).into());
        }
        let speaker = input.from.id.clone();
        self.inner
            .agent
            .send(&self.inner.router, input)
            .instrument(tracing::info_span!(parent: self.span(), "input", speaker = %speaker))
            .await
    }

    pub fn submission(&self, id: &str) -> Result<Option<Submission>> {
        self.inner.agent.submission(id)
    }

    /// Every thread, most recently active first.
    pub fn threads(&self) -> Result<Vec<ThreadSummary>> {
        let now = std::time::SystemTime::now();
        let views = self.inner.router.threads(self.inner.store.as_ref(), now)?;
        Ok(views
            .into_iter()
            .filter(|view| !agent::is_worker(&view.info.id))
            .map(|view| ThreadSummary {
                about: view.card.map(|card| card.text),
                resume: view.resume.map(|resume| resume.text),
                phase: match view.phase {
                    router::Phase::Hot => ThreadPhase::Hot,
                    router::Phase::Cool => ThreadPhase::Cool,
                    router::Phase::Settled => ThreadPhase::Settled,
                },
                id: view.info.id,
                title: view.info.title,
                turns: view.info.turns,
                created_at: telemetry::time::ms(view.info.created_at),
                active_at: telemetry::time::ms(view.info.active_at),
            })
            .collect())
    }

    /// The conversation as the user sees it: every thread's turns as one, in the order they
    /// started, as their checkpoints. The last `turns` of those that started before `before`
    /// (ms since the epoch; `None` for the latest), so earlier ones can be read page by page.
    pub fn timeline(&self, before: Option<u64>, turns: usize) -> Result<Vec<interface::Event>> {
        let engine = self.inner.agent.engine();
        // (when it started, the thread, its checkpoints) for every turn.
        let mut all: Vec<(u64, String, Vec<interface::Event>)> = Vec::new();
        for info in self.inner.store.threads()? {
            if agent::is_worker(&info.id) {
                continue;
            }
            engine.thread(&info.id, |thread| {
                for checkpoint in thread.checkpoints() {
                    let event = events::checkpoint(&info.id, checkpoint);
                    if matches!(checkpoint.event, state::Event::TurnStarted) {
                        let at = telemetry::time::ms(checkpoint.at);
                        all.push((at, info.id.clone(), vec![event]));
                    } else if let Some(turn) = all.last_mut().filter(|t| t.1 == info.id) {
                        // What happens between turns (routing the next input, dreaming) goes
                        // with the turn before it; what came before the first, nowhere.
                        turn.2.push(event);
                    }
                }
            })?;
        }
        all.retain(|turn| before.is_none_or(|before| turn.0 < before));
        all.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let skip = all.len().saturating_sub(turns);
        Ok(all.into_iter().skip(skip).flat_map(|turn| turn.2).collect())
    }

    /// What it remembers, newest first: the current version of each memory.
    pub fn memories(&self, from: &Speaker) -> Result<Vec<interface::Memory>> {
        self.admit(from)?;
        let scope = std::slice::from_ref(&self.inner.scope);
        let items = self.inner.memory.list(scope, MEMORIES_SHOWN)?;
        Ok(items.iter().map(events::memory).collect())
    }

    /// Something the user tells it outright, kept as a fact.
    pub fn remember(&self, from: &Speaker, text: &str) -> Result<interface::Memory> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        let write = Write::fact(self.inner.scope.clone(), stated(text)?);
        let item = self.inner.memory.write(write)?;
        tracing::info!(id = item.id, speaker = %from.id, "memory added by the user");
        Ok(events::memory(&item))
    }

    /// The user's correction of a fact: a new version of the memory `id`, which keeps where
    /// it was learned. Rejected for an ID that is no longer current (it was revised or
    /// forgotten meanwhile: read `memories` again).
    pub fn revise(&self, from: &Speaker, id: i64, text: &str) -> Result<interface::Memory> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        let old = self.current(id)?;
        if !matches!(old.body, Body::Semantic { .. }) {
            return Err(
                Rejected("only facts can be edited; forget this one instead".into()).into(),
            );
        }
        let mut write = Write::fact(self.inner.scope.clone(), stated(text)?);
        write.source = old.source;
        let item = self.inner.memory.replace(id, write)?;
        tracing::info!(id = item.id, replaces = id, speaker = %from.id, "memory revised by the user");
        Ok(events::memory(&item))
    }

    /// Removes the memory `id` and every earlier version of it, for good.
    pub fn forget(&self, from: &Speaker, id: i64) -> Result<()> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        self.current(id)?;
        self.inner.memory.forget(id)?;
        tracing::info!(id, speaker = %from.id, "memory forgotten by the user");
        Ok(())
    }

    /// Forgets everything it has learned: every fact and procedure, and its summaries of
    /// past conversations. The conversations themselves stay. There is no undoing it.
    pub fn wipe(&self, from: &Speaker) -> Result<()> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        self.inner.memory.wipe(&self.inner.scope)?;
        tracing::info!(speaker = %from.id, "memory wiped by the user");
        Ok(())
    }

    /// The user's notes that match `search`: best match first, or, with no query, the most
    /// recently changed first.
    pub fn notes(
        &self,
        from: &Speaker,
        search: interface::NoteSearch,
    ) -> Result<Vec<interface::Note>> {
        self.admit(from)?;
        let found = self.inner.notes.search(&notes::Search {
            query: search.query,
            tag: search.tag,
            project: search.project,
            limit: NOTES_SHOWN,
        })?;
        Ok(found.into_iter().map(note_tools::note).collect())
    }

    /// A new note from the user, or (with `id`) their change to one; it keeps where it came
    /// from.
    pub fn write_note(
        &self,
        from: &Speaker,
        id: Option<i64>,
        draft: interface::NoteDraft,
    ) -> Result<interface::Note> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        let draft = note_tools::draft(draft);
        let saved = match id {
            Some(id) => self.inner.notes.update(id, draft),
            None => self.inner.notes.add(draft, None),
        };
        // What can go wrong is the user's to fix: an empty note, or one deleted meanwhile.
        let saved = saved.map_err(|error| Rejected(format!("{error:#}")))?;
        Ok(note_tools::note(saved))
    }

    /// Deletes the note `id` for good.
    pub fn delete_note(&self, from: &Speaker, id: i64) -> Result<()> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        if !self.inner.notes.delete(id)? {
            return Err(Rejected(format!("note {id} is already gone")).into());
        }
        Ok(())
    }

    /// Every worker there has been, latest first: what it was asked, how it is doing, what it
    /// found.
    pub fn workers(&self, from: &Speaker) -> Result<Vec<interface::Worker>> {
        self.admit(from)?;
        let workers = self.inner.agent.workers()?.into_iter().rev();
        Ok(workers.map(worker_tools::shown).collect())
    }

    /// Stops the worker `id`; its report never comes. Returns whether it was running.
    pub fn cancel_worker(&self, from: &Speaker, id: &str) -> Result<bool> {
        let _recorded = self.span().entered();
        self.admit(from)?;
        self.inner.agent.cancel_worker(id)
    }

    /// Where the voice models stand; `None` when there is no voice (no folder for its models).
    pub fn voice(&self) -> Option<interface::VoiceStatus> {
        self.inner.voice.as_deref().map(speech::status)
    }

    /// Downloads the voice models that are missing (once, ~900 MB), showing progress as
    /// `Event::Voice`. Returns when both are ready.
    pub async fn prepare_voice(&self, from: &Speaker) -> Result<()> {
        self.admit(from)?;
        let Some(voice) = self.inner.voice.clone() else {
            return Err(Rejected("there is no voice here".into()).into());
        };
        let live = self.inner.live.clone();
        // Whole percents: a few hundred events, not one per network chunk.
        let last = std::sync::Mutex::new(None);
        let progress = |model: voice::Model, done: f32| {
            let now = Some((model, (done * 100.0) as u32));
            if std::mem::replace(&mut *last.lock().unwrap(), now) != now {
                live.show(interface::Event::Voice {
                    status: speech::status(&voice),
                });
            }
        };
        let prepared = voice.prepare(progress).await;
        live.show(interface::Event::Voice {
            status: speech::status(&voice),
        });
        prepared
    }

    /// What was said in `wav` (any rate, mono or stereo), as text, with its language.
    pub async fn listen(&self, from: &Speaker, wav: Vec<u8>) -> Result<interface::Heard> {
        self.admit(from)?;
        let voice = self.voice_ready()?;
        speech::listen(voice, wav).instrument(self.span()).await
    }

    /// `reply` (Markdown) read out in `language`, as WAV. Rejected for a language it can't
    /// speak (`Heard::speakable`).
    pub async fn speak(&self, from: &Speaker, reply: &str, language: &str) -> Result<Vec<u8>> {
        self.admit(from)?;
        let voice = self.voice_ready()?;
        if !voice::Voice::speaks(language) {
            return Err(Rejected(format!("tokidoki can't speak {language} yet")).into());
        }
        speech::speak(voice, reply.to_string(), language.to_string()).await
    }

    /// Loads the voice models if they aren't yet, in the background: call it as the user starts
    /// talking, so hearing them doesn't wait on it.
    pub fn warm_voice(&self) {
        if let Ok(voice) = self.voice_ready() {
            // A thread of its own: called from anywhere, a runtime or not.
            std::thread::spawn(move || {
                if let Err(error) = voice.warm() {
                    tracing::warn!(error = %format!("{error:#}"), "couldn't load the voice models");
                }
            });
        }
    }

    fn voice_ready(&self) -> Result<Arc<voice::Voice>> {
        let Some(voice) = self.inner.voice.clone() else {
            return Err(Rejected("there is no voice here".into()).into());
        };
        let ready = voice::Model::ALL
            .iter()
            .all(|model| voice.state(*model) == voice::State::Ready);
        if !ready {
            return Err(Rejected("the voice models aren't downloaded yet".into()).into());
        }
        Ok(voice)
    }

    /// The memory `id`, if it is this user's and still current.
    fn current(&self, id: i64) -> Result<memory::Item> {
        match self.inner.memory.get(id)? {
            Some(item) if item.scope == self.inner.scope => Ok(item),
            _ => Err(Rejected(format!("memory {id} has changed or is gone")).into()),
        }
    }

    /// A thread's checkpoints from `since` on, as events: to load a thread, or catch up after
    /// missing some (`Event::Checkpoint::seq` has no gaps).
    pub fn history(&self, thread: &str, since: usize) -> Result<Vec<interface::Event>> {
        events::checkpoints(self.inner.agent.engine(), thread, since..usize::MAX)
    }

    /// What happens from now on, across threads, until the receiver is dropped: what is
    /// committed, and replies as they are written (`Event::Delta`). Subscribe before `send`
    /// to miss nothing about the input. Call inside a Tokio runtime.
    pub fn events(&self) -> Events {
        events::follow(self.inner.agent.engine(), &self.inner.live)
    }

    /// Resolves once the submission is answered or can't be.
    pub async fn wait(&self, id: &str) -> Result<Submission> {
        self.inner.agent.wait(id).instrument(self.span()).await
    }

    /// Stops the thread's run, if one is running. Returns whether there was one.
    pub fn abort(&self, thread: &str) -> Result<bool> {
        let _recorded = self.span().entered();
        self.inner.agent.abort(thread)
    }

    /// "That was wrong", about a turn of a thread: kept with the thread in the state database,
    /// and traced as a warning so `just inspect recent` and `problems` show it on that turn.
    pub fn feedback(&self, feedback: Feedback) -> Result<()> {
        let _recorded = self.span().entered();
        self.admit(&feedback.from)?;
        let thread = &feedback.thread;
        let turn = self.inner.agent.engine().commit(|tx| {
            let turn = match feedback.turn {
                Some(turn) => turn,
                None => tx.thread(thread)?.turn(),
            };
            let key = feedback_key(thread);
            let mut all: Vec<Feedback> = tx.doc(&key)?.unwrap_or_default();
            all.push(Feedback {
                turn: Some(turn),
                ..feedback.clone()
            });
            tx.put_doc(&key, &all)?;
            Ok(turn)
        })?;
        tracing::warn!(
            thread = %thread,
            turn,
            speaker = %feedback.from.id,
            text = %feedback.text,
            "feedback"
        );
        Ok(())
    }
}

/// The span a system's work is traced in: its journal's, so what it does is recorded there
/// and not in another system's. With no journal, the caller's own span.
fn span_of(journal: &Option<telemetry::Recording>) -> tracing::Span {
    journal
        .as_ref()
        .map_or_else(tracing::Span::current, |recording| recording.span.clone())
}

/// How long `close` waits for running work to stop.
const STOP_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

/// How many memories `memories` returns at most.
const MEMORIES_SHOWN: usize = 2_000;

/// How many notes `notes` returns at most.
const NOTES_SHOWN: usize = 500;

/// A memory's text as the user gave it, trimmed; rejected when empty.
fn stated(text: &str) -> Result<String> {
    match text.trim() {
        "" => Err(Rejected("a memory needs some text".into()).into()),
        text => Ok(text.to_string()),
    }
}

/// Where a thread's feedback is kept, in the state database.
fn feedback_key(thread: &str) -> String {
    format!("feedback/{thread}")
}

#[cfg(test)]
mod tests;
