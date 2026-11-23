//! Looking at the app without a person: debug builds only.
//!
//! `TOKIDOKI_TEST=path/to/script.js` runs the script inside the window's page once it has
//! mounted, with `t` = `window.__tokidoki` (app/src/lib/debug.ts): it clicks and types as a
//! user would, and saves pictures of the window to `$TOKIDOKI_SHOTS`. `e2e/run.sh` runs one.

use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use tauri::{AppHandle, Manager};

fn testing() -> Option<String> {
    cfg!(debug_assertions)
        .then(|| std::env::var("TOKIDOKI_TEST").ok())
        .flatten()
}

/// Called by the page on mount; runs the test script if one was given.
#[tauri::command]
pub async fn debug_ready(app: AppHandle) -> Result<(), String> {
    let Some(path) = testing() else {
        return Ok(());
    };
    let script = std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))?;
    let page = app.get_webview_window("main").ok_or("no window")?;
    page.eval(format!(
        "(async () => {{ const t = window.__tokidoki; try {{ {script}\n }} catch (e) {{ await t.log('FAIL ' + (e?.message ?? e) + '\\n' + (e?.stack ?? '')); }} finally {{ await t.log('TEST DONE'); }} }})()"
    ))
    .map_err(|e| e.to_string())
}

/// Prints `[test] …` for the runner to read.
#[tauri::command]
pub fn debug_log(message: String) {
    if testing().is_some() {
        println!("[test] {message}");
    }
}

/// Saves a picture of the page as `$TOKIDOKI_SHOTS/<name>.png`, and returns its path. The
/// web view takes it of itself, so it needs no permission to record the screen and works
/// with the window behind others.
#[tauri::command]
pub async fn debug_screenshot(app: AppHandle, name: String) -> Result<String, String> {
    if testing().is_none() {
        return Err("not testing".into());
    }
    let dir = std::env::var("TOKIDOKI_SHOTS")
        .unwrap_or_else(|_| std::env::temp_dir().display().to_string());
    let path = format!("{dir}/{name}.png");
    let (done, taken) = tokio::sync::oneshot::channel::<bool>();
    let window = app.get_webview_window("main").ok_or("no window")?;
    let to = path.clone();
    window
        .with_webview(move |platform| snapshot(platform.inner() as *const AnyObject, to, done))
        .map_err(|e| e.to_string())?;
    match taken.await {
        Ok(true) => Ok(path),
        _ => Err(format!("no picture could be written to {path}")),
    }
}

/// Asks the WKWebView for a picture of what it shows and writes it to `path` as a PNG.
fn snapshot(view: *const AnyObject, path: String, done: tokio::sync::oneshot::Sender<bool>) {
    use std::cell::Cell;
    let done = Cell::new(Some(done));
    let write = move |image: *mut AnyObject, _error: *mut AnyObject| {
        // SAFETY: WebKit hands the handler an NSImage (or nil); the messages below are
        // AppKit's and Foundation's own, with the argument types they declare.
        let written = !image.is_null()
            && unsafe {
                let tiff: *mut AnyObject = msg_send![image, TIFFRepresentation];
                let bitmap: *mut AnyObject =
                    msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
                let none: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
                // 4 is NSBitmapImageFileTypePNG.
                let png: *mut AnyObject =
                    msg_send![bitmap, representationUsingType: 4usize, properties: none];
                let path = NSString::from_str(&path);
                !png.is_null() && msg_send![png, writeToFile: &*path, atomically: true]
            };
        if let Some(done) = done.take() {
            let _ = done.send(written);
        }
    };
    let handler = block2::RcBlock::new(write);
    let whole_view: *const AnyObject = std::ptr::null();
    // SAFETY: `view` is the live WKWebView Tauri runs this closure with, on the main thread.
    unsafe {
        let _: () = msg_send![
            view,
            takeSnapshotWithConfiguration: whole_view,
            completionHandler: &*handler
        ];
    }
}
