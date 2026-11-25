//! Importing commands that need the desktop (fetching web pages and photos, the inbox folder,
//! site-rules sync). Everything else about importing is shared, in tiffin_commands::importing.

use crate::{mutate, Shared};
use recipe_box::importer;
use recipe_box::inbox::{self, InboxItem};
use recipe_box::{nutrition, storage};
use serde::Serialize;
use tiffin_commands::importing::{dup_of, from_html, load_rules, review_of, rules_overview_in, save_review_in, web_url, Review, RulesOverview, Saved};

async fn fetch(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || importer::fetch(&url)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn import_url(state: tauri::State<'_, Shared>, url: String) -> Result<Review, String> {
    let url = web_url(&url)?;
    let html = fetch(url.clone()).await.map_err(|e| format!("Couldn't read {url}: {e}"))?;
    Ok(from_html(&state, &html, &url, "Web page"))
}

#[tauri::command]
pub(crate) fn save_review(state: tauri::State<Shared>, review: Review) -> Result<Saved, String> {
    let (saved, photo) = save_review_in(&state, review)?;
    if let Some(p) = photo {
        std::thread::spawn(move || {
            if let Ok(bytes) = importer::fetch_bytes(&p.url) {
                let _ = storage::write_photo(&bytes, &p.dir, &p.file);
            }
        });
    }
    Ok(saved)
}

#[derive(Serialize, Default)]
pub(crate) struct Inbox {
    reviews: Vec<Review>,
    notes: Vec<String>,
}

/// Pick up anything dropped in the inbox folder (by Claude, `--import`, or by hand). While the
/// library can't be changed (damaged or newer), the files stay where they are.
#[tauri::command]
pub(crate) async fn take_inbox(state: tauri::State<'_, Shared>) -> Result<Inbox, String> {
    // Locked (damaged or newer library): leave the files where they are until the problem is gone (D-06).
    let locked = state.lock().unwrap().store.problem.is_some();
    if locked {
        return Ok(Inbox::default());
    }
    let items = tauri::async_runtime::spawn_blocking(inbox::take_waiting).await.map_err(|e| e.to_string())?;
    let mut out = Inbox::default();
    for (name, item) in items {
        let origin = format!("Inbox: {name}");
        match item {
            InboxItem::Url(u) => match web_url(&u) {
                Ok(u) => match fetch(u.clone()).await {
                    Ok(html) => out.reviews.push(from_html(&state, &html, &u, &origin)),
                    Err(e) => out.notes.push(format!("{u}: {e}")),
                },
                Err(e) => out.notes.push(format!("{name}: {e}")),
            },
            InboxItem::Html { html, url } => out.reviews.push(from_html(&state, &html, &url, &origin)),
            InboxItem::Text(t) => {
                let s = state.lock().unwrap();
                let (body, extra) = t.split_once('\u{1}').map_or((t.as_str(), None), |(a, b)| (a, Some(b)));
                let mut rv = review_of(importer::import_text(body, &s.lib), &s.lib, &origin);
                if let Some(e) = extra {
                    importer::apply_extra(&mut rv.recipe, e);
                    rv.duplicate = dup_of(&s.lib, &rv.recipe);
                }
                out.reviews.push(rv);
            }
            InboxItem::Food(f) => {
                let food = f.name.clone();
                // the same gate the form uses: no calories or a negative value never reaches My Foods
                if let Err(p) = recipe_box::foods::check_food(&f) {
                    out.notes.push(format!("{food}: {}", p.message));
                    continue;
                }
                match mutate(&state, |st| {
                    nutrition::save_my_food_from_file(&mut st.lib, f);
                    Ok(())
                }) {
                    Ok(()) => out.notes.push(format!("Added {food} to My Foods. Recipes updated.")),
                    Err(e) => out.notes.push(format!("{food}: {e}")),
                }
            }
        }
    }
    Ok(out)
}

#[tauri::command]
pub(crate) fn rules_overview(state: tauri::State<Shared>, test: bool) -> RulesOverview {
    rules_overview_in(&state, test, inbox::inbox_dir().display().to_string())
}

/// Load rules-update.json from the sync folder, if one is set.
#[tauri::command]
pub(crate) fn rules_from_sync(state: tauri::State<Shared>) -> Result<String, String> {
    let folder = state.lock().unwrap().lib.settings.sync_folder.clone();
    if folder.trim().is_empty() {
        return Err("No sync folder is set up (Settings › Sync).".into());
    }
    let p = std::path::Path::new(&folder).join("rules-update.json");
    let text = std::fs::read_to_string(&p).map_err(|_| format!("There's no rules-update.json in {folder}."))?;
    load_rules(&state, text)
}
