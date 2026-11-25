//! Settings commands that need the desktop (app/folder info, sync folders, native backups,
//! tracker exports, the Claude inbox, Explorer, online label lookup/OCR, self-tests).
//! Everything else about settings is shared, in tiffin_commands::settings.

use crate::{mutate, Shared};
use recipe_box::model::{self, Library, MyFood};
use recipe_box::tracker::journal;
use recipe_box::{cooking, health, inbox, labels, nutrition, storage};
use serde::Serialize;
use std::path::Path;
pub(crate) use tiffin_commands::settings::backup_parts;
use tiffin_commands::settings::{delete_health_data_in, run_self_tests, settings_info_in, Check, DesktopInfo, Info};

#[tauri::command]
pub(crate) fn settings_info(state: tauri::State<Shared>) -> Info {
    let onedrive = ["OneDrive", "OneDriveConsumer"].iter().find_map(|v| std::env::var(v).ok()).map(|p| Path::new(&p).join("Tiffin").display().to_string());
    let dropbox = storage::documents_dir().parent().map(|h| h.join("Dropbox")).filter(|d| d.exists()).map(|d| d.join("Tiffin").display().to_string());
    let dir = inbox::inbox_dir();
    settings_info_in(
        &state,
        DesktopInfo {
            onedrive,
            dropbox,
            inbox_ready: dir.join(inbox::README_NAME).exists(),
            inbox_dir: dir.display().to_string(),
            instructions: inbox::claude_instructions(),
        },
    )
}

/// Sync with the shared folder now (a snapshot is taken first, inside storage::sync_folder).
#[tauri::command]
pub(crate) async fn sync_now(state: tauri::State<'_, Shared>) -> Result<String, String> {
    mutate(&state, |st| storage::sync_folder(&mut st.lib, &mut st.store).map(|r| r.message))
}

/// The automatic sync the page runs at start and every few minutes: nothing happens unless a
/// folder is set and auto-sync is on and the library can be changed (not damaged or newer).
#[tauri::command]
pub(crate) async fn auto_sync(state: tauri::State<'_, Shared>) -> Result<Option<String>, String> {
    let on = {
        let s = state.lock().unwrap();
        !s.lib.settings.sync_folder.trim().is_empty() && s.lib.settings.auto_sync && s.store.check_unlocked().is_ok()
    };
    if !on {
        return Ok(None);
    }
    sync_now(state).await.map(Some)
}

/// One-file backup in Documents\Tiffin Backups (a test copy's own Backups folder), plus a library-only copy in the sync folder if set.
#[tauri::command]
pub(crate) async fn backup_now(state: tauri::State<'_, Shared>) -> Result<String, String> {
    let (lib, photos, jdir, into) = {
        let s = state.lock().unwrap();
        let (lib, photos, jdir) = backup_parts(&s)?;
        (lib, photos, jdir, storage::backups_dir(&s.store.dir))
    };
    tauri::async_runtime::spawn_blocking(move || backup_into(&lib, &photos, &jdir, &into)).await.map_err(|e| e.to_string())?
}

/// Delete all health data: the journal folder and the diary in every backup. The lock is held throughout, so no
/// diary change can write a cached month back; it runs off the main thread because large backups take a while.
#[tauri::command]
pub(crate) async fn delete_health_data(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        delete_health_data_in(&app.state::<Shared>())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Write the full backup (library, photos, diary and profile) into `backups`, and the library-only
/// copy into the sync folder when one is set. A diary month that can't be read is left out and named.
fn backup_into(lib: &Library, photos: &Path, journal_dir: &Path, backups: &Path) -> Result<String, String> {
    let (bundle, left_out) = journal::bundle_from_disk(journal_dir);
    let p = storage::backup_with_photos(lib, backups, Some(photos), bundle.as_ref()).map_err(|e| format!("Backup failed: {e}"))?;
    if !lib.settings.sync_folder.is_empty() {
        // PRIV-01: diary, exercise, body and profile never go to the sync folder
        let _ = storage::backup_with_photos(lib, Path::new(&lib.settings.sync_folder), Some(photos), None);
    }
    let mut msg = format!("Backed up to {}", p.display());
    if !left_out.is_empty() {
        msg.push_str(&format!(" These diary months couldn't be read and were left out: {}.", left_out.join(", ")));
    }
    Ok(msg)
}

/// Write a tracker file for every recipe (estimating nutrition where there is none yet).
#[tauri::command]
pub(crate) async fn send_all_to_tracker(state: tauri::State<'_, Shared>) -> Result<String, String> {
    state.lock().unwrap().store.check_unlocked()?;
    let (recipes, svc, folder) = {
        let s = state.lock().unwrap();
        (s.lib.recipes.iter().filter(|r| !r.in_trash).cloned().collect::<Vec<_>>(), s.lib.settings.tracker_service.clone(), s.lib.settings.tracker_folder.clone())
    };
    let svc2 = svc.clone();
    let done = tauri::async_runtime::spawn_blocking(move || {
        let mut done = vec![];
        for mut r in recipes {
            let calc = r.nutrition.is_none().then(|| nutrition::calculate(&r).total);
            if calc.is_some() {
                r.nutrition = calc.clone();
            }
            if health::export(&r, &svc2, &folder).is_ok() {
                done.push((r.id, calc));
            }
        }
        done
    })
    .await
    .map_err(|e| e.to_string())?;
    let n = done.len();
    let _ = mutate(&state, |st| {
        let t = model::now_secs();
        for (id, calc) in done {
            if let Some(r) = st.lib.recipe_mut(id) {
                r.sent_to_tracker = Some(t);
                if r.nutrition.is_none() {
                    r.nutrition = calc;
                }
            }
        }
        Ok(())
    });
    Ok(format!("Prepared {n} recipe{} for {svc}", if n == 1 { "" } else { "s" }))
}

#[tauri::command]
pub(crate) fn setup_inbox() -> Result<String, String> {
    inbox::setup_inbox().map(|d| format!("Inbox ready: {}", d.display()))
}

/// Open one of the app's own folders in Explorer (only these, never a path from the page).
#[tauri::command]
pub(crate) fn open_folder(state: tauri::State<Shared>, which: String) -> Result<(), String> {
    let dir = match which.as_str() {
        "inbox" => inbox::inbox_dir(),
        "backups" => storage::backups_dir(&state.lock().unwrap().store.dir),
        "tracker" => {
            let set = &state.lock().unwrap().lib.settings;
            health::folder_for(&set.tracker_service, &set.tracker_folder)
        }
        _ => return Err("Unknown folder.".into()),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    cooking::open_external(&dir.display().to_string()).then_some(()).ok_or_else(|| "Couldn't open the folder.".into())
}

// ---- My Foods (online lookup & Windows label OCR) ----

/// What a product link, barcode or label photo filled in; the page merges it into the open form.
#[derive(Serialize)]
pub(crate) struct Guess {
    food: MyFood,
    source: String,
    missing: Vec<&'static str>,
    /// Sentences to check against the pack before saving (a misread-looking value, a weight worked out from ml).
    check: Vec<&'static str>,
}

impl From<labels::LabelGuess> for Guess {
    fn from(g: labels::LabelGuess) -> Self {
        Guess { food: g.food, source: g.source, missing: g.missing, check: g.check }
    }
}

/// A product link or a barcode (Open Food Facts) → label numbers. Goes online; a barcode with no saved contact email is refused with `needs_email`.
#[tauri::command]
pub(crate) async fn label_lookup(state: tauri::State<'_, Shared>, input: String) -> Result<Guess, String> {
    let email = state.lock().unwrap().lib.settings.off_email.clone();
    tauri::async_runtime::spawn_blocking(move || labels::lookup(input.trim(), &email).map(Guess::from)).await.map_err(|e| e.to_string())?
}

/// A photo of the Nutrition Facts panel (raw bytes, `name` header) → label numbers, read
/// offline with Windows' own text recognition.
#[tauri::command]
pub(crate) async fn label_photo(request: tauri::ipc::Request<'_>) -> Result<Guess, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("No picture was sent.".into());
    };
    let name: String = request.headers().get("name").and_then(|v| v.to_str().ok()).unwrap_or("label").chars().filter(|c| c.is_alphanumeric() || " .-_".contains(*c)).take(60).collect();
    let tmp = std::env::temp_dir().join(format!("recipebox-label-in-{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let res = tauri::async_runtime::spawn_blocking({
        let tmp = tmp.clone();
        move || labels::from_photo(&tmp)
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    let mut g = res?;
    g.source = format!("Photo of the label ({name})");
    Ok(g.into())
}

/// The release checks (timers, step-by-step, video links, site rules, the ingredient reader,
/// data safety), run against the sample library so nothing of the person's is touched.
#[tauri::command]
pub(crate) async fn self_tests(state: tauri::State<'_, Shared>) -> Result<Vec<Check>, String> {
    let overrides = state.lock().unwrap().lib.rule_overrides.clone();
    tauri::async_runtime::spawn_blocking(move || run_self_tests(overrides)).await.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Every backup file in a folder, newest name last, read back.
    fn backups_in(dir: &Path) -> Vec<storage::BackupFile> {
        let mut names: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == storage::BACKUP_EXT)).collect();
        names.sort();
        names.iter().map(|p| storage::read_backup(p).unwrap()).collect()
    }

    #[test]
    fn sync_folder_backup_has_no_journal() {
        use recipe_box::model::Nutrients;
        use recipe_box::tracker::journal::DiaryEntry;
        let d = std::env::temp_dir().join(format!("rb-desktop-privsync-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let shared = std::sync::Mutex::new(crate::load_state(d.clone()));
        shared.lock().unwrap().lib.settings.sync_folder = d.join("sync").display().to_string();
        let sentinel = DiaryEntry { day: "2026-11-13".into(), name: "SENTINEL-DIARY-7731".into(), basis: Nutrients { calories: Some(100.0), ..Default::default() }, ..Default::default() };
        crate::mutate_journal(&shared, |st| {
            let m = st.journal.month_mut("2026-11")?;
            m.entries.push(sentinel.clone());
            m.exercise.push(journal::ExerciseEntry { uid: "x1".into(), day: "2026-11-13".into(), name: "SENTINEL-EXERCISE-4411".into(), minutes: 30.0, est_kcal: Some(90.0), ..Default::default() });
            m.body.push(journal::BodyEntry { uid: "b1".into(), day: "2026-11-13".into(), measure: "waist".into(), value: 123.456, ..Default::default() });
            let template = recipe_box::tracker::goals::Template { id: 1, name: "SENTINEL-TEMPLATE-5523".into(), kcal_offset: 300, ..Default::default() };
            st.journal.profile_mut()?.targets.push(recipe_box::tracker::goals::TargetSet { effective_from: "2026-11-13".into(), kcal: 2100, templates: vec![template], ..Default::default() });
            Ok(())
        })
        .unwrap();
        let parts = || backup_parts(&shared.lock().unwrap()).unwrap();
        let (lib, photos, jdir) = parts();
        let msg = backup_into(&lib, &photos, &jdir, &d.join("backups")).unwrap();
        assert!(msg.starts_with("Backed up to ") && !msg.contains("left out"), "{msg}");
        let full = std::fs::read_dir(d.join("backups")).unwrap().flatten().next().unwrap().path();
        let text = std::fs::read_to_string(&full).unwrap();
        assert!(text.contains("SENTINEL-DIARY-7731") && text.contains("\"journal\""));
        for marker in ["SENTINEL-EXERCISE-4411", "SENTINEL-TEMPLATE-5523", "123.456"] {
            assert!(text.contains(marker), "the full backup holds {marker}");
        }
        assert_eq!(storage::read_backup(&full).unwrap().journal.unwrap().months.keys().collect::<Vec<_>>(), vec!["2026-11"]);
        let synced = std::fs::read_dir(d.join("sync")).unwrap().flatten().next().unwrap().path();
        let text = std::fs::read_to_string(&synced).unwrap();
        for marker in ["SENTINEL-DIARY-7731", "SENTINEL-EXERCISE-4411", "SENTINEL-TEMPLATE-5523", "123.456"] {
            assert!(!text.contains(marker), "the sync-folder copy never holds {marker}");
        }
        assert!(!text.contains("journal"));
        assert!(storage::read_backup(&synced).unwrap().journal.is_none());

        // a damaged month is left out and named; the backup still happens
        std::fs::write(d.join("journal").join("2026-10.json"), "{\"entries\": [").unwrap();
        let msg = backup_into(&lib, &photos, &jdir, &d.join("backups")).unwrap();
        assert!(msg.contains("2026-10"), "{msg}");
        let all = backups_in(&d.join("backups"));
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|b| b.journal.as_ref().is_some_and(|j| j.months.keys().collect::<Vec<_>>() == vec!["2026-11"])));

        // no journal files at all: no diary section
        let e = d.join("empty");
        let msg = backup_into(&lib, &photos, &e.join("journal"), &e.join("backups")).unwrap();
        assert!(!msg.contains("left out"));
        assert!(backups_in(&e.join("backups"))[0].journal.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
