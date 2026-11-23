use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::*;
use agent::{ToolFuture, Tools};
use context::{Block, Context, Message, Role as CtxRole};
use inference::Response;
use inference::providers::OpenAi;
use interface::{Change, Content, Event, Part, Speaker, Status};
use serde_json::{Value, json};

const T: Duration = Duration::from_secs(5);

enum Reply {
    Text(&'static str),
    /// Written word by word, for whoever watches.
    Streamed(&'static str),
    Call(&'static str, Value),
    Hang,
}

/// A model that replies from a script and remembers the contexts it saw.
#[derive(Default)]
struct Script {
    replies: Mutex<VecDeque<Reply>>,
    seen: Mutex<Vec<Context>>,
}

impl Model for Script {
    async fn chat(&self, ctx: &Context) -> Result<Response> {
        self.seen.lock().unwrap().push(ctx.clone());
        let reply = self.replies.lock().unwrap().pop_front();
        let content = match reply {
            Some(Reply::Text(text) | Reply::Streamed(text)) => {
                vec![Block::Text { text: text.into() }]
            }
            Some(Reply::Call(name, input)) => vec![Block::ToolCall {
                id: "call-1".into(),
                name: name.into(),
                input,
            }],
            Some(Reply::Hang) => std::future::pending().await,
            None => anyhow::bail!("the script ran out"),
        };
        Ok(Response {
            message: Message {
                role: CtxRole::Assistant,
                content,
            },
            input_tokens: 0,
            output_tokens: 0,
            stop: inference::Stop::Done,
        })
    }

    async fn stream(
        &self,
        ctx: &Context,
        on_delta: inference::providers::OnDelta<'_>,
    ) -> Result<Response> {
        if let Some(Reply::Streamed(text)) = self.replies.lock().unwrap().front() {
            for word in text.split_inclusive(' ') {
                on_delta(inference::Delta::Text(word.into()));
            }
        }
        self.chat(ctx).await
    }
}

struct Weather;

impl agent::Tool for Weather {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "weather".into(),
            description: "The weather in a city.".into(),
            input_schema: json!({ "type": "object", "properties": { "city": { "type": "string" } } }),
        }
    }

    fn execute(&self, _: agent::Call) -> ToolFuture<'_> {
        Box::pin(async { Ok("18C".to_string()) })
    }
}

fn start(replies: Vec<Reply>) -> (Tokidoki, Arc<Script>) {
    let (parts, script) = parts(replies, Sqlite::memory().unwrap());
    (Tokidoki::build(parts).unwrap(), script)
}

/// What `start` puts together, keeping its threads in `store`.
fn parts(replies: Vec<Reply>, store: Sqlite) -> (Parts<Script>, Arc<Script>) {
    let script = Arc::new(Script {
        replies: Mutex::new(replies.into()),
        ..Script::default()
    });
    let memory = Arc::new(Memory::in_memory().unwrap());
    // Nobody answers there: routing must decide in code.
    let nowhere =
        Provider::new(OpenAi, "Nowhere", "http://127.0.0.1:9/", vec![]).any(AnyApi::OpenAi);
    let router = Router::new(nowhere, Request::new("none"), memory.clone(), vec![]);
    let mut tools = Tools::new();
    tools.add(Weather);
    let parts = Parts {
        store: Arc::new(store),
        model: script.clone(),
        mind: None,
        router,
        memory,
        scope: Scope::user("owner"),
        notes: Arc::new(notes::Notes::in_memory().unwrap()),
        tools,
        system: Some("Be brief.".into()),
        settings: agent::Settings {
            backoff_ms: 10,
            ..agent::Settings::default()
        },
        speakers: vec!["owner".into()],
        journal: None,
        exa: None,
        review: None,
        voice: None,
    };
    (parts, script)
}

fn me(text: &str) -> Input {
    Input::text(Speaker::owner(), text)
}

async fn settled(tokidoki: &Tokidoki, id: &str) -> Submission {
    tokio::time::timeout(T, tokidoki.wait(id))
        .await
        .unwrap()
        .unwrap()
}

/// Events until (and with) the first that `last` matches.
async fn until(events: &mut Events, last: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(T, events.recv())
            .await
            .unwrap()
            .unwrap();
        let done = last(&event);
        seen.push(event);
        if done {
            return seen;
        }
    }
}

fn answered(event: &Event) -> bool {
    matches!(event, Event::Submission { submission } if submission.is_settled())
}

#[tokio::test]
async fn an_input_is_routed_answered_and_followed() {
    let (tokidoki, script) = start(vec![
        Reply::Call("weather", json!({ "city": "Paris" })),
        Reply::Text("18C in Paris."),
    ]);
    let mut events = tokidoki.events();
    let sent = tokidoki.send(me("Weather in Paris?")).await.unwrap();
    let route = sent.route.clone().unwrap();
    assert!(route.new);
    assert_eq!(route.by, "rule");

    let seen = until(&mut events, answered).await;
    let Some(Event::Submission { submission }) = seen.last() else {
        unreachable!()
    };
    assert_eq!(submission.status, Status::Done);
    assert_eq!(submission.answer.as_deref(), Some("18C in Paris."));
    // Each status as it was committed, in order: never a later one read early.
    let statuses: Vec<Status> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Submission { submission } if submission.id == sent.submission => {
                Some(submission.status)
            }
            _ => None,
        })
        .collect();
    assert_eq!(statuses, [Status::Placed, Status::Done]);

    // The thread's checkpoints arrive in order, without gaps, as history has them.
    let seqs: Vec<usize> = seen
        .iter()
        .filter_map(|e| match e {
            Event::Checkpoint { thread, seq, .. } if *thread == sent.thread => Some(*seq),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, (0..seqs.len()).collect::<Vec<_>>());
    let history = tokidoki.history(&sent.thread, 0).unwrap();
    assert_eq!(history.len(), seqs.len());
    let changes: Vec<&Change> = history
        .iter()
        .map(|e| match e {
            Event::Checkpoint { change, .. } => change,
            _ => unreachable!(),
        })
        .collect();
    assert!(matches!(changes[0], Change::System { .. }));
    assert!(changes.iter().any(|c| matches!(c, Change::Routed { .. })));
    let tool_result = changes.iter().any(|c| {
        matches!(c, Change::Message { content, .. }
            if matches!(&content[..], [Content::ToolResult { content, .. }] if content == "18C"))
    });
    assert!(tool_result, "{changes:#?}");
    assert!(matches!(changes.last(), Some(Change::TurnEnded)));
    assert_eq!(tokidoki.history(&sent.thread, 100).unwrap(), []);

    // What worked on it was reported as it went.
    assert!(seen.iter().any(
        |e| matches!(e, Event::Activity { doing: interface::Doing::Tool { name }, .. } if name == "weather")
    ));
    let thinking = |e: &Event| {
        matches!(
            e,
            Event::Activity {
                doing: interface::Doing::Thinking,
                ..
            }
        )
    };
    assert_eq!(
        seen.iter().filter(|e| thinking(e)).count(),
        2,
        "one per model request"
    );
    // The model saw the thread's setup: the prompt, with what it says of notes, and the
    // tools, with the note tools.
    let first = &script.seen.lock().unwrap()[0];
    let system = first.system.as_deref().unwrap();
    assert!(system.starts_with("Be brief.\n\nNotes are the user's own"));
    let tools: Vec<&str> = first.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(tools, ["get_note", "save_note", "search_notes", "weather"]);

    let threads = tokidoki.threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].title, "Weather in Paris?");
}

#[tokio::test]
async fn naming_a_thread_skips_the_router() {
    let (tokidoki, _) = start(vec![Reply::Text("one"), Reply::Text("two")]);
    let first = tokidoki.send(me("hi").to("mine")).await.unwrap();
    assert_eq!(first.thread, "mine");
    assert_eq!(first.route, None);
    settled(&tokidoki, &first.submission).await;
    let second = tokidoki.send(me("again").to("mine")).await.unwrap();
    assert_eq!(
        settled(&tokidoki, &second.submission)
            .await
            .answer
            .as_deref(),
        Some("two")
    );
}

#[tokio::test]
async fn refuses_strangers_and_senses_it_cant_use_yet() {
    let (tokidoki, script) = start(vec![]);
    let refused = |error: anyhow::Error| error.downcast_ref::<Rejected>().cloned().unwrap().0;

    let stranger = Input::text(Speaker::new("discord:42"), "hi");
    let error = tokidoki.send(stranger).await.unwrap_err();
    assert_eq!(refused(error), "discord:42 may not speak to this tokidoki");

    let image = Input {
        parts: vec![Part::Image {
            media_type: "image/png".into(),
            data: interface::Media::Url("https://example.com/a.png".into()),
        }],
        ..me("")
    };
    assert_eq!(
        refused(tokidoki.send(image).await.unwrap_err()),
        "image inputs aren't supported yet"
    );
    assert_eq!(
        refused(tokidoki.send(me("  ")).await.unwrap_err()),
        "the input is empty"
    );
    assert!(script.seen.lock().unwrap().is_empty());
    assert!(tokidoki.threads().unwrap().is_empty());
}

#[tokio::test]
async fn abort_stops_a_run_and_feedback_is_recorded() {
    let (journal, _guard) = telemetry::capture();
    let (tokidoki, script) = start(vec![Reply::Hang]);
    let sent = tokidoki.send(me("Think hard").to("t1")).await.unwrap();
    let started = tokio::time::Instant::now();
    while script.seen.lock().unwrap().is_empty() {
        assert!(started.elapsed() < T);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(tokidoki.abort("t1").unwrap());
    assert_eq!(
        settled(&tokidoki, &sent.submission).await.status,
        Status::Unanswered
    );

    tokidoki
        .feedback(Feedback {
            from: Speaker::owner(),
            thread: "t1".into(),
            turn: None,
            text: "it never answered".into(),
        })
        .unwrap();
    let recorded = journal
        .entries(&telemetry::Filter {
            message: Some("feedback".into()),
            ..telemetry::Filter::default()
        })
        .unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].thread.as_deref(), Some("t1"));
    assert_eq!(recorded[0].fields["turn"], 1);
    assert_eq!(recorded[0].severity, telemetry::Severity::Warn);
    // Kept with the thread too, whatever the journal records.
    let kept: Vec<Feedback> = tokidoki
        .inner
        .agent
        .engine()
        .doc(&feedback_key("t1"))
        .unwrap()
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].turn, Some(1));
}

/// A reply shows as it is written, then arrives whole: the pieces come first, add up to the
/// message, and `history` holds only the message.
#[tokio::test]
async fn a_reply_is_shown_as_it_is_written() {
    let (tokidoki, _) = start(vec![Reply::Streamed("One two three.")]);
    let mut events = tokidoki.events();
    let sent = tokidoki.send(me("Count.")).await.unwrap();
    let seen = until(&mut events, answered).await;

    let reply = seen
        .iter()
        .position(|event| {
            matches!(event, Event::Checkpoint { change: Change::Message { role, .. }, .. }
                if *role == interface::Role::Assistant)
        })
        .unwrap();
    let written: Vec<(usize, &str)> = seen
        .iter()
        .enumerate()
        .filter_map(|(at, event)| match event {
            Event::Delta {
                thread,
                delta: interface::Delta::Text { text },
            } if *thread == sent.thread => Some((at, text.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(written.len(), 3);
    assert!(written.iter().all(|(at, _)| *at < reply));
    let text: String = written.iter().map(|(_, text)| *text).collect();
    assert_eq!(text, "One two three.");
    let history = tokidoki.history(&sent.thread, 0).unwrap();
    assert!(!history.iter().any(|e| matches!(e, Event::Delta { .. })));
}

/// The user sees what is remembered, adds to it, corrects it and removes it; each change is
/// an event, and a correction keeps the history behind one memory.
#[tokio::test]
async fn memory_is_read_and_changed_by_its_owner() {
    use interface::MemoryChange;
    let (tokidoki, _) = start(vec![]);
    let owner = Speaker::owner();
    let mut events = tokidoki.events();
    async fn change(events: &mut Events) -> MemoryChange {
        match until(events, |e| matches!(e, Event::Memory { .. }))
            .await
            .pop()
        {
            Some(Event::Memory { change }) => change,
            _ => unreachable!(),
        }
    }

    let fact = tokidoki.remember(&owner, "  Vegetarian  ").unwrap();
    assert_eq!((fact.text.as_str(), fact.version), ("Vegetarian", 1));
    let learned = MemoryChange::Learned {
        memory: fact.clone(),
    };
    assert_eq!(change(&mut events).await, learned);

    let revised = tokidoki.revise(&owner, fact.id, "Vegan").unwrap();
    assert_eq!(revised.version, 2);
    let corrected = MemoryChange::Revised {
        memory: revised.clone(),
        replaces: fact.id,
        was: "Vegetarian".into(),
    };
    assert_eq!(change(&mut events).await, corrected);
    assert_eq!(tokidoki.memories(&owner).unwrap(), vec![revised.clone()]);

    // The version that was replaced can no longer be edited or forgotten.
    let stale = tokidoki.revise(&owner, fact.id, "Pescatarian").unwrap_err();
    assert!(stale.downcast_ref::<Rejected>().is_some());
    assert!(tokidoki.forget(&owner, fact.id).is_err());
    assert!(tokidoki.remember(&owner, "  ").is_err());
    assert!(tokidoki.memories(&Speaker::new("stranger")).is_err());

    tokidoki.forget(&owner, revised.id).unwrap();
    assert!(tokidoki.memories(&owner).unwrap().is_empty());
    let forgotten = MemoryChange::Forgotten {
        ids: vec![fact.id, revised.id],
    };
    assert_eq!(change(&mut events).await, forgotten);

    // A wipe takes everything at once, and only the owner may ask for it.
    let a = tokidoki.remember(&owner, "Likes tea").unwrap();
    let b = tokidoki.remember(&owner, "Lives in Kyoto").unwrap();
    assert!(tokidoki.wipe(&Speaker::new("stranger")).is_err());
    tokidoki.wipe(&owner).unwrap();
    assert!(tokidoki.memories(&owner).unwrap().is_empty());
    let mut wiped = Vec::new();
    for _ in 0..3 {
        if let MemoryChange::Forgotten { ids } = change(&mut events).await {
            wiped.extend(ids);
            if wiped.len() == 2 {
                break;
            }
        }
    }
    wiped.sort_unstable();
    assert_eq!(wiped, vec![a.id, b.id]);
}

/// A folder to work in, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("tokidoki-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opened asleep (no key to run with), a tokidoki shows what it has and takes no input.
#[tokio::test]
async fn asleep_it_can_be_read_and_takes_no_input() {
    let root = Scratch::new("asleep");
    std::fs::create_dir_all(&root.0).unwrap();
    let store = || Sqlite::open(root.0.join("tokidoki.db")).unwrap();
    let owner = Speaker::owner();

    let awake = Tokidoki::build(parts(vec![Reply::Streamed("Hello there.")], store()).0).unwrap();
    let sent = awake.send(Input::text(owner.clone(), "hi")).await.unwrap();
    awake.wait(&sent.submission).await.unwrap();
    let history = awake.history(&sent.thread, 0).unwrap();
    awake.close().await.unwrap();

    let why = Some("no key".to_string());
    let asleep = Tokidoki::assemble(parts(Vec::new(), store()).0, why).unwrap();
    let threads = asleep.threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, sent.thread);
    assert_eq!(asleep.history(&sent.thread, 0).unwrap(), history);
    asleep.remember(&owner, "Uses Rust").unwrap();
    let refused = asleep.send(Input::text(owner, "again")).await.unwrap_err();
    assert_eq!(refused.downcast::<Rejected>().unwrap().0, "no key");
    assert_eq!(asleep.threads().unwrap()[0].turns, threads[0].turns);
    asleep.close().await.unwrap();
}

#[tokio::test]
async fn a_note_the_user_asks_for_is_saved_from_its_thread_and_shown() {
    let save = json!({
        "title": "Groceries",
        "body": "- oat milk\n- lentils",
        "tags": "shopping, weekly"
    });
    let (tokidoki, _) = start(vec![Reply::Call("save_note", save), Reply::Text("Saved.")]);
    let owner = Speaker::owner();
    let mut events = tokidoki.events();
    let sent = tokidoki
        .send(me("Save these notes: oat milk, lentils"))
        .await
        .unwrap();
    let submission = settled(&tokidoki, &sent.submission).await;
    assert_eq!(submission.answer.as_deref(), Some("Saved."));

    let seen = until(&mut events, |e| matches!(e, Event::Note { .. })).await;
    let Some(Event::Note {
        change: interface::NoteChange::Saved { note },
    }) = seen.last()
    else {
        panic!("no note event in {seen:?}");
    };
    assert_eq!(note.title, "Groceries");
    assert_eq!(note.tags, ["shopping", "weekly"]);
    assert_eq!(note.thread.as_deref(), Some(sent.thread.as_str()));
    let all = tokidoki.notes(&owner, Default::default()).unwrap();
    assert_eq!(all, std::slice::from_ref(note));
}

#[tokio::test]
async fn notes_are_written_changed_and_deleted_by_their_owner() {
    let (tokidoki, _) = start(Vec::new());
    let owner = Speaker::owner();
    let stranger = Speaker::new("stranger");
    let draft = |title: &str, body: &str| interface::NoteDraft {
        title: title.into(),
        body: body.into(),
        ..Default::default()
    };

    let note = tokidoki
        .write_note(&owner, None, draft("", "Call the plumber\nabout the sink"))
        .unwrap();
    assert_eq!(note.title, "Call the plumber");
    assert_eq!(note.thread, None);
    let changed = tokidoki
        .write_note(&owner, Some(note.id), draft("Plumber", "Tuesday, 9am"))
        .unwrap();
    assert_eq!(changed.id, note.id);
    let search = interface::NoteSearch {
        query: "tuesday".into(),
        ..Default::default()
    };
    assert_eq!(tokidoki.notes(&owner, search).unwrap(), [changed]);

    let empty = tokidoki
        .write_note(&owner, None, draft(" ", ""))
        .unwrap_err();
    assert!(empty.downcast::<Rejected>().is_ok());
    assert!(tokidoki.notes(&stranger, Default::default()).is_err());
    assert!(tokidoki.delete_note(&stranger, note.id).is_err());
    tokidoki.delete_note(&owner, note.id).unwrap();
    let again = tokidoki.delete_note(&owner, note.id).unwrap_err();
    assert!(again.downcast::<Rejected>().is_ok());
    assert!(
        tokidoki
            .notes(&owner, Default::default())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn the_timeline_is_every_threads_turns_as_one_conversation() {
    let (tokidoki, _) = start(vec![
        Reply::Text("one"),
        Reply::Text("two"),
        Reply::Text("three"),
    ]);
    for (thread, text) in [("a", "first"), ("b", "second"), ("a", "third")] {
        let input = Input {
            thread: Some(thread.into()),
            ..me(text)
        };
        let sent = tokidoki.send(input).await.unwrap();
        settled(&tokidoki, &sent.submission).await;
        // Turns are ordered by the millisecond they started in: no two in the same one.
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    // What the user said, in order, and in which thread.
    let said = |events: Vec<Event>| -> Vec<(String, String)> {
        events
            .into_iter()
            .filter_map(|event| match event {
                Event::Checkpoint {
                    thread,
                    change:
                        Change::Message {
                            role: interface::Role::User,
                            content,
                        },
                    ..
                } => match &content[0] {
                    Content::Text { text } => Some((thread, text.clone())),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    };
    let pair = |thread: &str, text: &str| (thread.to_string(), text.to_string());
    let all = tokidoki.timeline(None, 10).unwrap();
    assert_eq!(
        said(all.clone()),
        [pair("a", "first"), pair("b", "second"), pair("a", "third")]
    );
    // Each turn whole: from its start to its answer.
    assert!(all.iter().any(|e| matches!(
        e,
        Event::Checkpoint {
            change: Change::TurnEnded,
            ..
        }
    )));

    let latest = tokidoki.timeline(None, 2).unwrap();
    assert_eq!(
        said(latest.clone()),
        [pair("b", "second"), pair("a", "third")]
    );
    let Some(Event::Checkpoint { at, .. }) = latest.first() else {
        panic!("no turns");
    };
    let earlier = tokidoki.timeline(Some(*at), 2).unwrap();
    assert_eq!(said(earlier), [pair("a", "first")]);

    let threads = tokidoki.threads().unwrap();
    assert!(
        threads
            .iter()
            .all(|t| t.phase == interface::ThreadPhase::Hot)
    );
}

/// Answers a research worker with `found`; everything else from `script`, in order.
struct Split {
    script: Script,
    found: &'static str,
}

impl Model for Split {
    async fn chat(&self, ctx: &Context) -> Result<Response> {
        let worker = ctx.system.as_deref() == Some(worker_tools::RESEARCH_PROMPT);
        if !worker {
            return self.script.chat(ctx).await;
        }
        Ok(Response {
            message: Message::assistant(self.found),
            input_tokens: 0,
            output_tokens: 0,
            stop: inference::Stop::Done,
        })
    }
}

/// Research handed to a worker comes back to the conversation by itself; the worker's thread
/// stays out of it.
#[tokio::test]
async fn research_goes_to_a_worker_and_comes_back() {
    let (parts, _) = parts(Vec::new(), Sqlite::memory().unwrap());
    let script = Script {
        replies: Mutex::new(
            vec![
                Reply::Call(
                    "delegate",
                    json!({ "task": "Find the population of Lisbon." }),
                ),
                Reply::Text("I'm on it."),
                Reply::Text("Lisbon has about 545,000 people."),
            ]
            .into(),
        ),
        ..Script::default()
    };
    let model = Arc::new(Split {
        script,
        found: "About 545,000 people (2024). Sources: https://example.com",
    });
    let parts = Parts {
        model,
        exa: Some(exa::Exa::new("unused").base("http://127.0.0.1:9")),
        store: parts.store,
        mind: parts.mind,
        router: parts.router,
        memory: parts.memory,
        scope: parts.scope,
        notes: parts.notes,
        tools: parts.tools,
        system: parts.system,
        settings: parts.settings,
        speakers: parts.speakers,
        journal: parts.journal,
        review: parts.review,
        voice: parts.voice,
    };
    let tokidoki = Tokidoki::build(parts).unwrap();
    let mut events = tokidoki.events();
    let sent = tokidoki
        .send(me("How many people live in Lisbon?"))
        .await
        .unwrap();
    assert_eq!(
        settled(&tokidoki, &sent.submission).await.answer.as_deref(),
        Some("I'm on it.")
    );

    let delivered = |e: &Event| matches!(e, Event::Worker { worker } if worker.delivery.is_some());
    let seen = until(&mut events, delivered).await;
    let Some(Event::Worker { worker }) = seen.last() else {
        unreachable!()
    };
    assert_eq!(worker.status, interface::WorkerStatus::Done);
    assert_eq!(worker.delivery, Some(interface::Delivery::Delivered));
    assert_eq!(worker.parent, sent.thread);
    assert!(worker.report.as_deref().unwrap().contains("545,000"));
    let answered = |e: &Event| {
        matches!(e, Event::Checkpoint { change: Change::Message { role: interface::Role::Assistant, content }, .. }
        if matches!(&content[0], Content::Text { text } if text.contains("Lisbon has about")))
    };
    until(&mut events, answered).await;

    let owner = Speaker::owner();
    assert_eq!(tokidoki.workers(&owner).unwrap().len(), 1);
    let threads: Vec<String> = tokidoki
        .threads()
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(threads, std::slice::from_ref(&sent.thread));
    let timeline = tokidoki.timeline(None, 10).unwrap();
    assert!(
        timeline
            .iter()
            .all(|e| matches!(e, Event::Checkpoint { thread, .. } if *thread == sent.thread))
    );
}

/// A research question on real models (Anthropic's cheapest, with Jev and Exa: their keys in
/// the environment): handed to a worker, answered at once with "on it", and the worker's
/// sourced report comes back by itself.
#[tokio::test]
#[ignore = "calls Anthropic, Jev and Exa"]
async fn research_on_real_models() {
    let root = Scratch::new("research");
    let key = std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY");
    let mut config = Config::new(&root.0, Vendor::Anthropic, key).with_env();
    let (model, router_model) = Config::cheap_models(Vendor::Anthropic);
    (config.model, config.router_model) = (model.into(), router_model.into());
    config.journal = false;
    assert!(
        config.exa_key.is_some() && config.jev_key.is_some(),
        "set EXA_API_KEY and TYPESAFE_API_KEY"
    );
    let tokidoki = Tokidoki::open(config, Tools::new()).unwrap();
    let mut events = tokidoki.events();
    let started = std::time::Instant::now();
    let sent = tokidoki
        .send(me(
            "Could you look into what the current population of Lisbon is, with a source?",
        ))
        .await
        .unwrap();
    let first = settled(&tokidoki, &sent.submission).await;
    println!(
        "first answer in {:?}: {:?}",
        started.elapsed(),
        first.answer
    );

    let delivered = |e: &Event| matches!(e, Event::Worker { worker } if worker.delivery.is_some());
    let seen = tokio::time::timeout(Duration::from_secs(180), until_long(&mut events, delivered))
        .await
        .expect("no report within 3 minutes");
    let Some(Event::Worker { worker }) = seen.last() else {
        unreachable!()
    };
    println!(
        "worker {} {:?} in {:?}:\n{}",
        worker.id,
        worker.delivery,
        started.elapsed(),
        worker.report.as_deref().unwrap_or("")
    );
    assert_eq!(worker.status, interface::WorkerStatus::Done);
    assert!(
        worker.report.as_deref().unwrap().contains("http"),
        "the report cites its sources"
    );

    let answered = |e: &Event| matches!(e, Event::Submission { submission } if submission.thread == sent.thread && submission.id != sent.submission && submission.status == Status::Done);
    let seen = tokio::time::timeout(Duration::from_secs(60), until_long(&mut events, answered))
        .await
        .unwrap();
    let Some(Event::Submission { submission }) = seen.last() else {
        unreachable!()
    };
    println!(
        "told the user:\n{}",
        submission.answer.as_deref().unwrap_or("")
    );
    tokidoki.close().await.unwrap();
}

/// `until`, without its short timeout.
async fn until_long(events: &mut Events, last: impl Fn(&Event) -> bool) -> Vec<Event> {
    let mut seen = Vec::new();
    while let Some(event) = events.recv().await {
        let done = last(&event);
        seen.push(event);
        if done {
            return seen;
        }
    }
    seen
}
