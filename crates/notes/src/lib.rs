//! The user's notes: what they want kept as they wrote it, apart from memory. Memory is what
//! tokidoki learns and tidies on its own; a note is the user's, saved when they ask (in the
//! app, or by telling the model "save these notes") and changed only by them. Nothing reads
//! notes into a conversation unless it is asked to.
//!
//! A note has a title, a Markdown body, tags, a project and the people it concerns. Search is
//! keyword search (FTS5, BM25) over all of it, the same way memory is searched. A note saved
//! from a conversation names its thread, and the tool call that saved it (`Origin`): saving
//! from the same call again returns that note instead of a second one, so a tool cut off by a
//! crash can simply run again.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub project: Option<String>,
    pub people: Vec<String>,
    /// The thread it was saved from; none for one the user wrote.
    pub thread: Option<String>,
    /// Unix milliseconds.
    pub created_at: u64,
    pub updated_at: u64,
}

/// A note as written, before it is saved.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    /// Empty takes the body's first line.
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
    pub project: Option<String>,
    pub people: Vec<String>,
}

/// Where a note saved from a conversation came from: the thread, and the call that saved it.
#[derive(Debug, Clone, PartialEq)]
pub struct Origin {
    pub thread: String,
    pub call: String,
}

/// What to look for. An empty query lists the most recently changed first.
#[derive(Debug, Clone, Default)]
pub struct Search {
    pub query: String,
    /// Only notes with this tag, or in this project (either ignoring case).
    pub tag: Option<String>,
    pub project: Option<String>,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Saved(Note),
    Deleted(i64),
}

type Watcher = Box<dyn Fn(Change) + Send + Sync>;

pub struct Notes {
    conn: Mutex<Connection>,
    watcher: OnceLock<Watcher>,
}

/// How many matches a search ranks before filtering by tag or project.
const CANDIDATES: usize = 200;

/// How long a title taken from the body may be.
const TITLE_CHARS: usize = 60;

impl Notes {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        Self::init(conn)
    }

    /// Gone when dropped; for tests.
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS notes (
                 id INTEGER PRIMARY KEY,
                 title TEXT NOT NULL,
                 body TEXT NOT NULL,
                 tags TEXT NOT NULL,
                 project TEXT,
                 people TEXT NOT NULL,
                 thread TEXT,
                 origin TEXT UNIQUE,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                 title, body, about,
                 tokenize = 'porter unicode61'
             );",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
            watcher: OnceLock::new(),
        })
    }

    /// Calls `watcher` with every change from now on. One watcher; a second is ignored.
    pub fn watch(&self, watcher: impl Fn(Change) + Send + Sync + 'static) {
        let _ = self.watcher.set(Box::new(watcher));
    }

    fn changed(&self, change: Change) {
        if let Some(watcher) = self.watcher.get() {
            watcher(change);
        }
    }

    /// Saves a new note. From a conversation, `origin` names the call that saved it, and
    /// saving from that call again returns the note already saved.
    pub fn add(&self, draft: Draft, origin: Option<Origin>) -> Result<Note> {
        let draft = tidy(draft)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let key = origin.as_ref().map(|o| format!("{}/{}", o.thread, o.call));
        if let Some(key) = &key
            && let Some(note) = tx
                .query_row("SELECT * FROM notes WHERE origin = ?1", [key], note_from)
                .optional()?
        {
            return Ok(note);
        }
        let now = now();
        tx.execute(
            "INSERT INTO notes (title, body, tags, project, people, thread, origin, created_at,
                 updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
            params![
                draft.title,
                draft.body,
                serde_json::to_string(&draft.tags)?,
                draft.project,
                serde_json::to_string(&draft.people)?,
                origin.map(|o| o.thread),
                key,
                now as i64,
            ],
        )?;
        let id = tx.last_insert_rowid();
        index(&tx, id, &draft)?;
        let note = tx.query_row("SELECT * FROM notes WHERE id = ?1", [id], note_from)?;
        tx.commit()?;
        drop(conn);
        tracing::info!(id, title = %note.title, thread = ?note.thread, "note saved");
        self.changed(Change::Saved(note.clone()));
        Ok(note)
    }

    /// Replaces the note `id`'s contents with `draft`; where it came from stays.
    pub fn update(&self, id: i64, draft: Draft) -> Result<Note> {
        let draft = tidy(draft)?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let Some(before) = tx
            .query_row("SELECT * FROM notes WHERE id = ?1", [id], note_from)
            .optional()?
        else {
            bail!("note {id} is gone");
        };
        // Always later than before, so "most recently changed" holds within a millisecond.
        let now = now().max(before.updated_at + 1);
        tx.execute(
            "UPDATE notes SET title = ?2, body = ?3, tags = ?4, project = ?5, people = ?6,
                 updated_at = ?7
             WHERE id = ?1",
            params![
                id,
                draft.title,
                draft.body,
                serde_json::to_string(&draft.tags)?,
                draft.project,
                serde_json::to_string(&draft.people)?,
                now as i64,
            ],
        )?;
        index(&tx, id, &draft)?;
        let note = tx.query_row("SELECT * FROM notes WHERE id = ?1", [id], note_from)?;
        tx.commit()?;
        drop(conn);
        tracing::info!(id, title = %note.title, "note changed");
        self.changed(Change::Saved(note.clone()));
        Ok(note)
    }

    pub fn get(&self, id: i64) -> Result<Option<Note>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT * FROM notes WHERE id = ?1", [id], note_from)
            .optional()?)
    }

    /// Removes the note `id` for good. Returns whether there was one.
    pub fn delete(&self, id: i64) -> Result<bool> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let removed = tx.execute("DELETE FROM notes WHERE id = ?1", [id])? > 0;
        tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [id])?;
        tx.commit()?;
        drop(conn);
        if removed {
            tracing::info!(id, "note deleted");
            self.changed(Change::Deleted(id));
        }
        Ok(removed)
    }

    /// Notes matching `search`: best match first, or, for an empty query, most recently
    /// changed first.
    pub fn search(&self, search: &Search) -> Result<Vec<Note>> {
        let conn = self.conn.lock().unwrap();
        let found: Vec<Note> = if search.query.trim().is_empty() {
            let mut select = conn
                .prepare_cached("SELECT * FROM notes ORDER BY updated_at DESC, id DESC LIMIT ?1")?;
            select
                .query_map([CANDIDATES as i64], note_from)?
                .collect::<Result<_, _>>()?
        } else {
            let Some(terms) = memory::fts_query(&search.query) else {
                return Ok(Vec::new());
            };
            let mut select = conn.prepare_cached(
                "SELECT notes.* FROM notes_fts JOIN notes ON notes.id = notes_fts.rowid
                 WHERE notes_fts MATCH ?1 ORDER BY bm25(notes_fts) LIMIT ?2",
            )?;
            select
                .query_map(params![terms, CANDIDATES as i64], note_from)?
                .collect::<Result<_, _>>()?
        };
        let same = |a: &str, b: &str| a.eq_ignore_ascii_case(b.trim());
        Ok(found
            .into_iter()
            .filter(|note| {
                let tag = search.tag.as_deref();
                let project = search.project.as_deref();
                tag.is_none_or(|tag| note.tags.iter().any(|t| same(t, tag)))
                    && project.is_none_or(|p| note.project.as_deref().is_some_and(|n| same(n, p)))
            })
            .take(search.limit.max(1))
            .collect())
    }
}

/// A draft as it is kept: trimmed, a title from the body's first line when it has none, and
/// tags and people without blanks or repeats (ignoring case).
fn tidy(draft: Draft) -> Result<Draft> {
    let body = draft.body.trim().to_string();
    let mut title = draft.title.trim().to_string();
    if title.is_empty() {
        title = first_line(&body);
    }
    if title.is_empty() {
        bail!("a note needs a title or some text");
    }
    let project = draft.project.map(|p| p.trim().to_string());
    Ok(Draft {
        title,
        body,
        tags: distinct(draft.tags),
        project: project.filter(|p| !p.is_empty()),
        people: distinct(draft.people),
    })
}

/// The body's first line with its Markdown marks taken off, cut to `TITLE_CHARS`.
fn first_line(body: &str) -> String {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty());
    let line = line.unwrap_or_default();
    let bare = line
        .trim_start_matches(['#', '>', '-', '*', '+', ' '])
        .replace(['*', '_', '`'], "");
    let bare = bare.trim();
    match bare.char_indices().nth(TITLE_CHARS) {
        Some((cut, _)) => format!("{}…", bare[..cut].trim_end()),
        None => bare.to_string(),
    }
}

fn distinct(values: Vec<String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for value in values {
        let value = value.trim();
        if !value.is_empty() && !kept.iter().any(|k| k.eq_ignore_ascii_case(value)) {
            kept.push(value.to_string());
        }
    }
    kept
}

/// Keeps the search index in step with the note `id`.
fn index(tx: &rusqlite::Transaction, id: i64, draft: &Draft) -> Result<()> {
    let about = [
        draft.tags.join(" "),
        draft.project.clone().unwrap_or_default(),
        draft.people.join(" "),
    ]
    .join(" ");
    tx.execute("DELETE FROM notes_fts WHERE rowid = ?1", [id])?;
    tx.execute(
        "INSERT INTO notes_fts (rowid, title, body, about) VALUES (?1, ?2, ?3, ?4)",
        params![id, draft.title, draft.body, about],
    )?;
    Ok(())
}

fn note_from(row: &Row) -> rusqlite::Result<Note> {
    let list = |column: &str| -> rusqlite::Result<Vec<String>> {
        let json: String = row.get(column)?;
        Ok(serde_json::from_str(&json).unwrap_or_default())
    };
    Ok(Note {
        id: row.get("id")?,
        title: row.get("title")?,
        body: row.get("body")?,
        tags: list("tags")?,
        project: row.get("project")?,
        people: list("people")?,
        thread: row.get("thread")?,
        created_at: row.get::<_, i64>("created_at")? as u64,
        updated_at: row.get::<_, i64>("updated_at")? as u64,
    })
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(title: &str, body: &str) -> Draft {
        Draft {
            title: title.into(),
            body: body.into(),
            ..Draft::default()
        }
    }

    fn from_call(call: &str) -> Option<Origin> {
        Some(Origin {
            thread: "thread-1".into(),
            call: call.into(),
        })
    }

    #[test]
    fn a_note_without_a_title_takes_its_first_line() {
        let notes = Notes::in_memory().unwrap();
        let note = notes
            .add(
                draft("", "\n## **Dentist** on Friday\n\nBring the forms."),
                None,
            )
            .unwrap();
        assert_eq!(note.title, "Dentist on Friday");
        assert_eq!(note.body, "## **Dentist** on Friday\n\nBring the forms.");
    }

    #[test]
    fn a_note_needs_something_in_it() {
        let notes = Notes::in_memory().unwrap();
        assert!(notes.add(draft(" ", "\n"), None).is_err());
    }

    #[test]
    fn tags_and_people_are_kept_once_each() {
        let notes = Notes::in_memory().unwrap();
        let note = notes
            .add(
                Draft {
                    tags: vec!["Work".into(), "work ".into(), "".into(), "q3".into()],
                    people: vec!["Ana".into(), "ana".into()],
                    project: Some("  ".into()),
                    ..draft("Plan", "")
                },
                None,
            )
            .unwrap();
        assert_eq!(note.tags, ["Work", "q3"]);
        assert_eq!(note.people, ["Ana"]);
        assert_eq!(note.project, None);
    }

    #[test]
    fn saving_again_from_the_same_call_keeps_one_note() {
        let notes = Notes::in_memory().unwrap();
        let first = notes
            .add(draft("Groceries", "oat milk"), from_call("c1"))
            .unwrap();
        let again = notes
            .add(draft("Groceries", "oat milk"), from_call("c1"))
            .unwrap();
        let other = notes
            .add(draft("Groceries", "oat milk"), from_call("c2"))
            .unwrap();
        assert_eq!(first, again);
        assert_ne!(first.id, other.id);
        assert_eq!(first.thread.as_deref(), Some("thread-1"));
    }

    #[test]
    fn search_finds_words_anywhere_in_a_note() {
        let notes = Notes::in_memory().unwrap();
        let tagged = Draft {
            tags: vec!["travel".into()],
            people: vec!["Ana".into()],
            ..draft("Lisbon", "Flights on the 3rd")
        };
        notes.add(tagged, None).unwrap();
        notes
            .add(draft("Groceries", "oat milk, lentils"), None)
            .unwrap();
        let find = |query: &str| -> Vec<String> {
            let search = Search {
                query: query.into(),
                limit: 5,
                ..Search::default()
            };
            notes
                .search(&search)
                .unwrap()
                .into_iter()
                .map(|n| n.title)
                .collect()
        };
        assert_eq!(find("flights"), ["Lisbon"]);
        assert_eq!(find("Ana's trip"), ["Lisbon"]);
        assert_eq!(find("lentil"), ["Groceries"]);
        assert!(find("dentist").is_empty());
    }

    #[test]
    fn an_empty_search_lists_the_latest_changed_first_within_a_tag() {
        let notes = Notes::in_memory().unwrap();
        let work = |title: &str| Draft {
            tags: vec!["work".into()],
            ..draft(title, "")
        };
        let first = notes.add(work("First"), None).unwrap();
        notes.add(work("Second"), None).unwrap();
        notes.add(draft("Elsewhere", ""), None).unwrap();
        notes.update(first.id, work("First, revised")).unwrap();
        let search = Search {
            tag: Some("WORK".into()),
            limit: 10,
            ..Search::default()
        };
        let titles: Vec<String> = notes
            .search(&search)
            .unwrap()
            .into_iter()
            .map(|n| n.title)
            .collect();
        assert_eq!(titles, ["First, revised", "Second"]);
    }

    #[test]
    fn a_change_keeps_where_the_note_came_from_and_is_searched_anew() {
        let notes = Notes::in_memory().unwrap();
        let note = notes
            .add(draft("Plan", "old words"), from_call("c1"))
            .unwrap();
        let changed = notes.update(note.id, draft("Plan", "new words")).unwrap();
        assert_eq!(changed.thread.as_deref(), Some("thread-1"));
        assert_eq!(changed.created_at, note.created_at);
        assert!(changed.updated_at > note.updated_at);
        let search = |query: &str| Search {
            query: query.into(),
            limit: 5,
            ..Search::default()
        };
        assert!(notes.search(&search("old")).unwrap().is_empty());
        assert_eq!(notes.search(&search("new")).unwrap().len(), 1);
    }

    #[test]
    fn a_deleted_note_is_gone_from_search_and_watchers_hear_of_it() {
        let notes = Notes::in_memory().unwrap();
        let heard = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = heard.clone();
        notes.watch(move |change| log.lock().unwrap().push(change));
        let note = notes.add(draft("Plan", "words"), None).unwrap();
        assert!(notes.delete(note.id).unwrap());
        assert!(!notes.delete(note.id).unwrap());
        assert!(notes.get(note.id).unwrap().is_none());
        let search = Search {
            query: "words".into(),
            limit: 5,
            ..Search::default()
        };
        assert!(notes.search(&search).unwrap().is_empty());
        assert_eq!(
            *heard.lock().unwrap(),
            [Change::Saved(note.clone()), Change::Deleted(note.id)]
        );
    }
}
