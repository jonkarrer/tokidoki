//! The user's notes, for the model: three tools and what the system prompt says of them.
//! The model saves a note when the user asks it to, and reads them when the user refers to
//! them; otherwise notes stay out of the conversation (memory is what it keeps on its own).

use std::sync::Arc;

use agent::{Call, Replay, Tool, ToolFuture, Tools};
use anyhow::{Context as _, bail};
use notes::{Draft, Notes, Origin, Search};
use serde_json::{Value, json};

/// What the system prompt says of notes, beside the tools.
pub const PROMPT: &str = "Notes are the user's own, kept apart from memory. Use the notes \
tools only when the user asks: \"save this as a note\" or \"save these notes: …\" means \
save_note, once, with the text as Markdown, every detail kept and nothing added (tidy it into \
headings and lists if it is a rough dump), and a short title, tags, project and people if the \
text makes them clear; then say in a few words what was saved. \"Check my notes\" or \"what \
did I write about …\" means search_notes (then get_note for a whole note) and answering from \
what they say. \"Remember that …\" is memory, not a note: memory needs no tool.";

/// Adds the note tools, over `notes`, to `tools`.
pub fn add(tools: &mut Tools, notes: &Arc<Notes>) {
    tools
        .add(SaveNote(notes.clone()))
        .add(SearchNotes(notes.clone()))
        .add(GetNote(notes.clone()));
}

/// How much of a note's body a search shows; `get_note` has the rest.
const SEARCH_BODY_CHARS: usize = 1_200;

struct SaveNote(Arc<Notes>);

impl Tool for SaveNote {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "save_note".into(),
            description: "Saves a note for the user, only when they ask for one.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string", "description": "Short; a few words." },
                    "body": { "type": "string", "description": "The note, in Markdown." },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "project": { "type": "string" },
                    "people": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["title", "body"]
            }),
        }
    }

    // Saving from the same call twice keeps one note (`Origin`).
    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let args = &call.arguments;
            let draft = Draft {
                title: text(args, "title"),
                body: text(args, "body"),
                tags: list(args, "tags"),
                project: Some(text(args, "project")),
                people: list(args, "people"),
            };
            let origin = Origin {
                thread: call.thread,
                call: call.id,
            };
            let note = self.0.add(draft, Some(origin))?;
            Ok(format!("Saved note {}: {}", note.id, note.title))
        })
    }
}

struct SearchNotes(Arc<Notes>);

impl Tool for SearchNotes {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "search_notes".into(),
            description: "Searches the user's notes, best match first. Without a query, the \
                          most recently changed. Long notes are cut short: get_note has them \
                          whole."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words to look for." },
                    "tag": { "type": "string" },
                    "project": { "type": "string" },
                    "limit": { "type": "integer", "description": "At most 20; 5 if not given." }
                }
            }),
        }
    }

    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let args = &call.arguments;
            let present = |key| Some(text(args, key)).filter(|t| !t.is_empty());
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5);
            let found = self.0.search(&Search {
                query: text(args, "query"),
                tag: present("tag"),
                project: present("project"),
                limit: limit.clamp(1, 20) as usize,
            })?;
            if found.is_empty() {
                return Ok("No notes found.".into());
            }
            let shown: Vec<String> = found
                .iter()
                .map(|note| shown(note, Some(SEARCH_BODY_CHARS)))
                .collect();
            Ok(shown.join("\n\n---\n\n"))
        })
    }
}

struct GetNote(Arc<Notes>);

impl Tool for GetNote {
    fn spec(&self) -> context::Tool {
        context::Tool {
            name: "get_note".into(),
            description: "One of the user's notes, whole, by its ID.".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"]
            }),
        }
    }

    fn replay(&self) -> Replay {
        Replay::Safe
    }

    fn execute(&self, call: Call) -> ToolFuture<'_> {
        Box::pin(async move {
            let id = call.arguments.get("id");
            // A model may send the ID as a string.
            let id = id
                .and_then(|id| id.as_i64().or_else(|| id.as_str()?.trim().parse().ok()))
                .context("get_note needs a note's id")?;
            match self.0.get(id)? {
                Some(note) => Ok(shown(&note, None)),
                None => bail!("there is no note {id}"),
            }
        })
    }
}

/// A note for the model: what it is about, then its body, cut to `cut` characters if given.
fn shown(note: &notes::Note, cut: Option<usize>) -> String {
    let mut lines = vec![format!("Note {}: {}", note.id, note.title)];
    if !note.tags.is_empty() {
        lines.push(format!("Tags: {}", note.tags.join(", ")));
    }
    if let Some(project) = &note.project {
        lines.push(format!("Project: {project}"));
    }
    if !note.people.is_empty() {
        lines.push(format!("People: {}", note.people.join(", ")));
    }
    let day = |ms: u64| telemetry::time::clock(ms * 1000)[..10].to_string();
    lines.push(format!("Changed: {}", day(note.updated_at)));
    let body = match cut.and_then(|cut| note.body.char_indices().nth(cut)) {
        Some((at, _)) => format!("{}… (cut short; get_note has it all)", &note.body[..at]),
        None => note.body.clone(),
    };
    format!("{}\n\n{body}", lines.join("\n"))
}

/// A string argument, or empty.
fn text(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// A list argument: an array of strings, or (some models write one) a comma-separated string.
fn list(args: &Value, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        Some(Value::String(text)) => text.split(',').map(String::from).collect(),
        _ => Vec::new(),
    }
}

/// A note as the protocol shows it.
pub(crate) fn note(note: notes::Note) -> interface::Note {
    interface::Note {
        id: note.id,
        title: note.title,
        body: note.body,
        tags: note.tags,
        project: note.project,
        people: note.people,
        thread: note.thread,
        created_at: note.created_at,
        updated_at: note.updated_at,
    }
}

/// What the notes store just did, as an event.
pub(crate) fn change(change: notes::Change) -> interface::Event {
    let change = match change {
        notes::Change::Saved(saved) => interface::NoteChange::Saved { note: note(saved) },
        notes::Change::Deleted(id) => interface::NoteChange::Deleted { id },
    };
    interface::Event::Note { change }
}

pub(crate) fn draft(draft: interface::NoteDraft) -> Draft {
    Draft {
        title: draft.title,
        body: draft.body,
        tags: draft.tags,
        project: draft.project,
        people: draft.people,
    }
}
