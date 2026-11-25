//! Tokidoki, the app: the `tokidoki` library in a window.
//!
//! The core runs in this process. The window's page calls the commands below, one per
//! `Tokidoki` method, and follows the `tokidoki:event` events, which are `interface::Event`s
//! as JSON: the page is a client of the same protocol anything else would speak.
//!
//! The user chooses whose models it runs on and gives their key for it (`settings`, `keys`).
//! Without the key the core opens asleep: what it has can be read, and nothing runs. It
//! keeps running while the window is closed, so idle threads still dream and unfinished work
//! still finishes; quitting stops it, and the next launch resumes what was cut off.

mod browser;
mod debug;
mod keys;
mod passport;
mod settings;
mod updater;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::menu::{Menu, MenuItem, MenuItemKind};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent, Wry};
use tokidoki::interface::{
    Event, Feedback, Heard, Input, Memory, Note, NoteDraft, NoteSearch, Sent, Speaker,
    ThreadSummary, VoiceStatus, WhenBusy, Worker,
};
use tokidoki::{Config, Tokidoki, Tools, Vendor};

use settings::Settings;

/// The running core, once it has opened.
#[derive(Default)]
struct Core {
    /// Held while opening, so two callers never open the same files twice.
    running: Mutex<Option<Running>>,
}

#[derive(Clone)]
struct Running {
    tokidoki: Tokidoki,
    /// Whether it has a key to run with; asleep otherwise.
    awake: bool,
}

/// Where the app is, for the page to choose what to show.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Status {
    /// No key for the chosen provider yet: what there is can be read, nothing can be sent.
    NeedsKeys,
    Ready,
    /// The core didn't open.
    Failed {
        message: String,
    },
}

type Said<T> = Result<T, String>;

/// An error as the page shows it: the whole chain, on one line.
fn say(error: anyhow::Error) -> String {
    format!("{error:#}")
}

fn core(app: &AppHandle) -> Said<Tokidoki> {
    let running = app.state::<Core>().running.lock().unwrap().clone();
    running
        .map(|running| running.tokidoki)
        .ok_or_else(|| "Tokidoki isn't running yet.".to_string())
}

/// Where the databases live: `TOKIDOKI_DATA` if set (a scratch run), else the app's data
/// folder. Development builds keep theirs apart, so `just app` never touches what the
/// installed app remembers.
fn data_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("TOKIDOKI_DATA").filter(|dir| !dir.is_empty()) {
        return Ok(dir.into());
    }
    let dir = app.path().app_data_dir()?;
    Ok(match cfg!(debug_assertions) {
        true => dir.join("dev"),
        false => dir,
    })
}

/// Where the voice models are kept: one folder for the installed app and `just app` alike,
/// since they are ~900 MB. `TOKIDOKI_VOICE_MODELS` puts them elsewhere.
fn models_dir(app: &AppHandle) -> anyhow::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("TOKIDOKI_VOICE_MODELS").filter(|dir| !dir.is_empty()) {
        return Ok(dir.into());
    }
    Ok(app.path().app_data_dir()?.join("models"))
}

/// What each provider is called, and where its keys are made.
fn named(provider: Vendor) -> (&'static str, &'static str) {
    match provider {
        Vendor::Anthropic => ("Anthropic", "console.anthropic.com"),
        Vendor::Fireworks => ("Fireworks", "app.fireworks.ai"),
        Vendor::Baseten => ("Baseten", "app.baseten.co"),
    }
}

/// Opens the core as the settings say, unless it is open, and passes its events on to the
/// page from then on. Whether it has a key to run with.
fn open(app: &AppHandle) -> anyhow::Result<bool> {
    let core = app.state::<Core>();
    let mut running = core.running.lock().unwrap();
    if let Some(running) = &*running {
        return Ok(running.awake);
    }
    let data = data_dir(app)?;
    let settings = Settings::load(&data);
    let provider = settings.provider();
    let key = keys::get(provider)?;
    let mut config = Config::new(data, provider, key.clone().unwrap_or_default());
    if let Some(model) = settings.model.filter(|model| !model.is_empty()) {
        config.model = model;
    }
    // With a TypeSafe key, Jev files messages into threads, before the model would.
    config.jev_key = keys::jev()?;
    // With an Exa key, research can be handed to workers that search the web.
    config.exa_key = keys::exa()?;
    // A window of its own for a mission to browse in.
    config.browser = Some(std::sync::Arc::new(browser::Page::new(app.clone())));
    // The voice models, shared by the installed app and development (they are big).
    config.voice_models = Some(models_dir(app)?);
    // Development: `TOKIDOKI_MODELS=cheap`, or a model by name.
    let config = config.with_models_from_env();
    let tokidoki = match key {
        Some(_) => Tokidoki::open(config, Tools::new())?,
        None => {
            let why = format!(
                "Add your {} API key to talk to Tokidoki.",
                named(provider).0
            );
            Tokidoki::open_asleep(config, why)?
        }
    };
    let mut events = tokidoki.events();
    let awake = key.is_some();
    *running = Some(Running {
        tokidoki: tokidoki.clone(),
        awake,
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Event::Worker { worker } = &event
                && worker.status != tokidoki::interface::WorkerStatus::Running
            {
                put_browser_away(&app, &tokidoki);
            }
            let _ = app.emit("tokidoki:event", event);
        }
    });
    Ok(awake)
}

/// A worker ended: with no mission still out, the browser's window is put out of the way
/// (its page and logins stay as they are, for the next one).
fn put_browser_away(app: &AppHandle, tokidoki: &Tokidoki) {
    let out = tokidoki.workers(&Speaker::owner()).is_ok_and(|workers| {
        let running = tokidoki::interface::WorkerStatus::Running;
        // Missions and learning browse; research doesn't.
        (workers.iter()).any(|worker| worker.kind != "research" && worker.status == running)
    });
    if !out {
        browser::put_away(app);
    }
}

/// Stops the core: running work resumes at the next launch.
pub(crate) async fn close_core(app: &AppHandle) {
    let running = app.state::<Core>().running.lock().unwrap().take();
    if let Some(running) = running
        && let Err(error) = running.tokidoki.close().await
    {
        eprintln!("closing tokidoki: {error:#}");
    }
}

/// Opens the core if it isn't, and says where things stand.
#[tauri::command]
async fn status(app: AppHandle) -> Status {
    match open(&app) {
        Ok(true) => Status::Ready,
        Ok(false) => Status::NeedsKeys,
        Err(error) => Status::Failed {
            message: say(error),
        },
    }
}

/// A provider the user can choose, for the form.
#[derive(Serialize)]
struct ProviderChoice {
    id: &'static str,
    name: &'static str,
    /// Where its keys are made.
    keys_at: &'static str,
    /// The model that answers unless the user names another.
    model: &'static str,
    /// Whether the user has given a key for it: never the key itself.
    has_key: bool,
}

/// What the user has chosen, and what there is to choose from.
#[derive(Serialize)]
struct Chosen {
    provider: &'static str,
    /// The model the user named, if not the provider's usual.
    model: Option<String>,
    providers: Vec<ProviderChoice>,
    /// Whether the user has given a TypeSafe key, for Jev: never the key itself.
    has_jev_key: bool,
    /// Whether the user has given an Exa key, for research workers.
    has_exa_key: bool,
}

#[tauri::command]
fn chosen(app: AppHandle) -> Said<Chosen> {
    let settings = Settings::load(&data_dir(&app).map_err(say)?);
    let providers = Vendor::ALL.into_iter().map(|provider| {
        let (name, keys_at) = named(provider);
        Ok(ProviderChoice {
            id: provider.id(),
            name,
            keys_at,
            model: Config::models(provider).0,
            has_key: keys::get(provider)?.is_some(),
        })
    });
    Ok(Chosen {
        provider: settings.provider().id(),
        model: settings.model.clone(),
        providers: providers.collect::<anyhow::Result<_>>().map_err(say)?,
        has_jev_key: keys::jev().map_err(say)?.is_some(),
        has_exa_key: keys::exa().map_err(say)?.is_some(),
    })
}

/// Sets whose models Tokidoki runs on: checks `key` with the provider and stores it in the
/// Keychain (`None` keeps the key there is), remembers the choice, and starts the core over
/// with it. `model` names the model that answers; empty for the provider's usual. `jev_key`,
/// checked and stored the same way, lets Jev make the quick decisions, and `exa_key`, stored, lets
/// research go to workers (`None` keeps the one there is, if any).
#[tauri::command]
async fn choose(
    app: AppHandle,
    provider: String,
    key: Option<String>,
    model: Option<String>,
    jev_key: Option<String>,
    exa_key: Option<String>,
) -> Said<Status> {
    let jev_key = jev_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    if let Some(jev_key) = &jev_key {
        check_jev_key(jev_key).await?;
    }
    let provider: Vendor = provider.parse().map_err(say)?;
    let key = key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty());
    match &key {
        Some(key) => check_key(provider, key).await?,
        None if keys::get(provider).map_err(say)?.is_none() => {
            return Err(format!(
                "Tokidoki needs your {} API key.",
                named(provider).0
            ));
        }
        None => {}
    }
    if let Some(key) = &key {
        keys::set(provider, key).map_err(say)?;
    }
    if let Some(jev_key) = &jev_key {
        keys::set_jev(jev_key).map_err(say)?;
    }
    // Exa has no free way to check a key: a wrong one shows as a research worker that fails.
    if let Some(exa_key) = exa_key
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
    {
        keys::set_exa(&exa_key).map_err(say)?;
    }
    let settings = Settings {
        provider: Some(provider.id().into()),
        model: model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty()),
    };
    settings.save(&data_dir(&app).map_err(say)?).map_err(say)?;
    // Over again with the new choice: what was running resumes under it.
    close_core(&app).await;
    Ok(status(app).await)
}

/// Asks the provider whether it takes `key`. Only a plain "no" stops the user: a provider
/// that can't be reached or answers something else may still take the key when it matters.
async fn check_key(provider: Vendor, key: &str) -> Said<()> {
    let name = named(provider).0;
    let http = reqwest::Client::new();
    let request = match provider {
        Vendor::Anthropic => http
            .get("https://api.anthropic.com/v1/models?limit=1")
            .header("x-api-key", key)
            .header("anthropic-version", "2023-06-01"),
        Vendor::Fireworks => http
            .get("https://api.fireworks.ai/inference/v1/models")
            .bearer_auth(key),
        Vendor::Baseten => http
            .get("https://inference.baseten.co/v1/models")
            .header("Authorization", format!("Api-Key {key}")),
    };
    match request
        .send()
        .await
        .map(|response| response.status().as_u16())
    {
        Ok(401 | 403) => Err(format!(
            "{name} didn't accept that key. Check it and paste it again."
        )),
        Ok(_) => Ok(()),
        Err(_) => Err(format!(
            "Couldn't reach {name} to check the key. Check your connection."
        )),
    }
}

/// Asks TypeSafe whether it takes `key`, as `check_key` asks a provider.
async fn check_jev_key(key: &str) -> Said<()> {
    let response = reqwest::Client::new()
        .get(format!("{}/v1/models", jev::BASE_URL))
        .bearer_auth(key)
        .send()
        .await;
    match response.map(|response| response.status().as_u16()) {
        Ok(401 | 403) => {
            Err("TypeSafe didn't accept that key. Check it and paste it again.".into())
        }
        Ok(_) => Ok(()),
        Err(_) => Err("Couldn't reach TypeSafe to check the key. Check your connection.".into()),
    }
}

/// Sends what the user wrote: to `thread`, or wherever the router decides. `steer` joins the
/// answer under way instead of waiting for it.
#[tauri::command]
async fn send(
    app: AppHandle,
    text: String,
    thread: Option<String>,
    steer: bool,
    request_id: Option<String>,
) -> Said<Sent> {
    let mut input = Input::text(Speaker::owner(), text);
    input.thread = thread;
    input.request_id = request_id;
    if steer {
        input.when_busy = WhenBusy::Steer;
    }
    core(&app)?.send(input).await.map_err(say)
}

#[tauri::command]
fn threads(app: AppHandle) -> Said<Vec<ThreadSummary>> {
    core(&app)?.threads().map_err(say)
}

#[tauri::command]
fn history(app: AppHandle, thread: String, since: usize) -> Said<Vec<Event>> {
    core(&app)?.history(&thread, since).map_err(say)
}

/// The conversation: every thread's last `turns` turns that started before `before`, as one.
#[tauri::command]
fn timeline(app: AppHandle, before: Option<u64>, turns: usize) -> Said<Vec<Event>> {
    core(&app)?.timeline(before, turns).map_err(say)
}

#[tauri::command]
fn abort(app: AppHandle, thread: String) -> Said<bool> {
    core(&app)?.abort(&thread).map_err(say)
}

#[tauri::command]
fn feedback(app: AppHandle, thread: String, turn: Option<u32>, text: String) -> Said<()> {
    let feedback = Feedback {
        from: Speaker::owner(),
        thread,
        turn,
        text,
    };
    core(&app)?.feedback(feedback).map_err(say)
}

/// What it remembers, newest first.
#[tauri::command]
fn memories(app: AppHandle) -> Said<Vec<Memory>> {
    core(&app)?.memories(&Speaker::owner()).map_err(say)
}

/// Keeps something the user tells it outright.
#[tauri::command]
fn remember(app: AppHandle, text: String) -> Said<Memory> {
    core(&app)?.remember(&Speaker::owner(), &text).map_err(say)
}

/// The user's correction of the memory `id`.
#[tauri::command]
fn revise_memory(app: AppHandle, id: i64, text: String) -> Said<Memory> {
    core(&app)?
        .revise(&Speaker::owner(), id, &text)
        .map_err(say)
}

/// Removes the memory `id`, for good.
#[tauri::command]
fn forget_memory(app: AppHandle, id: i64) -> Said<()> {
    core(&app)?.forget(&Speaker::owner(), id).map_err(say)
}

/// Forgets everything it has learned, for good. The conversations stay.
#[tauri::command]
fn wipe_memory(app: AppHandle) -> Said<()> {
    core(&app)?.wipe(&Speaker::owner()).map_err(say)
}

/// The user's notes that match `search`.
#[tauri::command]
fn notes(app: AppHandle, search: NoteSearch) -> Said<Vec<Note>> {
    core(&app)?.notes(&Speaker::owner(), search).map_err(say)
}

/// A new note, or (with `id`) a change to one.
#[tauri::command]
fn write_note(app: AppHandle, id: Option<i64>, draft: NoteDraft) -> Said<Note> {
    core(&app)?
        .write_note(&Speaker::owner(), id, draft)
        .map_err(say)
}

/// Deletes the note `id`, for good.
#[tauri::command]
fn delete_note(app: AppHandle, id: i64) -> Said<()> {
    core(&app)?.delete_note(&Speaker::owner(), id).map_err(say)
}

/// Where the voice models stand; `null` when there is no voice.
#[tauri::command]
fn voice_status(app: AppHandle) -> Said<Option<VoiceStatus>> {
    Ok(core(&app)?.voice())
}

/// Downloads the voice models (once); progress comes as `voice` events.
#[tauri::command]
async fn prepare_voice(app: AppHandle) -> Said<()> {
    let core = core(&app)?;
    core.prepare_voice(&Speaker::owner()).await.map_err(say)
}

/// Loads the voice models, if they aren't yet, while the user talks.
#[tauri::command]
fn warm_voice(app: AppHandle) -> Said<()> {
    core(&app)?.warm_voice();
    Ok(())
}

/// What was said: the request's body is a WAV recording.
#[tauri::command]
async fn listen(app: AppHandle, request: tauri::ipc::Request<'_>) -> Said<Heard> {
    let tauri::ipc::InvokeBody::Raw(wav) = request.body() else {
        return Err("send the recording as bytes".into());
    };
    let core = core(&app)?;
    core.listen(&Speaker::owner(), wav.clone())
        .await
        .map_err(say)
}

/// A reply read out in `language`, as WAV bytes.
#[tauri::command]
async fn speak(app: AppHandle, text: String, language: String) -> Said<tauri::ipc::Response> {
    let core = core(&app)?;
    let wav = (core.speak(&Speaker::owner(), &text, &language).await).map_err(say)?;
    Ok(tauri::ipc::Response::new(wav))
}

/// Every worker there has been, latest first.
#[tauri::command]
fn workers(app: AppHandle) -> Said<Vec<Worker>> {
    core(&app)?.workers(&Speaker::owner()).map_err(say)
}

/// Stops the worker `id`. Returns whether it was running.
#[tauri::command]
fn cancel_worker(app: AppHandle, id: String) -> Said<bool> {
    core(&app)?
        .cancel_worker(&Speaker::owner(), &id)
        .map_err(say)
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

const CHECK_UPDATES: &str = "check-updates";

/// The usual Mac menus (without Edit's items copy and paste don't work in a webview), with
/// Check for Updates under About, where Mac apps keep it.
fn menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let menu = Menu::default(app)?;
    if let Some(MenuItemKind::Submenu(tokidoki)) = menu.items()?.first() {
        let check =
            MenuItem::with_id(app, CHECK_UPDATES, "Check for Updates…", true, None::<&str>)?;
        tokidoki.insert(&check, 1)?;
    }
    Ok(menu)
}

pub fn run() {
    // Links (a reply's, a report's sources) open in the browser, not in this window.
    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());
    // One core per data folder: a second launch brings the first forward instead. Not in
    // development, whose data is its own, so `just app` runs beside the installed app.
    #[cfg(not(debug_assertions))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        show_window(app)
    }));
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Core::default())
        .manage(updater::Updates::default())
        .setup(|app| {
            app.set_menu(menu(app.handle())?)?;
            updater::start(app.handle());
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id() == CHECK_UPDATES {
                // What it finds is said in the window, so the window comes forward.
                show_window(app);
                let app = app.clone();
                tauri::async_runtime::spawn(async move { updater::check_now(&app).await });
            }
        })
        .on_window_event(|window, event| match event {
            // Closing the window leaves Tokidoki running; the Dock icon brings it back.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(true) => updater::on_focus(window.app_handle()),
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            status,
            chosen,
            choose,
            send,
            threads,
            history,
            timeline,
            abort,
            feedback,
            memories,
            remember,
            revise_memory,
            forget_memory,
            wipe_memory,
            notes,
            write_note,
            delete_note,
            workers,
            cancel_worker,
            voice_status,
            prepare_voice,
            warm_voice,
            listen,
            speak,
            passport::passport,
            passport::save_account,
            passport::delete_account,
            passport::sign_in_account,
            debug::debug_browser,
            updater::update_status,
            updater::check_for_updates,
            updater::install_update,
            updater::app_version,
            debug::debug_ready,
            debug::debug_log,
            debug::debug_screenshot,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Tokidoki")
        .run(|app, event| match event {
            RunEvent::Reopen { .. } => show_window(app),
            RunEvent::Exit => {
                tauri::async_runtime::block_on(close_core(app));
                updater::install_on_exit(app);
            }
            _ => {}
        });
}
