//! Settings: sync, backup and restore, Version History, My Foods, health trackers, the Claude
//! inbox and app info. Same rules as the classic Settings window.

use crate::{mutate, replace_library, replace_library_then, AppState, Shared};
use recipe_box::model::{self, Library, MyFood};
use recipe_box::tracker::journal;
use recipe_box::{cooking, health, importer, inbox, labels, nutrition, selftest, storage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub(crate) struct Info {
    version: &'static str,
    sync_folder: String,
    auto_sync: bool,
    device_name: String,
    last_sync: Option<String>,
    /// suggested sync folders that exist on this computer
    onedrive: Option<String>,
    dropbox: Option<String>,
    backups_dir: String,
    tracker_service: String,
    tracker_folder: String,
    tracker_default: String,
    tracker_auto_send: bool,
    /// the contact email Open Food Facts is told about; empty = the next online search asks
    off_email: String,
    services: &'static [&'static str],
    service_url: &'static str,
    inbox_dir: String,
    inbox_ready: bool,
    instructions: String,
    recipes: usize,
    favorites: usize,
}

#[tauri::command]
pub(crate) fn settings_info(state: tauri::State<Shared>) -> Info {
    let s = state.lock().unwrap();
    let set = &s.lib.settings;
    let onedrive = ["OneDrive", "OneDriveConsumer"].iter().find_map(|v| std::env::var(v).ok()).map(|p| Path::new(&p).join("Tiffin").display().to_string());
    let dropbox = storage::documents_dir().parent().map(|h| h.join("Dropbox")).filter(|d| d.exists()).map(|d| d.join("Tiffin").display().to_string());
    let dir = inbox::inbox_dir();
    Info {
        version: model::APP_VERSION,
        sync_folder: set.sync_folder.clone(),
        auto_sync: set.auto_sync,
        device_name: set.device_name.clone(),
        last_sync: set.last_sync.map(storage::fmt_stamp),
        onedrive,
        dropbox,
        backups_dir: storage::backups_dir().display().to_string(),
        tracker_service: set.tracker_service.clone(),
        tracker_folder: set.tracker_folder.clone(),
        tracker_default: health::default_folder(&set.tracker_service).display().to_string(),
        tracker_auto_send: set.tracker_auto_send,
        off_email: set.off_email.clone(),
        services: health::SERVICES,
        service_url: health::service_url(&set.tracker_service),
        inbox_ready: dir.join(inbox::README_NAME).exists(),
        inbox_dir: dir.display().to_string(),
        instructions: inbox::claude_instructions(),
        recipes: s.lib.recipes.iter().filter(|r| !r.in_trash).count(),
        favorites: s.lib.recipes.iter().filter(|r| r.favorite && !r.in_trash).count(),
    }
}

/// One settings change; only the fields sent are touched.
#[derive(Deserialize, Default)]
#[serde(default)]
pub(crate) struct Change {
    sync_folder: Option<String>,
    auto_sync: Option<bool>,
    device_name: Option<String>,
    tracker_service: Option<String>,
    tracker_folder: Option<String>,
    tracker_auto_send: Option<bool>,
}

#[tauri::command]
pub(crate) fn set_settings(state: tauri::State<Shared>, change: Change) -> Result<(), String> {
    mutate(&state, |st| {
        let set = &mut st.lib.settings;
        if let Some(v) = change.sync_folder {
            set.sync_folder = v.trim().to_string();
        }
        if let Some(v) = change.auto_sync {
            set.auto_sync = v;
        }
        if let Some(v) = change.device_name.filter(|v| !v.trim().is_empty()) {
            set.device_name = v.trim().to_string();
        }
        if let Some(v) = change.tracker_service.filter(|v| health::SERVICES.contains(&v.as_str())) {
            set.tracker_service = v;
        }
        if let Some(v) = change.tracker_folder {
            set.tracker_folder = v.trim().to_string();
        }
        if let Some(v) = change.tracker_auto_send {
            set.tracker_auto_send = v;
        }
        Ok(())
    })
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

/// One-file backup in Documents\Tiffin Backups, plus a library-only copy in the sync folder if set.
#[tauri::command]
pub(crate) async fn backup_now(state: tauri::State<'_, Shared>) -> Result<String, String> {
    let (lib, photos, jdir) = {
        let s = state.lock().unwrap();
        backup_parts(&s)?
    };
    tauri::async_runtime::spawn_blocking(move || backup_into(&lib, &photos, &jdir, &storage::backups_dir())).await.map_err(|e| e.to_string())?
}

/// What a backup is made from: the library, the photos folder and the journal folder. Refuses while
/// the library is locked, so a backup of the empty placeholder of a damaged library can never become
/// the newest backup.
pub(crate) fn backup_parts(st: &AppState) -> Result<(Library, PathBuf, PathBuf), String> {
    st.store.check_unlocked()?;
    Ok((st.lib.clone(), storage::photos_dir(&st.store.dir), journal::journal_dir(&st.store.dir)))
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

#[derive(Serialize)]
pub(crate) struct Saved {
    path: String,
    when: String,
    recipes: usize,
    /// snapshots only
    reason: String,
    favorites: usize,
    /// backups only: the file has a food diary section
    has_diary: bool,
}

fn sync_dirs(lib: &Library) -> Vec<PathBuf> {
    Some(&lib.settings.sync_folder).filter(|s| !s.is_empty()).map(PathBuf::from).into_iter().collect()
}

/// The backup list, or with `picked` (the text of a file the person chose) the one row for that file.
/// Whether a backup has a diary is decided here, so the page never reads backup files itself.
#[tauri::command]
pub(crate) fn backups(state: tauri::State<Shared>, picked: Option<String>) -> Result<Vec<Saved>, String> {
    if let Some(text) = picked {
        return picked_backup(&text).map(|r| vec![r]);
    }
    let extra = sync_dirs(&state.lock().unwrap().lib);
    Ok(storage::list_backups(&extra)
        .into_iter()
        .take(15)
        .map(|(p, created, n, has_diary)| Saved { path: p.display().to_string(), when: storage::fmt_stamp(created), recipes: n, reason: String::new(), favorites: 0, has_diary })
        .collect())
}

/// The row for a backup file the person picked (its text). The path is empty: it is restored from the text.
fn picked_backup(text: &str) -> Result<Saved, String> {
    let b = storage::parse_backup(text)?;
    Ok(Saved { path: String::new(), when: storage::fmt_stamp(b.created), recipes: b.library.recipes.len(), reason: String::new(), favorites: 0, has_diary: b.journal.is_some() })
}

#[tauri::command]
pub(crate) fn snapshots(state: tauri::State<Shared>) -> Result<Vec<Saved>, String> {
    let s = state.lock().unwrap();
    Ok(s.store
        .snapshots()
        .into_iter()
        .take(40)
        .map(|x| Saved { path: x.path.display().to_string(), when: storage::fmt_stamp(x.taken), recipes: x.recipes, reason: x.reason, favorites: x.favorites, has_diary: false })
        .collect())
}

#[tauri::command]
pub(crate) fn take_snapshot(state: tauri::State<Shared>) -> Result<(), String> {
    let mut g = state.lock().unwrap();
    let st = &mut *g;
    st.store.check_unlocked()?;
    st.store.snapshot(&st.lib, "taken by you")
}

/// Things every whole-library swap needs afterwards.
fn settle(st: &mut AppState) {
    st.lib.ensure_cuisines();
    model::assign_label_colors(&mut st.lib);
    nutrition::set_my_foods(&st.lib.my_foods);
}

/// "recipes" (or nothing) is recipes only, "everything" includes the diary; anything else is refused.
fn restore_scope(scope: Option<&str>) -> Result<bool, String> {
    match scope {
        None | Some("recipes") => Ok(false),
        Some("everything") => Ok(true),
        _ => Err("Unknown restore choice.".into()),
    }
}

/// Restore a backup file: from the list above, or a file the person picked (its text).
/// The current library goes to Version History first; this device keeps its name. `scope` is
/// "recipes" (the default) or "everything" (also the diary and profile, when the backup has them).
#[tauri::command]
pub(crate) fn restore_backup(state: tauri::State<Shared>, path: Option<String>, text: Option<String>, scope: Option<String>) -> Result<String, String> {
    let everything = restore_scope(scope.as_deref())?;
    let file = match (path, text) {
        (Some(p), _) => {
            let p = PathBuf::from(p);
            let known = storage::list_backups(&sync_dirs(&state.lock().unwrap().lib)).into_iter().any(|(b, _, _, _)| b == p);
            if !known {
                return Err("That backup isn't in the list any more.".into());
            }
            p
        }
        (None, Some(t)) => {
            let p = std::env::temp_dir().join(format!("recipe-box-restore.{}", storage::BACKUP_EXT));
            std::fs::write(&p, t).map_err(|e| e.to_string())?;
            p
        }
        _ => return Err("Choose a backup first.".into()),
    };
    restore_file(&state, &file, everything, &storage::backups_dir())
}

/// The restore itself, split from the command so a test can drive it. A library or diary from a newer
/// build, or a diary month with a bad name, is refused before anything changes. With `everything` and a
/// diary in the backup, the current diary is first backed up into `backups` and copied inside the journal
/// folder; a backup without a diary is recipes only, whatever was asked.
fn restore_file(state: &Shared, file: &Path, everything: bool, backups: &Path) -> Result<String, String> {
    let b = storage::read_backup(file)?;
    storage::ensure_not_newer(&b.library)?;
    let diary = if everything { b.journal.as_ref() } else { None };
    if let Some(j) = diary {
        journal::check_bundle(j)?;
        // While the library is damaged there is nothing to back up; the verified file copy in restore_bundle is the safety net.
        let parts = backup_parts(&state.lock().unwrap());
        if let Ok((lib, photos, jdir)) = parts {
            storage::backup_with_photos(&lib, backups, Some(&photos), journal::bundle_from_disk(&jdir).0.as_ref())
                .map_err(|e| format!("Couldn't back up your current library and diary first, so nothing was restored: {e}"))?;
        }
    }
    replace_library_then(state, |st| {
        st.store.snapshot(&st.lib, "before restore")?;
        let device = st.lib.settings.device_name.clone();
        storage::restore_photos(&b, &storage::photos_dir(&st.store.dir));
        st.lib = b.library.clone();
        st.lib.settings.device_name = device;
        st.lib.app_version = model::APP_VERSION.into();
        settle(st);
        let n = st.lib.recipes.len();
        Ok(if diary.is_some() { format!("Restored {n} recipes and your food diary. A backup of how things were before is in the list.") } else { format!("Restored {n} recipes") })
    }, |st| {
        // after the library is saved: if the library can't be saved the diary is still untouched
        if let Some(j) = diary {
            let dir = journal::journal_dir(&st.store.dir);
            journal::restore_bundle(&dir, j).map_err(|e| format!("Your recipes were restored, but the diary was not. {e}"))?;
            st.journal = journal::Journal::open(dir);
        }
        Ok(())
    })
}

/// Roll back to a Version History snapshot; the current version is saved first and settings stay.
#[tauri::command]
pub(crate) fn rollback(state: tauri::State<Shared>, path: String) -> Result<String, String> {
    rollback_to(&state, &PathBuf::from(&path))
}

/// The rollback itself, split from the command so a test can drive it.
fn rollback_to(state: &Shared, p: &Path) -> Result<String, String> {
    let known = state.lock().unwrap().store.snapshots().into_iter().any(|s| s.path == p);
    if !known {
        return Err("That version isn't in Version History any more.".into());
    }
    let mut old = storage::load_snapshot(p)?;
    storage::ensure_not_newer(&old)?;
    replace_library(state, |st| {
        st.store.snapshot(&st.lib, "before rollback")?;
        // While recovering from damage the in-memory settings are defaults, so the snapshot keeps its own.
        if !matches!(st.store.problem, Some(storage::Problem::Damaged { .. })) {
            old.settings = st.lib.settings.clone();
        }
        st.lib = old;
        settle(st);
        Ok("Rolled back".into())
    })
}

/// Start with an empty library (never the samples), after the damaged file has been kept. Refused unless the library is damaged.
#[tauri::command]
pub(crate) fn start_empty(state: tauri::State<Shared>) -> Result<String, String> {
    empty_library(&state)
}

fn empty_library(state: &Shared) -> Result<String, String> {
    replace_library(state, |st| {
        // Checked under the same lock as the swap, so nothing can unlock in between.
        if !matches!(st.store.problem, Some(storage::Problem::Damaged { .. })) {
            return Err("Start an Empty Library is only for a library file that couldn't be read. Nothing was changed.".into());
        }
        st.lib = Library::default();
        settle(st);
        Ok("Started an empty library".into())
    })
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
        "backups" => storage::backups_dir(),
        "tracker" => {
            let set = &state.lock().unwrap().lib.settings;
            health::folder_for(&set.tracker_service, &set.tracker_folder)
        }
        _ => return Err("Unknown folder.".into()),
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    cooking::open_external(&dir.display().to_string()).then_some(()).ok_or_else(|| "Couldn't open the folder.".into())
}

// ---- My Foods ----

#[tauri::command]
pub(crate) fn my_foods(state: tauri::State<Shared>) -> Vec<MyFood> {
    state.lock().unwrap().lib.my_foods.clone()
}

/// Add a food or save an edit (a name another food has is refused), then re-estimate every recipe not typed in by hand.
#[tauri::command]
pub(crate) fn save_my_food(state: tauri::State<Shared>, food: MyFood) -> Result<(), String> {
    // the crate decides what blocks a save (the form shows the same answer), so a page can't slip an invalid food in
    recipe_box::foods::check_food(&food).map_err(|p| p.message.to_string())?;
    mutate(&state, |st| {
        nutrition::check_my_food_name(&st.lib, &food)?; // checked inside mutate, after the reload, so it sees the current list
        nutrition::save_my_food(&mut st.lib, MyFood { name: food.name.trim().to_string(), ..food });
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn delete_my_food(state: tauri::State<Shared>, id: u64) -> Result<(), String> {
    mutate(&state, |st| {
        st.lib.my_foods.retain(|f| f.id != id);
        nutrition::refresh_estimates(&mut st.lib);
        Ok(())
    })
}

/// What a product link, barcode or label photo filled in; the page merges it into the open form.
#[derive(Serialize)]
pub(crate) struct Guess {
    food: MyFood,
    source: String,
    missing: Vec<&'static str>,
}

impl From<labels::LabelGuess> for Guess {
    fn from(g: labels::LabelGuess) -> Self {
        Guess { food: g.food, source: g.source, missing: g.missing }
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

#[derive(Serialize)]
pub(crate) struct Check {
    group: &'static str,
    name: String,
    passed: bool,
    detail: String,
}

/// The release checks (timers, step-by-step, video links, site rules, the ingredient reader,
/// data safety), run against the sample library so nothing of the person's is touched.
#[tauri::command]
pub(crate) async fn self_tests(state: tauri::State<'_, Shared>) -> Result<Vec<Check>, String> {
    let mut lib = model::sample_library(model::today());
    lib.rule_overrides = state.lock().unwrap().lib.rule_overrides.clone();
    tauri::async_runtime::spawn_blocking(move || selftest::run_all(&lib).into_iter().map(|r| Check { group: r.group, name: r.name, passed: r.passed, detail: r.detail }).collect())
        .await
        .map_err(|e| e.to_string())
}

/// Merge every duplicate recipe into its older copy (a Version History snapshot first).
#[tauri::command]
pub(crate) fn merge_duplicates(state: tauri::State<Shared>) -> Result<String, String> {
    mutate(&state, |st| {
        st.store.snapshot(&st.lib, "before merging duplicates")?;
        Ok(match importer::merge_all_duplicates(&mut st.lib) {
            0 => "No duplicates found".into(),
            1 => "Merged 1 duplicate recipe".into(),
            n => format!("Merged {n} duplicate recipes"),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damaged_library_is_never_backed_up() {
        let d = std::env::temp_dir().join(format!("rb-desktop-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("library.json"), "{\"recipes\": [").unwrap();
        let mut store = storage::Store::new(d.clone(), false);
        assert!(matches!(store.load(), storage::LoadOutcome::Damaged));
        let st = AppState { lib: Library::default(), store, undo: None, journal: recipe_box::tracker::journal::Journal::open(d.join("journal")), unsaved: false };
        assert_eq!(backup_parts(&st).err(), Some(storage::DAMAGED.to_string()));
        let ok = AppState { lib: Library::default(), store: storage::Store::new(d.clone(), false), undo: None, journal: recipe_box::tracker::journal::Journal::open(d.join("journal")), unsaved: false };
        assert!(backup_parts(&ok).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_backup_or_snapshot_is_refused_and_nothing_changes() {
        let d = std::env::temp_dir().join(format!("rb-desktop-newer-restore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        drop(crate::load_state(d.clone())); // first run writes the sample library
        std::fs::copy(d.join("library.json"), d.join("library.json.bak")).unwrap();
        let shared = std::sync::Mutex::new(crate::load_state(d.clone()));
        let (main, bak) = (std::fs::read(d.join("library.json")).unwrap(), std::fs::read(d.join("library.json.bak")).unwrap());
        let (snaps, recipes) = {
            let g = shared.lock().unwrap();
            (g.store.snapshots().len(), g.lib.recipes.len())
        };

        let mut newer = model::sample_library(model::today());
        newer.format = model::LIBRARY_FORMAT + 1;
        let p = storage::backup_to(&newer, &d.join("newer")).unwrap();
        assert!(storage::read_backup(&p).is_ok()); // still opens, so it stays in the list
        for _ in 0..2 {
            assert_eq!(restore_file(&shared, &p, false, &d.join("backups")), Err(storage::NEWER_BACKUP.to_string()));
        }
        let snap = d.join("history").join("snapshot-1.json");
        std::fs::create_dir_all(snap.parent().unwrap()).unwrap();
        std::fs::write(&snap, format!("{{\"taken\": 1, \"reason\": \"from a newer build\", \"library\": {{\"format\": {}}}}}", model::LIBRARY_FORMAT + 1)).unwrap();
        for _ in 0..2 {
            assert_eq!(rollback_to(&shared, &snap), Err(storage::NEWER_BACKUP.to_string()));
        }

        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), main);
        assert_eq!(std::fs::read(d.join("library.json.bak")).unwrap(), bak);
        let g = shared.lock().unwrap();
        assert_eq!(g.store.snapshots().len(), snaps + 1); // only the planted file
        assert!(g.store.snapshots().iter().all(|s| s.reason != "before restore" && s.reason != "before rollback"));
        assert_eq!(g.lib.recipes.len(), recipes);
        assert!(g.store.problem.is_none());
        drop(g);

        let ok = storage::backup_to(&model::sample_library(model::today()), &d.join("current")).unwrap();
        assert!(restore_file(&shared, &ok, false, &d.join("backups")).unwrap().starts_with("Restored"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn start_empty_only_unlocks_a_damaged_library() {
        let d = std::env::temp_dir().join(format!("rb-desktop-empty-ok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        drop(crate::load_state(d.clone()));
        let shared = std::sync::Mutex::new(crate::load_state(d.clone()));
        let main = std::fs::read(d.join("library.json")).unwrap();
        let recipes = shared.lock().unwrap().lib.recipes.len();
        assert!(recipes > 0);
        let err = empty_library(&shared).unwrap_err();
        assert!(err.contains("only for a library file that couldn't be read"), "{err}");
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), main);
        assert_eq!(shared.lock().unwrap().lib.recipes.len(), recipes);

        let d2 = std::env::temp_dir().join(format!("rb-desktop-empty-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d2);
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(d2.join("library.json"), "{\"recipes\": [").unwrap();
        let shared2 = std::sync::Mutex::new(crate::load_state(d2.clone()));
        assert_eq!(empty_library(&shared2), Ok("Started an empty library".to_string()));
        {
            let g = shared2.lock().unwrap();
            assert!(g.store.problem.is_none());
            assert!(g.lib.recipes.is_empty());
        }
        assert!(matches!(storage::Store::new(d2.clone(), true).load(), storage::LoadOutcome::Loaded { .. }));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d2);
    }

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
            st.journal.month_mut("2026-11")?.entries.push(sentinel.clone());
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
        assert_eq!(storage::read_backup(&full).unwrap().journal.unwrap().months.keys().collect::<Vec<_>>(), vec!["2026-11"]);
        let synced = std::fs::read_dir(d.join("sync")).unwrap().flatten().next().unwrap().path();
        let text = std::fs::read_to_string(&synced).unwrap();
        assert!(!text.contains("SENTINEL-DIARY-7731") && !text.contains("journal"));
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

    fn state_in(name: &str) -> (PathBuf, std::sync::Mutex<AppState>) {
        let d = std::env::temp_dir().join(format!("rb-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        drop(crate::load_state(d.clone())); // first run writes the sample library
        let shared = std::sync::Mutex::new(crate::load_state(d.clone()));
        (d, shared)
    }

    fn log(shared: &Shared, month: &str, food: &str) {
        let e = journal::DiaryEntry { day: format!("{month}-13"), name: food.into(), ..Default::default() };
        crate::mutate_journal(shared, |st| {
            st.journal.month_mut(month)?.entries.push(e);
            Ok(())
        })
        .unwrap();
    }

    fn bundle(months: &[(&str, &str)]) -> journal::JournalBundle {
        let mut b = journal::JournalBundle::default();
        for (m, food) in months {
            let e = journal::DiaryEntry { day: format!("{m}-13"), name: (*food).into(), ..Default::default() };
            b.months.insert((*m).into(), journal::JournalMonth { month: (*m).into(), entries: vec![e], ..Default::default() });
        }
        b
    }

    /// Every file in the journal folder (not the history folder) with its bytes.
    fn journal_files(d: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(d.join("journal"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_file())
            .map(|e| (e.file_name().to_string_lossy().to_string(), std::fs::read(e.path()).unwrap()))
            .collect();
        v.sort();
        v
    }

    fn restore_copies(d: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(d.join("journal").join("history")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("restore-")).collect()
    }

    #[test]
    fn old_backup_without_journal_restores_and_leaves_diary() {
        let (d, shared) = state_in("restore-old");
        log(&shared, "2026-11", "Soup");
        let before = journal_files(&d);
        let p = storage::backup_to(&model::sample_library(model::today()), &d.join("old")).unwrap();
        assert!(storage::read_backup(&p).unwrap().journal.is_none());
        let msg = restore_file(&shared, &p, true, &d.join("backups")).unwrap();
        assert!(msg.starts_with("Restored") && !msg.contains("diary"), "{msg}");
        assert_eq!(journal_files(&d), before);
        assert!(!d.join("backups").exists() && restore_copies(&d).is_empty());
        assert_eq!(restore_scope(None), Ok(false));
        assert_eq!(restore_scope(Some("everything")), Ok(true));
        assert_eq!(restore_scope(Some("all")), Err("Unknown restore choice.".to_string()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn old_named_backup_restores() {
        let (d, shared) = state_in("restore-old-name");
        let p = storage::backup_to(&model::sample_library(model::today()), &d.join("old")).unwrap();
        let old = p.with_file_name("Recipe Box backup 2026-01-01-10-00.recipebox");
        std::fs::rename(&p, &old).unwrap();
        let msg = restore_file(&shared, &old, false, &d.join("backups")).unwrap();
        assert!(msg.starts_with("Restored"), "{msg}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_recipes_only_leaves_the_diary() {
        let (d, shared) = state_in("restore-recipes");
        log(&shared, "2026-11", "Soup");
        let before = journal_files(&d);
        let p = storage::backup_with_photos(&model::sample_library(model::today()), &d.join("src"), None, Some(&bundle(&[("2026-11", "Cake")]))).unwrap();
        assert!(restore_file(&shared, &p, false, &d.join("backups")).unwrap().starts_with("Restored"));
        assert_eq!(journal_files(&d), before);
        assert!(!d.join("backups").exists() && restore_copies(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_everything_copies_journal_first() {
        let (d, shared) = state_in("restore-all");
        log(&shared, "2026-10", "A");
        log(&shared, "2026-11", "B");
        std::fs::write(d.join("journal").join("2026-09.json"), "{\"entries\": [").unwrap();
        let before: std::collections::BTreeMap<_, _> = journal_files(&d).into_iter().collect();
        let p = storage::backup_with_photos(&model::sample_library(model::today()), &d.join("src"), None, Some(&bundle(&[("2026-11", "C")]))).unwrap();
        let msg = restore_file(&shared, &p, true, &d.join("backups")).unwrap();
        assert!(msg.contains("food diary"), "{msg}");
        // the backup's month replaced the current one; the months it lacks were moved, not deleted
        let now: Vec<String> = journal_files(&d).into_iter().map(|(n, _)| n).filter(|n| n.ends_with(".json")).collect();
        assert_eq!(now, vec!["2026-11.json".to_string()]);
        let copy = restore_copies(&d);
        assert_eq!(copy.len(), 1);
        for (name, bytes) in &before {
            assert_eq!(&std::fs::read(copy[0].join(name)).unwrap(), bytes, "{name} kept byte for byte");
        }
        // a full backup of the old state is in the list: A and B, and not the unreadable month
        let saved: Vec<_> = std::fs::read_dir(d.join("backups")).unwrap().flatten().map(|e| storage::read_backup(&e.path()).unwrap()).collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].journal.as_ref().unwrap().months.keys().collect::<Vec<_>>(), vec!["2026-10", "2026-11"]);
        // the open diary reads the restored month
        assert_eq!(shared.lock().unwrap().journal.month("2026-11").unwrap().entries[0].name, "C");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_library_save_leaves_the_diary_alone() {
        let (d, shared) = state_in("restore-save-fails");
        log(&shared, "2026-11", "B");
        let before = journal_files(&d);
        let p = storage::backup_with_photos(&model::sample_library(model::today()), &d.join("src"), None, Some(&bundle(&[("2026-11", "C")]))).unwrap();
        std::fs::create_dir_all(d.join("library.tmp")).unwrap(); // the library's temp file can't be created
        assert!(restore_file(&shared, &p, true, &d.join("backups")).is_err());
        assert_eq!(journal_files(&d), before, "the diary was not replaced");
        assert!(restore_copies(&d).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bad_diary_in_backup_changes_nothing() {
        let (d, shared) = state_in("restore-bad");
        log(&shared, "2026-11", "B");
        let (lib_before, journal_before) = (std::fs::read(d.join("library.json")).unwrap(), journal_files(&d));
        let mut evil = bundle(&[("2026-11", "C")]);
        evil.months.insert("../../x".into(), journal::JournalMonth::default());
        let mut newer = bundle(&[("2026-11", "C")]);
        newer.months.get_mut("2026-11").unwrap().format = journal::JOURNAL_FORMAT + 1;
        let lib = model::sample_library(model::today());
        let p = storage::backup_with_photos(&lib, &d.join("evil"), None, Some(&evil)).unwrap();
        assert!(restore_file(&shared, &p, true, &d.join("backups")).unwrap_err().contains("bad name"));
        let q = storage::backup_with_photos(&lib, &d.join("newer"), None, Some(&newer)).unwrap();
        assert_eq!(restore_file(&shared, &q, true, &d.join("backups")), Err(storage::NEWER_BACKUP.to_string()));
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(journal_files(&d), journal_before);
        assert!(!d.join("backups").exists() && restore_copies(&d).is_empty() && !d.join("x.json").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rollback_never_touches_the_diary() {
        let (d, shared) = state_in("rollback-diary");
        log(&shared, "2026-11", "Soup");
        let before = journal_files(&d);
        let snap = {
            let mut g = shared.lock().unwrap();
            let st = &mut *g;
            st.store.snapshot(&st.lib, "test").unwrap();
            st.store.snapshots().into_iter().next().unwrap().path
        };
        assert_eq!(rollback_to(&shared, &snap), Ok("Rolled back".to_string()));
        assert_eq!(journal_files(&d), before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn picked_backup_reports_its_diary() {
        let d = std::env::temp_dir().join(format!("rb-desktop-picked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let lib = model::sample_library(model::today());
        let with = storage::backup_with_photos(&lib, &d.join("with"), None, Some(&bundle(&[("2026-11", "C")]))).unwrap();
        let without = storage::backup_to(&lib, &d.join("without")).unwrap();
        let row = picked_backup(&std::fs::read_to_string(&with).unwrap()).unwrap();
        assert!(row.has_diary && row.recipes == lib.recipes.len() && row.path.is_empty());
        assert!(!picked_backup(&std::fs::read_to_string(&without).unwrap()).unwrap().has_diary);
        assert_eq!(picked_backup("{}").err(), Some("This file isn't a Tiffin backup.".to_string()));
        assert!(!d.join("backups").exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
