//! Open Food Facts search and barcode photos: the two foods commands that need the desktop (a
//! worker thread for the network, the raw picture bytes Tauri sends). Everything else about foods
//! is shared, in tiffin_commands::foods.

use recipe_box::foods::off;
use recipe_box::model;
use tiffin_commands::foods::{begin, finish, named, rows, OffState, OnlineResult};
use tiffin_commands::Shared;

/// Search Online: one request with only the typed words. The limit is checked here, not only by the button.
#[tauri::command]
pub(crate) async fn off_search(state: tauri::State<'_, Shared>, off: tauri::State<'_, OffState>, q: String) -> Result<OnlineResult, String> {
    if !off::query_ready(&q) {
        return Err("Type at least two letters to search online.".into());
    }
    // a barcode is looked up as one product; digits that don't add up are refused before anything is sent
    let barcode = match off::barcode_in(&q) {
        Some(Err(_)) => return Ok(OnlineResult { status: "bad_barcode", retry_in: 0, added: 0, rows: rows(&state, &off, &q, false) }),
        Some(Ok(code)) => Some(code),
        None => None,
    };
    // no email, no send: the page asks for one, and the limiter doesn't count a request that never left
    if off::valid_email(&state.lock().unwrap().lib.settings.off_email).is_err() {
        return Ok(finish(&state, &off, &q, off::OffAnswer::NeedsEmail, model::now_secs()));
    }
    if let Some(wait) = begin(&off, model::now_secs()) {
        return Ok(OnlineResult { status: "wait", retry_in: wait, added: 0, rows: rows(&state, &off, &q, true) });
    }
    let text = q.trim().to_string();
    let email = state.lock().unwrap().lib.settings.off_email.clone();
    let answer = tauri::async_runtime::spawn_blocking(move || match barcode {
        // the code passed barcode_in, so product() can't refuse it; a refusal would read as "slow"
        Some(code) => off::product(&code, &email).unwrap_or(off::OffAnswer::Slow),
        None => off::search(&text, &email),
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(named(finish(&state, &off, &q, answer, model::now_secs()), off::barcode_in(&q).is_some()))
}

/// A photo of a product's barcode (raw bytes) → the code printed on it, or `None` when none can be read. Offline.
#[tauri::command]
pub(crate) async fn barcode_photo(request: tauri::ipc::Request<'_>) -> Result<Option<String>, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("No picture was sent.".into());
    };
    // one file per call, so two scans at once can't read each other's picture
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("recipebox-barcode-in-{}-{stamp}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let res = tauri::async_runtime::spawn_blocking({
        let tmp = tmp.clone();
        move || recipe_box::labels::barcode_from_photo(&tmp)
    })
    .await
    .map_err(|e| e.to_string());
    let _ = std::fs::remove_file(&tmp);
    res?
}

