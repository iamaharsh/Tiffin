//! Settings commands shared by the desktop window and the web version: sidebar order, launch
//! page, preferences, backup/snapshot listing, snapshots, restore, rollback, damaged-library empty
//! start, My Foods, and duplicate merging. Desktop-only settings commands (sync folders, inbox,
//! Explorer, native backups, tracker export, online label lookup/OCR, self-tests) stay in the shell.

use crate::{mutate, replace_library, replace_library_then, AppState, Shared};
use recipe_box::model::{self, Library, MyFood};
use recipe_box::tracker::journal;
use recipe_box::{credits, health, importer, nutrition, rules, selftest, storage};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Desktop-only fields of [`Info`] (cloud sync suggestions and the local Claude inbox); the web
/// version passes `DesktopInfo::default()`.
#[derive(Default)]
pub struct DesktopInfo {
    pub onedrive: Option<String>,
    pub dropbox: Option<String>,
    pub inbox_dir: String,
    pub inbox_ready: bool,
    pub instructions: String,
}

#[derive(Serialize)]
pub struct Info {
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
    /// the data sources credited on the About screen
    sources: Vec<credits::Source>,
}

pub fn settings_info_in(state: &Shared, desktop: DesktopInfo) -> Info {
    let s = state.lock().unwrap();
    let set = &s.lib.settings;
    Info {
        version: model::APP_VERSION,
        sync_folder: set.sync_folder.clone(),
        auto_sync: set.auto_sync,
        device_name: set.device_name.clone(),
        last_sync: set.last_sync.map(storage::fmt_stamp),
        onedrive: desktop.onedrive,
        dropbox: desktop.dropbox,
        backups_dir: storage::backups_dir(&s.store.dir).display().to_string(),
        tracker_service: set.tracker_service.clone(),
        tracker_folder: set.tracker_folder.clone(),
        tracker_default: health::default_folder(&set.tracker_service).display().to_string(),
        tracker_auto_send: set.tracker_auto_send,
        off_email: set.off_email.clone(),
        services: health::SERVICES,
        service_url: health::service_url(&set.tracker_service),
        inbox_ready: desktop.inbox_ready,
        inbox_dir: desktop.inbox_dir,
        instructions: desktop.instructions,
        recipes: s.lib.recipes.iter().filter(|r| !r.in_trash).count(),
        favorites: s.lib.recipes.iter().filter(|r| r.favorite && !r.in_trash).count(),
        sources: credits::data_sources(),
    }
}

/// Delete all health data: the journal folder and the diary in every backup. The lock is held
/// throughout, so no diary change can write a cached month back.
pub fn delete_health_data_in(state: &Shared) -> Result<String, String> {
    let mut st = state.lock().unwrap();
    let dir = journal::journal_dir(&st.store.dir);
    let out = journal::delete_all(&dir, &storage::backup_dirs(&st.store.dir, &sync_dirs(&st.lib)));
    // a fresh journal either way: it reads what is left on disk and drops the undo
    st.journal = journal::Journal::open(dir);
    out
}

#[derive(Serialize)]
pub struct Check {
    group: &'static str,
    name: String,
    passed: bool,
    detail: String,
}

/// The release checks (timers, step-by-step, video links, site rules, the ingredient reader,
/// data safety), run against the sample library so nothing of the person's is touched.
pub fn run_self_tests(rule_overrides: Vec<rules::SiteRule>) -> Vec<Check> {
    let mut lib = model::sample_library(model::today());
    lib.rule_overrides = rule_overrides;
    selftest::run_all(&lib).into_iter().map(|r| Check { group: r.group, name: r.name, passed: r.passed, detail: r.detail }).collect()
}

pub fn self_tests_in(state: &Shared) -> Vec<Check> {
    let overrides = state.lock().unwrap().lib.rule_overrides.clone();
    run_self_tests(overrides)
}

/// One settings change; only the fields sent are touched.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Change {
    sync_folder: Option<String>,
    auto_sync: Option<bool>,
    device_name: Option<String>,
    tracker_service: Option<String>,
    tracker_folder: Option<String>,
    tracker_auto_send: Option<bool>,
}

/// Store the sidebar's tab order; an id that isn't a tab refuses the whole change.
pub fn set_sidebar_order(state: &Shared, order: Vec<String>) -> Result<(), String> {
    mutate(state, |st| recipe_box::sidebar::set_order(&mut st.lib.settings, &order))
}

/// Make a tab the one Tiffin opens on.
pub fn set_launch_page(state: &Shared, page: String) -> Result<(), String> {
    mutate(state, |st| recipe_box::sidebar::set_launch_page(&mut st.lib.settings, &page))
}

pub fn set_settings(state: &Shared, change: Change) -> Result<(), String> {
    mutate(state, |st| {
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

#[derive(Serialize)]
pub struct Saved {
    path: String,
    when: String,
    recipes: usize,
    /// snapshots only
    reason: String,
    favorites: usize,
    /// backups only: the file has a food diary section
    has_diary: bool,
}

pub fn sync_dirs(lib: &Library) -> Vec<PathBuf> {
    Some(&lib.settings.sync_folder).filter(|s| !s.is_empty()).map(PathBuf::from).into_iter().collect()
}

/// The backup list, or with `picked` (the text of a file the person chose) the one row for that file.
/// Whether a backup has a diary is decided here, so the page never reads backup files itself.
pub fn backups(state: &Shared, picked: Option<String>) -> Result<Vec<Saved>, String> {
    if let Some(text) = picked {
        return picked_backup(&text).map(|r| vec![r]);
    }
    let (data, extra) = { let s = state.lock().unwrap(); (s.store.dir.clone(), sync_dirs(&s.lib)) };
    Ok(storage::list_backups(&data, &extra)
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

pub fn snapshots(state: &Shared) -> Result<Vec<Saved>, String> {
    let s = state.lock().unwrap();
    Ok(s.store
        .snapshots()
        .into_iter()
        .take(40)
        .map(|x| Saved { path: x.path.display().to_string(), when: storage::fmt_stamp(x.taken), recipes: x.recipes, reason: x.reason, favorites: x.favorites, has_diary: false })
        .collect())
}

pub fn take_snapshot(state: &Shared) -> Result<(), String> {
    let mut g = state.lock().unwrap();
    let st = &mut *g;
    st.store.check_unlocked()?;
    st.store.snapshot(&st.lib, "taken by you")
}

/// What a backup is made from: the library, the photos folder and the journal folder. Refuses while
/// the library is locked, so a backup of the empty placeholder of a damaged library can never become
/// the newest backup.
pub fn backup_parts(st: &AppState) -> Result<(Library, PathBuf, PathBuf), String> {
    st.store.check_unlocked()?;
    Ok((st.lib.clone(), storage::photos_dir(&st.store.dir), journal::journal_dir(&st.store.dir)))
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
pub fn restore_backup(state: &Shared, path: Option<String>, text: Option<String>, scope: Option<String>) -> Result<String, String> {
    let everything = restore_scope(scope.as_deref())?;
    let b = match (path, text) {
        (Some(p), _) => {
            let p = PathBuf::from(p);
            let known = { let s = state.lock().unwrap(); storage::list_backups(&s.store.dir, &sync_dirs(&s.lib)) }.into_iter().any(|(b, _, _, _)| b == p);
            if !known {
                return Err("That backup isn't in the list any more.".into());
            }
            storage::read_backup(&p)?
        }
        (None, Some(t)) => storage::parse_backup(&t)?,
        _ => return Err("Choose a backup first.".into()),
    };
    let backups = storage::backups_dir(&state.lock().unwrap().store.dir);
    restore_parsed(state, b, everything, &backups)
}

/// The restore itself, split from the command so tests and both input modes share one path. A library
/// or diary from a newer build, or a diary month with a bad name, is refused before anything changes.
/// With `everything` and a diary in the backup, the current diary is first backed up into `backups`
/// and copied inside the journal folder; a backup without a diary is recipes only, whatever was asked.
fn restore_parsed(state: &Shared, b: storage::BackupFile, everything: bool, backups: &Path) -> Result<String, String> {
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
pub fn rollback(state: &Shared, path: String) -> Result<String, String> {
    rollback_to(state, &PathBuf::from(&path))
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
pub fn start_empty(state: &Shared) -> Result<String, String> {
    empty_library(state)
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

// ---- My Foods ----

pub fn my_foods(state: &Shared) -> Result<Vec<MyFood>, String> {
    Ok(state.lock().unwrap().lib.my_foods.clone())
}

/// Add a food or save an edit (a name another food has is refused), then re-estimate every recipe not typed in by hand.
pub fn save_my_food(state: &Shared, food: MyFood) -> Result<(), String> {
    // the crate decides what blocks a save (the form shows the same answer), so a page can't slip an invalid food in
    recipe_box::foods::check_food(&food).map_err(|p| p.message.to_string())?;
    mutate(state, |st| {
        nutrition::check_my_food_name(&st.lib, &food)?; // checked inside mutate, after the reload, so it sees the current list
        nutrition::save_my_food(&mut st.lib, MyFood { name: food.name.trim().to_string(), ..food });
        Ok(())
    })
}

pub fn delete_my_food(state: &Shared, id: u64) -> Result<(), String> {
    mutate(state, |st| {
        st.lib.my_foods.retain(|f| f.id != id);
        nutrition::refresh_estimates(&mut st.lib);
        Ok(())
    })
}

/// Merge every duplicate recipe into its older copy (a Version History snapshot first).
pub fn merge_duplicates(state: &Shared) -> Result<String, String> {
    mutate(state, |st| {
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
    use crate::{dispatch, foods::OffState, load_state, Services};
    use recipe_box::model::Nutrients;
    use serde_json::json;
    use std::sync::Mutex;

    fn restore_file(state: &Shared, file: &Path, everything: bool, backups: &Path) -> Result<String, String> {
        restore_parsed(state, storage::read_backup(file)?, everything, backups)
    }

    fn restore_copies(d: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(d.join("journal").join("history")).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("restore-")).collect()
    }

    fn state_in(name: &str) -> (PathBuf, Mutex<AppState>) {
        let d = crate::scratch_dir().join(format!("tiffin-commands-settings-{name}-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        drop(load_state(d.clone())); // first run writes the sample library
        let shared = Mutex::new(load_state(d.clone()));
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

    #[test]
    fn start_empty_only_unlocks_a_damaged_library() {
        let d = crate::scratch_dir().join(format!("tiffin-commands-empty-ok-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        drop(load_state(d.clone()));
        let shared = Mutex::new(load_state(d.clone()));
        let main = std::fs::read(d.join("library.json")).unwrap();
        let recipes = shared.lock().unwrap().lib.recipes.len();
        assert!(recipes > 0);
        let err = empty_library(&shared).unwrap_err();
        assert!(err.contains("only for a library file that couldn't be read"), "{err}");
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), main);
        assert_eq!(shared.lock().unwrap().lib.recipes.len(), recipes);

        let d2 = crate::scratch_dir().join(format!("tiffin-commands-empty-bad-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&d2);
        std::fs::create_dir_all(&d2).unwrap();
        std::fs::write(d2.join("library.json"), "{\"recipes\": [").unwrap();
        let shared2 = Mutex::new(load_state(d2.clone()));
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
        let d = crate::scratch_dir().join(format!("tiffin-commands-picked-{}", crate::test_pid()));
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

    #[test]
    fn settings_commands_dispatch_with_json() {
        let (d, state) = state_in("dispatch");
        let off = OffState::new(&d);
        let app = Services { state, off };

        // set_sidebar_order & set_launch_page
        let mut order: Vec<String> = recipe_box::sidebar::TABS.iter().map(|s| (*s).to_string()).collect();
        order.swap(0, 1);
        dispatch(&app, "set_sidebar_order", json!({ "order": order })).unwrap();
        assert_eq!(app.state.lock().unwrap().lib.settings.sidebar_order, order);
        assert!(dispatch(&app, "set_sidebar_order", json!({ "order": ["not_a_tab"] })).is_err());

        dispatch(&app, "set_launch_page", json!({ "page": "recipes" })).unwrap();
        assert_eq!(app.state.lock().unwrap().lib.settings.launch_page, "recipes");
        assert!(dispatch(&app, "set_launch_page", json!({ "page": "nope" })).is_err());

        // set_settings (partial changes across all fields: trims text, accepts valid service, ignores unknown)
        dispatch(
            &app,
            "set_settings",
            json!({ "change": { "sync_folder": "  /sync  ", "auto_sync": false, "device_name": "  Kitchen  ", "tracker_service": "MyFitnessPal", "tracker_folder": "  /exports  ", "tracker_auto_send": true } }),
        )
        .unwrap();
        dispatch(&app, "set_settings", json!({ "change": { "tracker_service": "UnknownApp", "sync_folder": "" } })).unwrap();
        {
            let set = &app.state.lock().unwrap().lib.settings;
            assert_eq!(set.sync_folder, "");
            assert!(!set.auto_sync);
            assert_eq!(set.device_name, "Kitchen");
            assert_eq!(set.tracker_service, "MyFitnessPal");
            assert_eq!(set.tracker_folder, "/exports");
            assert!(set.tracker_auto_send);
        }

        // take_snapshot, snapshots, rollback
        let orig_name = app.state.lock().unwrap().lib.recipes[0].name.clone();
        dispatch(&app, "take_snapshot", serde_json::Value::Null).unwrap();
        let snaps = dispatch(&app, "snapshots", serde_json::Value::Null).unwrap();
        let snap = snaps.as_array().unwrap().iter().find(|s| s["reason"] == "taken by you").unwrap();
        let snap_path = snap["path"].as_str().unwrap().to_string();
        assert!(!snap["when"].as_str().unwrap().is_empty());
        assert!(snap["favorites"].is_u64());
        assert_eq!(snap["has_diary"], false);
        assert!(snap["recipes"].as_u64().unwrap() > 0);

        app.state.lock().unwrap().lib.recipes[0].name = "Changed Name".into();
        assert_eq!(dispatch(&app, "rollback", json!({ "path": snap_path })).unwrap(), "Rolled back");
        assert_eq!(app.state.lock().unwrap().lib.recipes[0].name, orig_name);
        assert_eq!(dispatch(&app, "rollback", json!({ "path": "no-such-snapshot.json" })).unwrap_err(), "That version isn't in Version History any more.");

        // backups (list without args via null payload, picked backup text, invalid picked text)
        let lib_copy = app.state.lock().unwrap().lib.clone();
        let saved_backup = storage::backup_to(&lib_copy, &storage::backups_dir(&d)).unwrap();
        let listed = dispatch(&app, "backups", serde_json::Value::Null).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["path"], saved_backup.display().to_string());
        assert!(!listed[0]["when"].as_str().unwrap().is_empty());
        assert_eq!(listed[0]["favorites"], 0);
        assert_eq!(listed[0]["has_diary"], false);

        let with_diary = storage::backup_with_photos(&lib_copy, &d.join("pick"), None, Some(&bundle(&[("2026-11", "Oats")]))).unwrap();
        let picked = dispatch(&app, "backups", json!({ "picked": std::fs::read_to_string(&with_diary).unwrap() })).unwrap();
        assert_eq!(picked.as_array().unwrap().len(), 1);
        assert_eq!(picked[0]["path"], "");
        assert!(!picked[0]["when"].as_str().unwrap().is_empty());
        assert_eq!(picked[0]["has_diary"], true);
        assert_eq!(dispatch(&app, "backups", json!({ "picked": "{}" })).unwrap_err(), "This file isn't a Tiffin backup.");

        // my_foods, save_my_food (validation + duplicate-name refusal), delete_my_food
        assert_eq!(dispatch(&app, "my_foods", serde_json::Value::Null).unwrap(), json!([]));
        let bad_food = MyFood { name: "No Cals".into(), ..Default::default() };
        assert!(dispatch(&app, "save_my_food", json!({ "food": bad_food })).is_err());
        let good_food = MyFood {
            name: "  Oat Milk  ".into(),
            serving: "1 cup".into(),
            serving_g: 240.0,
            per_serving: Nutrients { calories: Some(120.0), ..Default::default() },
            ..Default::default()
        };
        dispatch(&app, "save_my_food", json!({ "food": good_food })).unwrap();
        let dup_food = MyFood { name: "oat milk".into(), ..good_food.clone() };
        assert!(dispatch(&app, "save_my_food", json!({ "food": dup_food })).is_err(), "duplicate My Food name is refused");
        let foods = dispatch(&app, "my_foods", serde_json::Value::Null).unwrap();
        assert_eq!(foods.as_array().unwrap().len(), 1);
        assert_eq!(foods[0]["name"], "Oat Milk");
        let food_id = foods[0]["id"].as_u64().unwrap();
        dispatch(&app, "delete_my_food", json!({ "id": food_id })).unwrap();
        assert_eq!(dispatch(&app, "my_foods", serde_json::Value::Null).unwrap(), json!([]));

        // merge_duplicates
        assert_eq!(dispatch(&app, "merge_duplicates", serde_json::Value::Null).unwrap(), "No duplicates found");
        let before_count = {
            let mut st = app.state.lock().unwrap();
            let dup = st.lib.recipes[0].clone();
            st.lib.add_recipe(dup);
            st.lib.recipes.len()
        };
        assert_eq!(dispatch(&app, "merge_duplicates", serde_json::Value::Null).unwrap(), "Merged 1 duplicate recipe");
        assert_eq!(app.state.lock().unwrap().lib.recipes.len(), before_count - 1);

        // start_empty (refused on healthy library, unlocks damaged library; damaged library also refuses take_snapshot)
        assert!(dispatch(&app, "start_empty", serde_json::Value::Null).unwrap_err().contains("only for a library file that couldn't be read"));
        let d_bad = crate::scratch_dir().join(format!("tiffin-commands-settings-bad-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&d_bad);
        std::fs::create_dir_all(&d_bad).unwrap();
        std::fs::write(d_bad.join("library.json"), "{\"recipes\": [").unwrap();
        let bad_app = Services { state: Mutex::new(load_state(d_bad.clone())), off: OffState::new(&d_bad) };
        assert!(bad_app.state.lock().unwrap().store.problem.is_some());
        assert_eq!(dispatch(&bad_app, "take_snapshot", serde_json::Value::Null).unwrap_err(), storage::DAMAGED);
        assert_eq!(dispatch(&bad_app, "start_empty", serde_json::Value::Null).unwrap(), "Started an empty library");
        {
            let st = bad_app.state.lock().unwrap();
            assert!(st.store.problem.is_none(), "start_empty unlocked the damaged library");
            assert!(st.lib.recipes.is_empty());
        }

        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&d_bad);
    }

    #[test]
    fn damaged_library_is_never_backed_up() {
        let d = crate::scratch_dir().join(format!("tiffin-commands-backup-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("library.json"), "{\"recipes\": [").unwrap();
        let mut store = storage::Store::new(d.clone(), false);
        assert!(matches!(store.load(), storage::LoadOutcome::Damaged));
        let st = AppState { lib: Library::default(), store, undo: None, journal: journal::Journal::open(d.join("journal")), unsaved: false };
        assert_eq!(backup_parts(&st).err(), Some(storage::DAMAGED.to_string()));
        let ok = AppState { lib: Library::default(), store: storage::Store::new(d.clone(), false), undo: None, journal: journal::Journal::open(d.join("journal")), unsaved: false };
        assert!(backup_parts(&ok).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_backup_or_snapshot_is_refused_and_nothing_changes() {
        let d = crate::scratch_dir().join(format!("tiffin-commands-newer-restore-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        drop(load_state(d.clone())); // first run writes the sample library
        std::fs::copy(d.join("library.json"), d.join("library.json.bak")).unwrap();
        let shared = Mutex::new(load_state(d.clone()));
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
        // the current profile, so the copy folder has an old profile.json to keep
        crate::mutate_journal(&shared, |st| {
            st.journal.profile_mut()?.height_cm = Some(170.0);
            Ok(())
        })
        .unwrap();
        let before: std::collections::BTreeMap<_, _> = journal_files(&d).into_iter().collect();
        assert!(before.contains_key("profile.json"));
        // the backup's month also holds a workout and a measurement, and its profile one target
        let mut restored = bundle(&[("2026-11", "C")]);
        let month = restored.months.get_mut("2026-11").unwrap();
        month.exercise.push(journal::ExerciseEntry { uid: "x1".into(), day: "2026-11-13".into(), name: "Walk".into(), minutes: 30.0, est_kcal: Some(90.0), ..Default::default() });
        month.body.push(journal::BodyEntry { uid: "b1".into(), day: "2026-11-13".into(), measure: "weight".into(), value: 80.0, ..Default::default() });
        restored.profile = Some(journal::Profile { targets: vec![recipe_box::tracker::goals::TargetSet { effective_from: "2026-11-13".into(), kcal: 1900, ..Default::default() }], ..Default::default() });
        let p = storage::backup_with_photos(&model::sample_library(model::today()), &d.join("src"), None, Some(&restored)).unwrap();
        let msg = restore_file(&shared, &p, true, &d.join("backups")).unwrap();
        assert!(msg.contains("food diary"), "{msg}");
        // the backup's month replaced the current one; the months it lacks were moved, not deleted
        let now: Vec<String> = journal_files(&d).into_iter().map(|(n, _)| n).filter(|n| n.ends_with(".json")).collect();
        assert_eq!(now, vec!["2026-11.json".to_string(), "profile.json".to_string()]);
        let copy = restore_copies(&d);
        assert_eq!(copy.len(), 1);
        for (name, bytes) in &before {
            assert_eq!(&std::fs::read(copy[0].join(name)).unwrap(), bytes, "{name} kept byte for byte");
        }
        // a full backup of the old state is in the list: A and B, and not the unreadable month
        let saved: Vec<_> = std::fs::read_dir(d.join("backups")).unwrap().flatten().map(|e| storage::read_backup(&e.path()).unwrap()).collect();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].journal.as_ref().unwrap().months.keys().collect::<Vec<_>>(), vec!["2026-10", "2026-11"]);
        assert_eq!(saved[0].journal.as_ref().unwrap().profile.as_ref().and_then(|p| p.height_cm), Some(170.0), "the old profile is in that backup too");
        // the open diary reads the restored month, with its workout, measurement and target
        let mut g = shared.lock().unwrap();
        let m = g.journal.month("2026-11").unwrap().clone();
        assert_eq!((m.entries[0].name.as_str(), m.exercise.len(), m.body.len()), ("C", 1, 1));
        assert_eq!((m.exercise[0].name.as_str(), m.body[0].value), ("Walk", 80.0));
        let targets = g.journal.profile().unwrap().targets.clone();
        assert_eq!(targets.iter().map(|t| t.kcal).collect::<Vec<_>>(), [1900]);
        assert_eq!(g.journal.profile().unwrap().height_cm, None, "the backup's profile replaced the current one");
        drop(g);
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
        let mut newer_profile = bundle(&[("2026-11", "C")]);
        newer_profile.profile = Some(journal::Profile { format: journal::PROFILE_FORMAT + 1, ..Default::default() });
        let lib = model::sample_library(model::today());
        let p = storage::backup_with_photos(&lib, &d.join("evil"), None, Some(&evil)).unwrap();
        assert!(restore_file(&shared, &p, true, &d.join("backups")).unwrap_err().contains("bad name"));
        let q = storage::backup_with_photos(&lib, &d.join("newer"), None, Some(&newer)).unwrap();
        assert_eq!(restore_file(&shared, &q, true, &d.join("backups")), Err(storage::NEWER_BACKUP.to_string()));
        let r = storage::backup_with_photos(&lib, &d.join("newer-profile"), None, Some(&newer_profile)).unwrap();
        assert_eq!(restore_file(&shared, &r, true, &d.join("backups")), Err(storage::NEWER_BACKUP.to_string()), "a newer profile is refused too");
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(journal_files(&d), journal_before);
        assert!(!d.join("backups").exists() && restore_copies(&d).is_empty() && !d.join("x.json").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn restore_backup_dispatches_from_text_and_path() {
        let (d, state) = state_in("restore-dispatch");
        log(&state, "2026-11", "Old Food");
        let app = Services { state, off: OffState::new(&d) };

        // error paths
        assert_eq!(dispatch(&app, "restore_backup", serde_json::Value::Null).unwrap_err(), "Choose a backup first.");
        assert_eq!(dispatch(&app, "restore_backup", json!({ "scope": "all" })).unwrap_err(), "Unknown restore choice.");
        assert_eq!(
            dispatch(&app, "restore_backup", json!({ "path": d.join("unknown.recipebox").display().to_string() })).unwrap_err(),
            "That backup isn't in the list any more."
        );
        assert_eq!(dispatch(&app, "restore_backup", json!({ "text": "{}" })).unwrap_err(), "This file isn't a Tiffin backup.");

        // recipes-only restore from raw backup text (no temp file; works under WASI)
        let mut lib_a = Library::default();
        lib_a.add_recipe(model::Recipe { name: "From Text".into(), ..Default::default() });
        let text_a = std::fs::read_to_string(storage::backup_with_photos(&lib_a, &d.join("src-a"), None, Some(&bundle(&[("2026-11", "Skipped Meal")]))).unwrap()).unwrap();
        let msg = dispatch(&app, "restore_backup", json!({ "path": null, "text": text_a, "scope": "recipes" })).unwrap();
        assert_eq!(msg, "Restored 1 recipes");
        {
            let mut st = app.state.lock().unwrap();
            assert_eq!(st.lib.recipes.len(), 1);
            assert_eq!(st.lib.recipes[0].name, "From Text");
            assert_eq!(st.journal.month("2026-11").unwrap().entries[0].name, "Old Food");
        }

        // full restore (recipes + diary) from raw backup text
        let mut lib_b = Library::default();
        lib_b.add_recipe(model::Recipe { name: "Full Restore".into(), ..Default::default() });
        let text_b = std::fs::read_to_string(storage::backup_with_photos(&lib_b, &d.join("src-b"), None, Some(&bundle(&[("2026-11", "Restored Meal")]))).unwrap()).unwrap();
        let msg = dispatch(&app, "restore_backup", json!({ "path": null, "text": text_b, "scope": "everything" })).unwrap();
        assert!(msg.as_str().unwrap().starts_with("Restored 1 recipes and your food diary."), "{msg}");
        {
            let mut st = app.state.lock().unwrap();
            assert_eq!(st.lib.recipes.len(), 1);
            assert_eq!(st.lib.recipes[0].name, "Full Restore");
            assert_eq!(st.journal.month("2026-11").unwrap().entries[0].name, "Restored Meal");
        }

        // path restore from the safety backup that the full restore just placed in Backups (omitted text & scope -> recipes only)
        let listed = dispatch(&app, "backups", serde_json::Value::Null).unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        let safety_path = listed[0]["path"].as_str().unwrap().to_string();
        assert_eq!(dispatch(&app, "restore_backup", json!({ "path": safety_path })).unwrap(), "Restored 1 recipes");
        {
            let mut st = app.state.lock().unwrap();
            assert_eq!(st.lib.recipes[0].name, "From Text");
            assert_eq!(st.journal.month("2026-11").unwrap().entries[0].name, "Restored Meal");
        }

        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn settings_info_delete_health_and_self_tests_work_portably() {
        let (d, shared) = state_in("portable-cores");
        log(&shared, "2026-11", "Porridge");
        let (lib, photos, jdir) = backup_parts(&shared.lock().unwrap()).unwrap();
        let backup_path = storage::backup_with_photos(&lib, &storage::backups_dir(&d), Some(&photos), journal::bundle_from_disk(&jdir).0.as_ref()).unwrap();
        assert!(storage::read_backup(&backup_path).unwrap().journal.is_some());

        // settings_info_in with web defaults (empty DesktopInfo) and desktop overrides
        let web_info = serde_json::to_value(settings_info_in(&shared, DesktopInfo::default())).unwrap();
        assert_eq!(web_info["version"], model::APP_VERSION);
        assert_eq!(web_info["onedrive"], serde_json::Value::Null);
        assert_eq!(web_info["inbox_ready"], false);
        assert_eq!(web_info["inbox_dir"], "");
        assert!(web_info["recipes"].as_u64().unwrap() > 0);
        assert!(!web_info["sources"].as_array().unwrap().is_empty());

        let desk_info = serde_json::to_value(settings_info_in(
            &shared,
            DesktopInfo { onedrive: Some("C:/OneDrive/Tiffin".into()), inbox_dir: "C:/Inbox".into(), inbox_ready: true, instructions: "drop here".into(), dropbox: None },
        ))
        .unwrap();
        assert_eq!(desk_info["onedrive"], "C:/OneDrive/Tiffin");
        assert_eq!(desk_info["inbox_dir"], "C:/Inbox");
        assert_eq!(desk_info["inbox_ready"], true);

        // delete_health_data_in wipes the journal folder, strips diary from backups, and keeps recipes
        let recipes_before = shared.lock().unwrap().lib.recipes.len();
        let msg = delete_health_data_in(&shared).unwrap();
        assert!(!msg.is_empty());
        assert!(journal_files(&d).is_empty(), "journal folder emptied");
        assert!(shared.lock().unwrap().journal.month("2026-11").unwrap().entries.is_empty(), "reopened journal is empty");
        let backup_after = storage::read_backup(&backup_path).unwrap();
        assert!(backup_after.journal.is_none(), "diary stripped from backup");
        assert_eq!(backup_after.library.recipes.len(), recipes_before);

        // self_tests_in runs every check against the sample library and passes
        let checks = self_tests_in(&shared);
        assert!(!checks.is_empty());
        assert!(checks.iter().all(|c| c.passed), "all self tests pass: {:?}", checks.iter().filter(|c| !c.passed).map(|c| (&c.name, &c.detail)).collect::<Vec<_>>());
        let first_json = serde_json::to_value(&checks[0]).unwrap();
        assert!(first_json["group"].is_string() && first_json["name"].is_string() && first_json["passed"] == true);

        let _ = std::fs::remove_dir_all(&d);
    }
}
