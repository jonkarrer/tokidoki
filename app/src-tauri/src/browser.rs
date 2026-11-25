//! The browser a mission drives: a window of the app's own, with its own saved logins, apart
//! from Safari's and from the app's page. It is `browser::Engine`, which only has to go to an
//! address and run a script in the page; what the scripts do is in the `browser` crate.
//!
//! The window is made the first time a mission browses, and shown without taking the
//! keyboard: the user can watch, and can click and type in it themselves (signing in to a
//! site is done here, once; the session is kept). Closing it hides it, and it is put away
//! when the last mission out comes back (`put_away`). A page in it can't
//! call the app: the app's commands are granted to the `main` window alone
//! (capabilities/default.json).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result};
use browser::{Engine, EngineFuture};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const WINDOW: &str = "browser";

/// Which saved logins the window uses: a store of its own, named by these bytes (any 16,
/// kept the same for good). Development builds have another, so trying things out never
/// signs the installed app out.
const STORE: [u8; 16] = *b"tokidoki-browser";
const DEV_STORE: [u8; 16] = *b"tokidoki-dev-web";

/// What it says it is: Safari, which it is underneath. A bare web view leaves the browser's
/// name out, and sites that look for one send it a page for unknown browsers, or none.
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
                          (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

/// The window was put away because no mission was out (`put_away`).
static AWAY: AtomicBool = AtomicBool::new(false);

/// Hides the window, if there is one, until a mission next browses.
pub fn put_away(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        AWAY.store(true, Ordering::Release);
        let _ = window.hide();
    }
}

pub struct Page {
    app: AppHandle,
    /// Held while the window is made, so two first steps make one.
    making: Mutex<()>,
}

impl Page {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            making: Mutex::default(),
        }
    }

    fn window(&self) -> Result<WebviewWindow> {
        let _making = self.making.lock().unwrap();
        if let Some(window) = self.app.get_webview_window(WINDOW) {
            // Put away when the last mission ended: back for this one. Not when the user
            // closed it themselves meanwhile: then it stays closed until they ask for it.
            if AWAY.swap(false, Ordering::AcqRel) {
                let _ = window.show();
            }
            return Ok(window);
        }
        let blank = WebviewUrl::External("about:blank".parse()?);
        let store = match cfg!(debug_assertions) {
            true => DEV_STORE,
            false => STORE,
        };
        WebviewWindowBuilder::new(&self.app, WINDOW, blank)
            .title("Tokidoki’s browser")
            .inner_size(1280.0, 900.0)
            .data_store_identifier(store)
            .user_agent(USER_AGENT)
            .focused(false)
            .build()
            .context("the browser window couldn't be opened")
    }
}

impl Engine for Page {
    fn navigate<'a>(&'a self, url: &'a str) -> EngineFuture<'a, ()> {
        Box::pin(async move {
            let url = url
                .parse()
                .with_context(|| format!("{url} isn't an address"))?;
            Ok(self.window()?.navigate(url)?)
        })
    }

    fn eval<'a>(&'a self, script: &'a str) -> EngineFuture<'a, String> {
        Box::pin(async move {
            let (answer, answered) = tokio::sync::oneshot::channel();
            // The callback may be called more than once by its type; it answers once.
            let answer = Mutex::new(Some(answer));
            self.window()?.eval_with_callback(script, move |said| {
                if let Some(answer) = answer.lock().unwrap().take() {
                    let _ = answer.send(said);
                }
            })?;
            answered
                .await
                .context("the page went away before answering")
        })
    }
}

/// Brings the browser window forward, for the user to look or sign in.
#[tauri::command]
pub fn show_browser(app: AppHandle) -> Result<(), String> {
    let window = Page::new(app).window().map_err(|e| format!("{e:#}"))?;
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())
}
