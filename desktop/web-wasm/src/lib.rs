//! WASI reactor entry point for Tiffin on the web.
//!
//! Runs inside a dedicated browser Web Worker backed by the OPFS sync bridge
//! (`desktop/src/web/wasi-opfs.ts`). Call `init("/data")` once after WASI initialization, then
//! invoke commands via `call` (JSON) or `call_bytes` (raw binary payloads for photos) and read the
//! reply bytes from `reply_ptr()` / `reply_len()` / `reply_is_err()`.

use recipe_box::foods::off;
use recipe_box::model;
use recipe_box::tracker::journal;
use recipe_box::{foods, labels, storage};
use serde::Serialize;
use serde_json::{json, Value};
use std::alloc::{self, Layout};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};
use tiffin_commands::foods::{self as foods_cmd, OffState, OnlineResult};
use tiffin_commands::{importing, recipes, settings, Services};

static APP: OnceLock<Services> = OnceLock::new();

struct Reply {
    bytes: Vec<u8>,
    is_err: bool,
}

static REPLY: Mutex<Reply> = Mutex::new(Reply { bytes: Vec::new(), is_err: false });

/// Photo file names are made by storage::photo_name: letters, digits, '-', '_' and ".jpg"/".png".
pub fn safe_photo_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains("..") && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
}

#[derive(Serialize)]
struct BackupExport {
    filename: String,
    text: String,
    message: String,
}

/// Write a full backup (library, photos, diary and profile) into the store's Backups folder and
/// return its filename, JSON text (for browser download), and status message.
fn backup_export_in(state: &tiffin_commands::Shared) -> Result<BackupExport, String> {
    let (lib, photos, jdir, into) = {
        let s = state.lock().unwrap();
        let (lib, photos, jdir) = settings::backup_parts(&s)?;
        (lib, photos, jdir, storage::backups_dir(&s.store.dir))
    };
    let (bundle, left_out) = journal::bundle_from_disk(&jdir);
    let p = storage::backup_with_photos(&lib, &into, Some(&photos), bundle.as_ref()).map_err(|e| format!("Backup failed: {e}"))?;
    let text = std::fs::read_to_string(&p).map_err(|e| format!("Backup failed: {e}"))?;
    let filename = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("Tiffin-backup.{}", storage::BACKUP_EXT));
    let mut message = format!("Backed up to {filename}");
    if !left_out.is_empty() {
        message.push_str(&format!(" These diary months couldn't be read and were left out: {}.", left_out.join(", ")));
    }
    Ok(BackupExport { filename, text, message })
}

#[derive(Serialize)]
struct Guess {
    food: model::MyFood,
    source: String,
    missing: Vec<&'static str>,
    check: Vec<&'static str>,
}

impl From<labels::LabelGuess> for Guess {
    fn from(g: labels::LabelGuess) -> Self {
        Guess { food: g.food, source: g.source, missing: g.missing, check: g.check }
    }
}

/// Dispatch one JSON command by name: the 106 shared commands in `tiffin_commands::dispatch`, the
/// offline cores (`settings_info`, `rules_overview`, `delete_health_data`, `self_tests`,
/// `save_review`/`web_save_review`, `backup_now`/`web_backup_export`), the web network bridge
/// step commands (`web_prepare_import_url`, `web_finish_import_url`, `web_off_prepare`,
/// `web_off_finish`, `web_off_finish_product`, `web_label_prepare`, `web_label_finish_off`,
/// `web_label_finish_html`), `warm_foods`, and desktop-only startup no-ops so the Svelte UI runs
/// unchanged on the web.
pub fn handle_json(app: &Services, name: &str, args: Value) -> Result<Value, String> {
    if tiffin_commands::NAMES.contains(&name) {
        return tiffin_commands::dispatch(app, name, args);
    }
    let args = if args.is_null() { json!({}) } else { args };
    let bad = |e: serde_json::Error| format!("Bad arguments for {name}: {e}");
    let reply = |v: Result<Value, serde_json::Error>| v.map_err(|e| e.to_string());
    match name {
        "settings_info" => reply(serde_json::to_value(settings::settings_info_in(&app.state, settings::DesktopInfo::default()))),
        "rules_overview" => {
            #[derive(serde::Deserialize)]
            struct Args {
                test: bool,
            }
            let Args { test } = serde_json::from_value(args).map_err(bad)?;
            reply(serde_json::to_value(importing::rules_overview_in(&app.state, test, String::new())))
        }
        "delete_health_data" => reply(serde_json::to_value(settings::delete_health_data_in(&app.state)?)),
        "self_tests" => reply(serde_json::to_value(settings::self_tests_in(&app.state))),
        "save_review" => {
            #[derive(serde::Deserialize)]
            struct Args {
                review: importing::Review,
            }
            let Args { review } = serde_json::from_value(args).map_err(bad)?;
            let (saved, _photo) = importing::save_review_in(&app.state, review)?;
            reply(serde_json::to_value(saved))
        }
        "web_save_review" => {
            #[derive(serde::Deserialize)]
            struct Args {
                review: importing::Review,
            }
            let Args { review } = serde_json::from_value(args).map_err(bad)?;
            let (saved, photo) = importing::save_review_in(&app.state, review)?;
            Ok(json!({
                "saved": saved,
                "photo": photo.map(|p| json!({ "url": p.url, "file": p.file })),
            }))
        }
        "web_prepare_import_url" => {
            #[derive(serde::Deserialize)]
            struct Args {
                url: String,
            }
            let Args { url } = serde_json::from_value(args).map_err(bad)?;
            reply(serde_json::to_value(importing::web_url(&url)?))
        }
        "web_finish_import_url" => {
            #[derive(serde::Deserialize)]
            struct Args {
                html: String,
                url: String,
            }
            let Args { html, url } = serde_json::from_value(args).map_err(bad)?;
            reply(serde_json::to_value(importing::from_html(&app.state, &html, &url, "Web page")))
        }
        "web_off_prepare" => {
            #[derive(serde::Deserialize)]
            struct Args {
                q: String,
            }
            let Args { q } = serde_json::from_value(args).map_err(bad)?;
            if !off::query_ready(&q) {
                return Err("Type at least two letters to search online.".into());
            }
            let barcode = match off::barcode_in(&q) {
                Some(Err(_)) => {
                    let result = OnlineResult { status: "bad_barcode", retry_in: 0, added: 0, rows: foods_cmd::rows(&app.state, &app.off, &q, false) };
                    return Ok(json!({ "kind": "done", "result": result }));
                }
                Some(Ok(code)) => Some(code),
                None => None,
            };
            let email = app.state.lock().unwrap().lib.settings.off_email.clone();
            let Some(ua) = off::user_agent(&email) else {
                let result = foods_cmd::finish(&app.state, &app.off, &q, off::OffAnswer::NeedsEmail, model::now_secs());
                return Ok(json!({ "kind": "done", "result": result }));
            };
            if let Some(wait) = foods_cmd::begin(&app.off, model::now_secs()) {
                let result = OnlineResult { status: "wait", retry_in: wait, added: 0, rows: foods_cmd::rows(&app.state, &app.off, &q, true) };
                return Ok(json!({ "kind": "done", "result": result }));
            }
            match barcode {
                Some(code) => Ok(json!({
                    "kind": "product",
                    "code": code,
                    "url": format!("{}/{code}.json?fields={}", off::OFF_PRODUCT_URL, off::OFF_PRODUCT_FIELDS),
                    "user_agent": ua,
                })),
                None => Ok(json!({
                    "kind": "search",
                    "text": q.trim(),
                    "email": email.trim(),
                })),
            }
        }
        "web_off_finish" => {
            #[derive(serde::Deserialize)]
            struct Args {
                q: String,
                barcode: bool,
                answer: Value,
            }
            let Args { q, barcode, answer } = serde_json::from_value(args).map_err(bad)?;
            let answer = serde_json::from_value::<off::OffAnswer>(answer).unwrap_or(off::OffAnswer::Slow);
            let res = foods_cmd::named(foods_cmd::finish(&app.state, &app.off, &q, answer, model::now_secs()), barcode);
            reply(serde_json::to_value(res))
        }
        "web_off_finish_product" => {
            #[derive(serde::Deserialize)]
            struct Args {
                q: String,
                code: String,
                body: String,
            }
            let Args { q, code, body } = serde_json::from_value(args).map_err(bad)?;
            let answer = serde_json::from_str::<Value>(&body).map(|v| off::parse_product(&code, v)).unwrap_or(off::OffAnswer::Slow);
            let res = foods_cmd::named(foods_cmd::finish(&app.state, &app.off, &q, answer, model::now_secs()), true);
            reply(serde_json::to_value(res))
        }
        "web_label_prepare" => {
            #[derive(serde::Deserialize)]
            struct Args {
                input: String,
            }
            let Args { input } = serde_json::from_value(args).map_err(bad)?;
            match labels::classify_lookup(&input)? {
                labels::LookupTarget::Barcode(code) => {
                    let email = app.state.lock().unwrap().lib.settings.off_email.clone();
                    let ua = off::user_agent(&email).ok_or_else(|| off::NEEDS_EMAIL.to_string())?;
                    Ok(json!({
                        "kind": "barcode",
                        "code": code,
                        "url": format!("{}/{code}.json?fields={}", off::OFF_PRODUCT_URL, labels::OFF_LABEL_FIELDS),
                        "user_agent": ua,
                    }))
                }
                labels::LookupTarget::Url(url) => Ok(json!({ "kind": "url", "url": url })),
            }
        }
        "web_label_finish_off" => {
            #[derive(serde::Deserialize)]
            struct Args {
                code: String,
                body: String,
            }
            let Args { code, body } = serde_json::from_value(args).map_err(bad)?;
            let guess = Guess::from(labels::parse_off_product_response(&body, &code)?);
            reply(serde_json::to_value(guess))
        }
        "web_label_finish_html" => {
            #[derive(serde::Deserialize)]
            struct Args {
                html: String,
                url: String,
            }
            let Args { html, url } = serde_json::from_value(args).map_err(bad)?;
            let guess = Guess::from(labels::from_html(&html, &url).ok_or_else(|| labels::NO_NUTRITION_ON_PAGE.to_string())?);
            reply(serde_json::to_value(guess))
        }
        "web_backup_export" => reply(serde_json::to_value(backup_export_in(&app.state)?)),
        "backup_now" => reply(serde_json::to_value(backup_export_in(&app.state)?.message)),
        "warm_foods" => {
            foods::warm();
            Ok(Value::Null)
        }
        // Desktop-only startup/shell commands: clean no-ops on the web.
        "mica" => Ok(json!(false)),
        "show_window" | "set_unsaved" | "auto_sync" | "check_update" => Ok(Value::Null),
        "launch_notes" => Ok(json!([])),
        "take_inbox" => Ok(json!({ "reviews": [], "notes": [] })),
        _ => Err(format!("Unknown command {name}")),
    }
}

/// Dispatch one binary command: `add_photo` (raw image bytes -> UTF-8 filename), `write_photo`
/// (raw image bytes -> saves under a reserved filename), `read_photo` (filename -> raw bytes),
/// `barcode_photo` (raw image bytes -> UTF-8 barcode digits or empty when none found), or
/// `label_photo` (raw image bytes -> JSON `Guess`, or non-native manual-entry message).
pub fn handle_bytes(app: &Services, name: &str, arg: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    match name {
        "add_photo" => {
            let stem = if arg.is_empty() { "photo" } else { arg };
            recipes::add_photo_bytes(&app.state, bytes, stem).map(String::into_bytes)
        }
        "write_photo" => {
            if !safe_photo_name(arg) {
                return Err("Invalid photo name.".into());
            }
            let dir = {
                let s = app.state.lock().unwrap();
                s.store.check_unlocked()?;
                storage::photos_dir(&s.store.dir)
            };
            storage::write_photo(bytes, &dir, arg)?;
            Ok(Vec::new())
        }
        "read_photo" => {
            if !safe_photo_name(arg) {
                return Err("Photo not found.".into());
            }
            let path = {
                let s = app.state.lock().unwrap();
                storage::photos_dir(&s.store.dir).join(arg)
            };
            std::fs::read(&path).map_err(|_| "Photo not found.".to_string())
        }
        "barcode_photo" => Ok(labels::barcode_from_bytes(bytes)?.map(String::into_bytes).unwrap_or_default()),
        "label_photo" => {
            let clean: String = arg.chars().filter(|c| c.is_alphanumeric() || " .-_".contains(*c)).take(60).collect();
            let guess = Guess::from(labels::from_photo_bytes(bytes, if clean.is_empty() { "label" } else { &clean })?);
            serde_json::to_vec(&guess).map_err(|e| e.to_string())
        }
        _ => Err(format!("Unknown binary command {name}")),
    }
}

fn set_reply(res: Result<Vec<u8>, String>) -> usize {
    let mut r = REPLY.lock().unwrap_or_else(|e| e.into_inner());
    match res {
        Ok(bytes) => {
            r.bytes = bytes;
            r.is_err = false;
        }
        Err(msg) => {
            r.bytes = msg.into_bytes();
            r.is_err = true;
        }
    }
    r.bytes.len()
}

unsafe fn read_slice<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], String> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err("Null pointer passed to WebAssembly.".into());
    }
    if len > isize::MAX as usize {
        return Err("Buffer length exceeds maximum slice size.".into());
    }
    // SAFETY: caller allocated `len` (<= isize::MAX) readable bytes at non-null `ptr` for this call.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

unsafe fn read_str<'a>(ptr: *const u8, len: usize) -> Result<&'a str, String> {
    let bytes = unsafe { read_slice(ptr, len)? };
    std::str::from_utf8(bytes).map_err(|e| format!("Invalid UTF-8: {e}"))
}

#[no_mangle]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return NonNull::dangling().as_ptr();
    }
    let Ok(layout) = Layout::from_size_align(len, 1) else {
        return std::ptr::null_mut();
    };
    // SAFETY: layout has non-zero size and alignment 1.
    unsafe { alloc::alloc(layout) }
}

/// # Safety
/// `ptr` must have been returned by [`alloc`] with the same `len` (or `len == 0` / null).
#[no_mangle]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    if len == 0 || ptr.is_null() {
        return;
    }
    if let Ok(layout) = Layout::from_size_align(len, 1) {
        // SAFETY: caller guarantees `ptr` came from `alloc(len)` with alignment 1.
        unsafe { alloc::dealloc(ptr, layout) }
    }
}

/// Open the library and diary under `dir` (for example `"/data"`). Returns 0 on success or 1 with
/// the error message stored in the reply buffer.
///
/// # Safety
/// `dir_ptr` must be valid for `dir_len` readable bytes (or `dir_len == 0`).
#[no_mangle]
pub unsafe extern "C" fn init(dir_ptr: *const u8, dir_len: usize) -> i32 {
    let res = (|| -> Result<Vec<u8>, String> {
        let dir = unsafe { read_str(dir_ptr, dir_len)? };
        if dir.is_empty() {
            return Err("Data directory is empty.".into());
        }
        // Check before load_state so a duplicate init never writes to disk or overwrites global MY_FOODS.
        if APP.get().is_some() {
            return Err("Already initialized.".into());
        }
        let path = PathBuf::from(dir);
        let services = Services { state: Mutex::new(tiffin_commands::load_state(path.clone())), off: OffState::new(&path) };
        APP.set(services).map_err(|_| "Already initialized.".to_string())?;
        Ok(Vec::new())
    })();
    let err = res.is_err();
    set_reply(res);
    if err { 1 } else { 0 }
}

/// Run one JSON command. Stores the reply JSON (or UTF-8 error text when `reply_is_err() == 1`) in
/// the reply buffer and returns its byte length.
///
/// # Safety
/// `name_ptr`/`name_len` and `args_ptr`/`args_len` must be valid readable byte slices (or len 0).
#[no_mangle]
pub unsafe extern "C" fn call(name_ptr: *const u8, name_len: usize, args_ptr: *const u8, args_len: usize) -> usize {
    let res = (|| -> Result<Vec<u8>, String> {
        let app = APP.get().ok_or_else(|| "Tiffin isn't initialized yet.".to_string())?;
        let name = unsafe { read_str(name_ptr, name_len)? };
        let args_str = unsafe { read_str(args_ptr, args_len)? };
        let args: Value = if args_str.is_empty() { Value::Null } else { serde_json::from_str(args_str).map_err(|e| format!("Bad JSON for {name}: {e}"))? };
        let out = handle_json(app, name, args)?;
        serde_json::to_vec(&out).map_err(|e| e.to_string())
    })();
    set_reply(res)
}

/// Run one binary command (`add_photo`, `write_photo`, `read_photo`, `barcode_photo`, `label_photo`).
/// Stores the reply bytes (or UTF-8 error text when `reply_is_err() == 1`) in the reply buffer and
/// returns its byte length.
///
/// # Safety
/// `name_ptr`/`name_len`, `arg_ptr`/`arg_len`, and `bin_ptr`/`bin_len` must be valid readable byte
/// slices (or len 0).
#[no_mangle]
pub unsafe extern "C" fn call_bytes(name_ptr: *const u8, name_len: usize, arg_ptr: *const u8, arg_len: usize, bin_ptr: *const u8, bin_len: usize) -> usize {
    let res = (|| -> Result<Vec<u8>, String> {
        let app = APP.get().ok_or_else(|| "Tiffin isn't initialized yet.".to_string())?;
        let name = unsafe { read_str(name_ptr, name_len)? };
        let arg = unsafe { read_str(arg_ptr, arg_len)? };
        let bin = unsafe { read_slice(bin_ptr, bin_len)? };
        handle_bytes(app, name, arg, bin)
    })();
    set_reply(res)
}

#[no_mangle]
pub extern "C" fn reply_ptr() -> *const u8 {
    REPLY.lock().unwrap_or_else(|e| e.into_inner()).bytes.as_ptr()
}

#[no_mangle]
pub extern "C" fn reply_len() -> usize {
    REPLY.lock().unwrap_or_else(|e| e.into_inner()).bytes.len()
}

#[no_mangle]
pub extern "C" fn reply_is_err() -> i32 {
    if REPLY.lock().unwrap_or_else(|e| e.into_inner()).is_err { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        #[cfg(target_os = "wasi")]
        return PathBuf::from("/tmp");
        #[cfg(not(target_os = "wasi"))]
        std::env::temp_dir()
    }

    fn test_pid() -> u32 {
        #[cfg(target_os = "wasi")]
        return 0;
        #[cfg(not(target_os = "wasi"))]
        std::process::id()
    }

    fn fresh_app(name: &str) -> (PathBuf, Services) {
        let dir = scratch_dir().join(format!("tiffin-web-wasm-{name}-{}", test_pid()));
        let _ = std::fs::remove_dir_all(&dir);
        let app = Services { state: Mutex::new(tiffin_commands::load_state(dir.clone())), off: OffState::new(&dir) };
        (dir, app)
    }

    /// Valid 1×1 24-bit BMP (58 bytes).
    const ONE_PX_BMP: [u8; 58] = [
        b'B', b'M', 58, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0,
        40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 24, 0,
        0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0x20, 0x80, 0xe0, 0x00,
    ];

    #[test]
    fn json_dispatch_covers_shared_offline_and_noop_commands() {
        let (dir, app) = fresh_app("json");

        // Shared crate command
        let ov = handle_json(&app, "overview", json!({ "today": "2026-11-24" })).unwrap();
        assert!(ov["counts"][0].as_u64().unwrap() > 0);

        // Offline cores
        let info = handle_json(&app, "settings_info", Value::Null).unwrap();
        assert_eq!(info["inbox_ready"], false);
        assert_eq!(info["onedrive"], Value::Null);
        assert!(info["recipes"].as_u64().unwrap() > 0);

        let rules = handle_json(&app, "rules_overview", json!({ "test": false })).unwrap();
        assert_eq!(rules["inbox"], "");
        assert!(!rules["rules"].as_array().unwrap().is_empty());

        let checks = handle_json(&app, "self_tests", Value::Null).unwrap();
        let checks_arr = checks.as_array().unwrap();
        assert!(!checks_arr.is_empty());
        assert!(checks_arr.iter().all(|c| c["passed"] == true));

        // web_save_review (with and without image_url) and save_review (returns Saved directly)
        let mut rv = handle_json(&app, "import_text", json!({ "text": "Tea\n\nIngredients\n1 cup water\n\nDirections\n1. Boil." })).unwrap();
        rv["image_url"] = json!("https://example.com/tea.jpg");
        let res = handle_json(&app, "web_save_review", json!({ "review": rv })).unwrap();
        assert_eq!(res["saved"]["message"], "Saved to your library");
        assert_eq!(res["photo"]["url"], "https://example.com/tea.jpg");
        assert!(res["photo"]["file"].as_str().unwrap().ends_with(".jpg"));

        let rv_no_img = handle_json(&app, "import_text", json!({ "text": "Toast\n\nIngredients\n1 slice bread\n\nDirections\n1. Toast." })).unwrap();
        let res_no_img = handle_json(&app, "web_save_review", json!({ "review": rv_no_img })).unwrap();
        assert_eq!(res_no_img["saved"]["message"], "Saved to your library");
        assert_eq!(res_no_img["photo"], Value::Null);

        let rv2 = handle_json(&app, "import_text", json!({ "text": "Cocoa\n\nIngredients\n1 cup milk\n\nDirections\n1. Warm." })).unwrap();
        let saved2 = handle_json(&app, "save_review", json!({ "review": rv2 })).unwrap();
        assert_eq!(saved2["message"], "Saved to your library");
        assert!(saved2["id"].as_u64().is_some());

        // web_backup_export -> valid backup text that restores cleanly; backup_now -> message
        handle_json(&app, "quick_add", json!({ "day": "2026-11-24", "today": "2026-11-24", "slot": "Breakfast", "label": "Oats", "calories": 200.0 })).unwrap();
        let exp = handle_json(&app, "web_backup_export", Value::Null).unwrap();
        let filename = exp["filename"].as_str().unwrap();
        let text = exp["text"].as_str().unwrap();
        assert!(filename.ends_with(".recipebox"), "{filename}");
        assert!(exp["message"].as_str().unwrap().starts_with("Backed up to "));
        let parsed = storage::parse_backup(text).unwrap();
        assert!(parsed.journal.is_some());

        let msg = handle_json(&app, "backup_now", Value::Null).unwrap();
        assert!(msg.as_str().unwrap().starts_with("Backed up to "));

        // delete_health_data wipes journal and strips backups
        let del = handle_json(&app, "delete_health_data", Value::Null).unwrap();
        assert!(!del.as_str().unwrap().is_empty());
        let day_after = handle_json(&app, "diary_day", json!({ "day": "2026-11-24", "today": "2026-11-24" })).unwrap();
        assert!(day_after["slots"][0]["entries"].as_array().unwrap().is_empty());

        // restore_backup from the exported backup text brings the diary back
        let restored = handle_json(&app, "restore_backup", json!({ "path": null, "text": text, "scope": "everything" })).unwrap();
        assert!(restored.as_str().unwrap().contains("food diary"), "{restored}");
        let day_back = handle_json(&app, "diary_day", json!({ "day": "2026-11-24", "today": "2026-11-24" })).unwrap();
        assert_eq!(day_back["slots"][0]["entries"][0]["name"], "Oats");

        // Desktop-only no-ops and warm_foods
        assert_eq!(handle_json(&app, "warm_foods", Value::Null).unwrap(), Value::Null);
        assert_eq!(handle_json(&app, "mica", Value::Null).unwrap(), json!(false));
        assert_eq!(handle_json(&app, "show_window", Value::Null).unwrap(), Value::Null);
        assert_eq!(handle_json(&app, "set_unsaved", json!({ "unsaved": true })).unwrap(), Value::Null);
        assert_eq!(handle_json(&app, "launch_notes", Value::Null).unwrap(), json!([]));
        assert_eq!(handle_json(&app, "take_inbox", Value::Null).unwrap(), json!({ "reviews": [], "notes": [] }));
        assert_eq!(handle_json(&app, "auto_sync", Value::Null).unwrap(), Value::Null);
        assert_eq!(handle_json(&app, "check_update", Value::Null).unwrap(), Value::Null);

        // Error paths
        assert_eq!(handle_json(&app, "no_such_cmd", Value::Null).unwrap_err(), "Unknown command no_such_cmd");
        assert!(handle_json(&app, "rules_overview", json!({})).unwrap_err().starts_with("Bad arguments for rules_overview:"));

        // Damaged library blocks web_backup_export, backup_now, and web_save_review
        let bad_dir = scratch_dir().join(format!("tiffin-web-wasm-json-locked-{}", test_pid()));
        let _ = std::fs::remove_dir_all(&bad_dir);
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("library.json"), "{\"recipes\": [").unwrap();
        let bad_app = Services { state: Mutex::new(tiffin_commands::load_state(bad_dir.clone())), off: OffState::new(&bad_dir) };
        assert_eq!(handle_json(&bad_app, "web_backup_export", Value::Null).unwrap_err(), storage::DAMAGED);
        assert_eq!(handle_json(&bad_app, "backup_now", Value::Null).unwrap_err(), storage::DAMAGED);
        let rv_locked = handle_json(&bad_app, "import_text", json!({ "text": "Locked\n\nIngredients\n1 cup water\n\nDirections\n1. Boil." })).unwrap();
        assert_eq!(handle_json(&bad_app, "web_save_review", json!({ "review": rv_locked })).unwrap_err(), storage::DAMAGED);

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bad_dir);
    }

    #[test]
    fn binary_commands_roundtrip_photos_and_block_traversal_and_locks() {
        let (dir, app) = fresh_app("bytes");

        // add_photo (named stem + empty default stem + invalid image) -> read_photo round-trip
        let name_bytes = handle_bytes(&app, "add_photo", "Berry-Pie", &ONE_PX_BMP).unwrap();
        let name = String::from_utf8(name_bytes).unwrap();
        assert!(name.starts_with("berry-pie-") && name.ends_with(".jpg"), "{name}");
        let jpeg = handle_bytes(&app, "read_photo", &name, &[]).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]), "saved photo is JPEG");

        let default_name = String::from_utf8(handle_bytes(&app, "add_photo", "", &ONE_PX_BMP).unwrap()).unwrap();
        assert!(default_name.starts_with("photo-") && default_name.ends_with(".jpg"), "{default_name}");
        assert!(handle_bytes(&app, "add_photo", "bad", b"not an image").is_err());

        // write_photo -> read_photo round-trip (and invalid image refused)
        assert!(handle_bytes(&app, "write_photo", "fetched-1.jpg", &ONE_PX_BMP).unwrap().is_empty());
        let jpeg2 = handle_bytes(&app, "read_photo", "fetched-1.jpg", &[]).unwrap();
        assert!(jpeg2.starts_with(&[0xff, 0xd8]));
        assert!(handle_bytes(&app, "write_photo", "fetched-bad.jpg", b"not an image").is_err());

        // path traversal, dotfiles, NUL, and missing photo refused
        for bad in ["", ".", "..", ".hidden", "../library.json", "..\\x.jpg", "a/b.jpg", "C:x.jpg", "%2e%2e", "a\0b.jpg"] {
            assert!(!safe_photo_name(bad), "{bad:?}");
            assert_eq!(handle_bytes(&app, "read_photo", bad, &[]).unwrap_err(), "Photo not found.");
            assert_eq!(handle_bytes(&app, "write_photo", bad, &ONE_PX_BMP).unwrap_err(), "Invalid photo name.");
        }
        assert_eq!(handle_bytes(&app, "read_photo", "missing.jpg", &[]).unwrap_err(), "Photo not found.");
        assert_eq!(handle_bytes(&app, "unknown_bin", "", &[]).unwrap_err(), "Unknown binary command unknown_bin");

        // damaged library blocks add_photo and write_photo without creating photos/
        let bad_dir = scratch_dir().join(format!("tiffin-web-wasm-locked-{}", test_pid()));
        let _ = std::fs::remove_dir_all(&bad_dir);
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("library.json"), "{\"recipes\": [").unwrap();
        let bad_app = Services { state: Mutex::new(tiffin_commands::load_state(bad_dir.clone())), off: OffState::new(&bad_dir) };
        assert_eq!(handle_bytes(&bad_app, "add_photo", "x", &ONE_PX_BMP).unwrap_err(), storage::DAMAGED);
        assert_eq!(handle_bytes(&bad_app, "write_photo", "x.jpg", &ONE_PX_BMP).unwrap_err(), storage::DAMAGED);
        assert!(!storage::photos_dir(&bad_dir).exists());

        // TooNew library blocks add_photo and write_photo with READ_ONLY without creating photos/
        let newer_dir = scratch_dir().join(format!("tiffin-web-wasm-toonew-{}", test_pid()));
        let _ = std::fs::remove_dir_all(&newer_dir);
        std::fs::create_dir_all(&newer_dir).unwrap();
        std::fs::write(
            newer_dir.join("library.json"),
            format!(r#"{{"format": {}, "app_version": "9.0.0", "recipes": []}}"#, recipe_box::model::LIBRARY_FORMAT + 1),
        )
        .unwrap();
        let newer_app = Services { state: Mutex::new(tiffin_commands::load_state(newer_dir.clone())), off: OffState::new(&newer_dir) };
        assert_eq!(handle_bytes(&newer_app, "add_photo", "x", &ONE_PX_BMP).unwrap_err(), storage::READ_ONLY);
        assert_eq!(handle_bytes(&newer_app, "write_photo", "x.jpg", &ONE_PX_BMP).unwrap_err(), storage::READ_ONLY);
        assert!(!storage::photos_dir(&newer_dir).exists());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bad_dir);
        let _ = std::fs::remove_dir_all(&newer_dir);
    }

    #[test]
    fn ffi_abi_roundtrips_init_call_and_call_bytes() {
        let dir = scratch_dir().join(format!("tiffin-web-wasm-ffi-{}", test_pid()));
        let dir2 = scratch_dir().join(format!("tiffin-web-wasm-ffi-dup-{}", test_pid()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
        let write_buf = |b: &[u8]| -> (*mut u8, usize) {
            let p = alloc(b.len());
            if !b.is_empty() {
                unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len()) };
            }
            (p, b.len())
        };
        let read_reply = |len: usize| -> (Vec<u8>, bool) {
            assert_eq!(len, reply_len());
            let bytes = if len == 0 {
                assert!(!reply_ptr().is_null());
                vec![]
            } else {
                unsafe { std::slice::from_raw_parts(reply_ptr(), len) }.to_vec()
            };
            (bytes, reply_is_err() != 0)
        };

        // alloc/dealloc boundary cases: zero length, oversized layout, null dealloc
        let z = alloc(0);
        assert!(!z.is_null());
        assert!(alloc(usize::MAX).is_null());
        unsafe {
            dealloc(z, 0);
            dealloc(std::ptr::null_mut(), 8);
        }

        // call and call_bytes before init return "Tiffin isn't initialized yet."
        let (np, nl) = write_buf(b"mica");
        let pre_len = unsafe { call(np, nl, z, 0) };
        let (pre_err, is_err) = read_reply(pre_len);
        assert!(is_err && String::from_utf8(pre_err).unwrap() == "Tiffin isn't initialized yet.");
        let pre_bin_len = unsafe { call_bytes(np, nl, z, 0, z, 0) };
        let (pre_bin_err, is_err) = read_reply(pre_bin_len);
        assert!(is_err && String::from_utf8(pre_bin_err).unwrap() == "Tiffin isn't initialized yet.");
        unsafe { dealloc(np, nl) };

        // init with empty dir fails
        assert_eq!(unsafe { init(z, 0) }, 1);
        assert_eq!(reply_is_err(), 1);

        // real init succeeds; second init on a DIFFERENT dir fails without creating library.json there
        let d_str = dir.display().to_string();
        let (dp, dl) = write_buf(d_str.as_bytes());
        assert_eq!(unsafe { init(dp, dl) }, 0);
        assert_eq!(reply_is_err(), 0);
        let (_, is_err) = read_reply(0);
        assert!(!is_err);
        unsafe { dealloc(dp, dl) };

        let d2_str = dir2.display().to_string();
        let (d2p, d2l) = write_buf(d2_str.as_bytes());
        assert_eq!(unsafe { init(d2p, d2l) }, 1);
        let (dup_err, is_err) = read_reply(reply_len());
        assert!(is_err && String::from_utf8(dup_err).unwrap() == "Already initialized.");
        assert!(!dir2.join("library.json").exists(), "duplicate init must not touch disk");
        unsafe { dealloc(d2p, d2l) };

        // call JSON command via FFI (with JSON args and with empty args_len == 0)
        let (np, nl) = write_buf(b"overview");
        let (ap, al) = write_buf(br#"{"today":"2026-11-24"}"#);
        let len = unsafe { call(np, nl, ap, al) };
        let (bytes, is_err) = read_reply(len);
        assert!(!is_err);
        let ov: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(ov["counts"][0].as_u64().unwrap() > 0);
        unsafe {
            dealloc(np, nl);
            dealloc(ap, al);
        }

        let (np, nl) = write_buf(b"mica");
        let len = unsafe { call(np, nl, z, 0) };
        let (bytes, is_err) = read_reply(len);
        assert!(!is_err && bytes == b"false");
        unsafe { dealloc(np, nl) };

        // invalid JSON, invalid UTF-8, null pointer, and oversized len (> isize::MAX) return errors safely
        let (np, nl) = write_buf(b"overview");
        let (bp, bl) = write_buf(b"{bad");
        let len = unsafe { call(np, nl, bp, bl) };
        let (err_bytes, is_err) = read_reply(len);
        assert!(is_err && String::from_utf8(err_bytes).unwrap().starts_with("Bad JSON for overview:"));
        unsafe { dealloc(bp, bl) };

        let (bad_utf8_p, bad_utf8_l) = write_buf(&[0xff, 0xfe]);
        let len = unsafe { call(np, nl, bad_utf8_p, bad_utf8_l) };
        let (err_bytes, is_err) = read_reply(len);
        assert!(is_err && String::from_utf8(err_bytes).unwrap().starts_with("Invalid UTF-8:"));
        unsafe { dealloc(bad_utf8_p, bad_utf8_l) };

        let len = unsafe { call(np, nl, std::ptr::null(), 5) };
        assert_eq!(reply_is_err(), 1);
        assert!(len > 0);

        let len = unsafe { call(np, nl, z, (isize::MAX as usize) + 1) };
        let (err_bytes, is_err) = read_reply(len);
        assert!(is_err && String::from_utf8(err_bytes).unwrap().contains("maximum slice size"));
        unsafe { dealloc(np, nl) };

        // call_bytes round-trip: add_photo -> read_photo
        let (np, nl) = write_buf(b"add_photo");
        let (sp, sl) = write_buf(b"FFI-Soup");
        let (bp, bl) = write_buf(&ONE_PX_BMP);
        let len = unsafe { call_bytes(np, nl, sp, sl, bp, bl) };
        let (name_bytes, is_err) = read_reply(len);
        assert!(!is_err);
        let photo_name = String::from_utf8(name_bytes).unwrap();
        assert!(photo_name.starts_with("ffi-soup-") && photo_name.ends_with(".jpg"));
        unsafe {
            dealloc(np, nl);
            dealloc(sp, sl);
            dealloc(bp, bl);
        }

        let (np, nl) = write_buf(b"read_photo");
        let (ap, al) = write_buf(photo_name.as_bytes());
        let len = unsafe { call_bytes(np, nl, ap, al, z, 0) };
        let (img_bytes, is_err) = read_reply(len);
        assert!(!is_err && img_bytes.starts_with(&[0xff, 0xd8]));
        unsafe {
            dealloc(np, nl);
            dealloc(ap, al);
        }

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dir2);
    }

    #[test]
    fn web_network_bridge_step_commands_match_desktop_rules() {
        let (dir, app) = fresh_app("net");

        // import_url prepare & finish
        assert_eq!(handle_json(&app, "web_prepare_import_url", json!({ "url": "example.com/soup" })).unwrap(), json!("https://example.com/soup"));
        assert!(handle_json(&app, "web_prepare_import_url", json!({ "url": "ftp://example.com/soup" })).unwrap_err().starts_with(recipe_box::importer::ONLY_WEB));
        assert!(handle_json(&app, "web_prepare_import_url", json!({ "url": "not a url" })).is_err());

        let html = r#"<html><head><script type="application/ld+json">{"@type":"Recipe","name":"Mint Tea","recipeIngredient":["1 cup water","5 mint leaves"],"recipeInstructions":["Boil water and steep mint."],"image":"https://example.com/mint.jpg"}</script></head><body></body></html>"#;
        let rv = handle_json(&app, "web_finish_import_url", json!({ "html": html, "url": "https://example.com/mint" })).unwrap();
        assert_eq!(rv["recipe"]["name"], "Mint Tea");
        assert_eq!(rv["origin"], "Web page");
        assert_eq!(rv["image_url"], "https://example.com/mint.jpg");

        // off_search prepare: short query, bad barcode, needs_email
        assert!(handle_json(&app, "web_off_prepare", json!({ "q": "a" })).unwrap_err().contains("two letters"));
        let bad_bc = handle_json(&app, "web_off_prepare", json!({ "q": "4006381333932" })).unwrap();
        assert_eq!(bad_bc["kind"], "done");
        assert_eq!(bad_bc["result"]["status"], "bad_barcode");
        let need_email = handle_json(&app, "web_off_prepare", json!({ "q": "yogurt" })).unwrap();
        assert_eq!(need_email["kind"], "done");
        assert_eq!(need_email["result"]["status"], "needs_email");

        // save contact email, then prepare barcode (direct product, including UPC-E expansion) and text search (helper)
        handle_json(&app, "set_off_email", json!({ "email": "cook@example.com" })).unwrap();
        let prod = handle_json(&app, "web_off_prepare", json!({ "q": "4006381333931" })).unwrap();
        assert_eq!(prod["kind"], "product");
        assert_eq!(prod["code"], "4006381333931");
        assert!(prod["url"].as_str().unwrap().starts_with("https://world.openfoodfacts.org/api/v2/product/4006381333931.json?fields="));
        assert!(prod["user_agent"].as_str().unwrap().contains("cook@example.com"));
        let upce = handle_json(&app, "web_off_prepare", json!({ "q": "04252614" })).unwrap();
        assert_eq!(upce["kind"], "product");
        assert_eq!(upce["code"], "042100005264");

        // finish product: valid, missing (no_product), invalid JSON (slow), and web_off_finish barcode empty / malformed answer
        let ok_prod = handle_json(
            &app,
            "web_off_finish_product",
            json!({ "q": "4006381333931", "code": "4006381333931", "body": r#"{"status":1,"product":{"product_name":"Pretzel","brands":"Test","nutriments":{"energy-kcal_100g":380}}}"# }),
        )
        .unwrap();
        assert_eq!(ok_prod["status"], "ok");
        assert_eq!(ok_prod["added"], 1);
        let no_prod = handle_json(&app, "web_off_finish_product", json!({ "q": "4006381333931", "code": "4006381333931", "body": r#"{"status":0}"# })).unwrap();
        assert_eq!(no_prod["status"], "no_product");
        let bad_json = handle_json(&app, "web_off_finish_product", json!({ "q": "4006381333931", "code": "4006381333931", "body": "not json" })).unwrap();
        assert_eq!(bad_json["status"], "slow");
        let empty_bc = handle_json(&app, "web_off_finish", json!({ "q": "4006381333931", "barcode": true, "answer": { "answer": "empty" } })).unwrap();
        assert_eq!(empty_bc["status"], "no_product");
        let bad_ans = handle_json(&app, "web_off_finish", json!({ "q": "oats", "barcode": false, "answer": { "answer": "bogus", "value": -1 } })).unwrap();
        assert_eq!(bad_ans["status"], "slow");

        // text search prepare + finish (including Limited blocking the next prepare)
        let srch = handle_json(&app, "web_off_prepare", json!({ "q": "  oats " })).unwrap();
        assert_eq!(srch["kind"], "search");
        assert_eq!(srch["text"], "oats");
        assert_eq!(srch["email"], "cook@example.com");
        let lim = handle_json(&app, "web_off_finish", json!({ "q": "oats", "barcode": false, "answer": { "answer": "limited", "value": 45 } })).unwrap();
        assert_eq!(lim["status"], "limited");
        assert_eq!(lim["retry_in"], 45);
        let wait = handle_json(&app, "web_off_prepare", json!({ "q": "oats" })).unwrap();
        assert_eq!(wait["kind"], "done");
        assert_eq!(wait["result"]["status"], "wait");
        assert!(wait["result"]["retry_in"].as_u64().unwrap() > 0);

        // label_lookup prepare & finish (barcode + OFF link + product URL + NEEDS_EMAIL)
        let lp_bc = handle_json(&app, "web_label_prepare", json!({ "input": "4006381333931" })).unwrap();
        assert_eq!(lp_bc["kind"], "barcode");
        assert_eq!(lp_bc["code"], "4006381333931");
        let lp_off_link = handle_json(&app, "web_label_prepare", json!({ "input": "https://world.openfoodfacts.org/product/4006381333931/pretzel" })).unwrap();
        assert_eq!(lp_off_link["kind"], "barcode");
        assert_eq!(lp_off_link["code"], "4006381333931");
        let lp_url = handle_json(&app, "web_label_prepare", json!({ "input": "example.com/granola" })).unwrap();
        assert_eq!(lp_url["kind"], "url");
        assert_eq!(lp_url["url"], "https://example.com/granola");

        let g_off = handle_json(
            &app,
            "web_label_finish_off",
            json!({ "code": "4006381333931", "body": r#"{"status":1,"product":{"brands":"Fage","product_name":"Yogurt","serving_size":"170g","serving_quantity":170,"nutriments":{"energy-kcal_100g":59,"proteins_100g":10}}}"# }),
        )
        .unwrap();
        assert_eq!(g_off["food"]["name"], "Fage Yogurt");
        assert_eq!(g_off["food"]["barcode"], "4006381333931");
        assert_eq!(handle_json(&app, "web_label_finish_off", json!({ "code": "4006381333931", "body": r#"{"status":0}"# })).unwrap_err(), labels::BARCODE_NOT_FOUND);
        assert!(handle_json(&app, "web_label_finish_off", json!({ "code": "4006381333931", "body": "not json" })).is_err());

        let label_html = r#"<html><head><title>Granola - Store</title></head><body><div>Nutrition Facts Serving size 1 cup (50g) Calories 200 Total Fat 6g Total Carbohydrate 30g Dietary Fiber 4g Total Sugars 8g Protein 6g Sodium 90mg</div></body></html>"#;
        let g_html = handle_json(&app, "web_label_finish_html", json!({ "html": label_html, "url": "https://example.com/granola" })).unwrap();
        assert_eq!(g_html["food"]["name"], "Granola");
        assert_eq!(g_html["food"]["serving_g"], 50.0);
        assert_eq!(g_html["source"], "Product page (example.com)");
        assert_eq!(
            handle_json(&app, "web_label_finish_html", json!({ "html": "<html><body>nothing here</body></html>", "url": "https://example.com/x" })).unwrap_err(),
            labels::NO_NUTRITION_ON_PAGE
        );

        // clearing email makes barcode label_lookup return NEEDS_EMAIL
        handle_json(&app, "set_off_email", json!({ "email": "" })).unwrap();
        assert_eq!(handle_json(&app, "web_label_prepare", json!({ "input": "4006381333931" })).unwrap_err(), off::NEEDS_EMAIL);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Build a crisp uncompressed 24-bit BMP of EAN-13 `4006381333931` (115 modules × 4 px = 460 × 160).
    fn ean13_4006381333931_bmp() -> Vec<u8> {
        // First digit 4 -> parity LGLLGG for digits 0,0,6,3,8,1; right side R for 3,3,3,9,3,1.
        let bits = concat!(
            "0000000000", // left quiet zone
            "101",        // start guard
            "0001101",    // 0 (L)
            "0100111",    // 0 (G)
            "0101111",    // 6 (L)
            "0111101",    // 3 (L)
            "0001001",    // 8 (G)
            "0110011",    // 1 (G)
            "01010",      // center guard
            "1000010",    // 3 (R)
            "1000010",    // 3 (R)
            "1000010",    // 3 (R)
            "1110100",    // 9 (R)
            "1000010",    // 3 (R)
            "1100110",    // 1 (R)
            "101",        // end guard
            "0000000000"  // right quiet zone
        );
        let scale = 4u32;
        let w = (bits.len() as u32) * scale; // 115 * 4 = 460 (460*3 = 1380, divisible by 4)
        let h = 160u32;
        let row_bytes = w * 3;
        let pixel_bytes = row_bytes * h;
        let file_size = 54 + pixel_bytes;
        let mut out = Vec::with_capacity(file_size as usize);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&file_size.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&24u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&pixel_bytes.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut bar_row = Vec::with_capacity(row_bytes as usize);
        for b in bits.bytes() {
            let px = if b == b'1' { [0u8, 0, 0] } else { [255u8, 255, 255] };
            for _ in 0..scale {
                bar_row.extend_from_slice(&px);
            }
        }
        let white_row = vec![255u8; row_bytes as usize];
        for y in 0..h {
            if (20..h - 20).contains(&y) {
                out.extend_from_slice(&bar_row);
            } else {
                out.extend_from_slice(&white_row);
            }
        }
        out
    }

    #[test]
    fn barcode_and_label_photo_binary_commands() {
        let (dir, app) = fresh_app("scan");

        // Real EAN-13 barcode decoded offline inside WASM
        let bmp = ean13_4006381333931_bmp();
        let code = handle_bytes(&app, "barcode_photo", "", &bmp).unwrap();
        assert_eq!(String::from_utf8(code).unwrap(), "4006381333931");

        // Blank 1x1 image has no barcode -> empty bytes; corrupt bytes -> error
        assert!(handle_bytes(&app, "barcode_photo", "", &ONE_PX_BMP).unwrap().is_empty());
        assert!(handle_bytes(&app, "barcode_photo", "", b"not an image").is_err());

        // label_photo returns the crate's non-native fallback message without Windows OCR
        let err = handle_bytes(&app, "label_photo", "pack.jpg", &ONE_PX_BMP).unwrap_err();
        assert!(err.starts_with("Reading a label photo isn't available here yet."), "{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
