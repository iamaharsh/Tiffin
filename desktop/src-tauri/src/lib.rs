//! Tiffin with a web interface. The window is Svelte (../src); everything it shows comes
//! from the `recipe_box` crate through the commands below.

mod foods_online;
mod importing;
mod settings;
mod update;

use recipe_box::model;
use recipe_box::{cooking, inbox, ingredients, selftest, shortcut, storage};
use tauri::Manager;
use std::path::PathBuf;
use std::sync::Mutex;

// The state, its save rules and the shared commands live in tiffin-commands (also built for the web).
pub(crate) use tiffin_commands::{mutate, web_address, AppState, Shared};
#[cfg(test)]
pub(crate) use tiffin_commands::replace_library;
#[cfg(test)] // only the tests change the diary directly now; its commands are shared
pub(crate) use tiffin_commands::mutate_journal;

/// The picture's bytes arrive as the raw request body (not a JSON number array); the `stem`
/// header names the file. Returns the new file name in the photos folder.
#[tauri::command]
async fn add_photo(state: tauri::State<'_, Shared>, request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("No picture was sent.".into());
    };
    let stem = request.headers().get("stem").and_then(|v| v.to_str().ok()).unwrap_or("photo");
    tiffin_commands::recipes::add_photo_bytes(&state, bytes, stem)
}

/// Whether Mica went on behind the window in `setup` (it can't on older Windows).
struct MicaOn(bool);

/// Problems the launch clean-up could not fix (a folder that did not move, the old app still installed);
/// the page asks for them once and shows each as an error toast.
pub(crate) struct LaunchNotes(pub(crate) std::sync::Arc<Mutex<Vec<String>>>);

/// Hand over the launch problems, once.
#[tauri::command]
fn launch_notes(state: tauri::State<LaunchNotes>) -> Result<Vec<String>, String> {
    Ok(std::mem::take(&mut *state.0.lock().unwrap()))
}

/// Tells the page whether Mica is on, so it can let the sidebar show it. The backdrop itself is applied in `setup`, before the window is shown.
#[tauri::command]
fn mica(on: tauri::State<MicaOn>) -> bool {
    on.0
}

/// The page calls this after its first paint. It is a Rust command so the capabilities stay `core:default`.
#[tauri::command]
fn show_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())
}

/// Open a web address in the browser. Only http(s), because this hands the text to the system opener.
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    let u = url.trim();
    if !web_address(u) {
        return Err("That isn't a web address.".into());
    }
    cooking::open_external(u).then_some(()).ok_or_else(|| "Couldn't open the browser.".into())
}

/// Photo file names are made by storage::photo_name: letters, digits, '-' and one ".jpg".
fn safe_photo_name(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
}

fn load_state(data_dir: PathBuf) -> AppState {
    let state = tiffin_commands::load_state(data_dir);
    remove_instance_file();
    state
}

/// The classic window kept a `instance-<user>.txt` (port and token of its listener) in the data
/// folder. Nothing reads it any more, so a leftover one goes; nothing else in the folder is touched.
fn remove_instance_file() {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let safe: String = user.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let _ = std::fs::remove_file(storage::data_dir().join(format!("instance-{}.txt", safe)));
}

/// Command-line arguments that aren't flags (or a flag's value), after the program name.
fn opened_with(args: &[String]) -> Vec<&String> {
    let mut out = vec![];
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        if a == "--data-dir" {
            it.next();
        } else if !a.starts_with("--") {
            out.push(a);
        }
    }
    out
}

fn send_opened(args: &[String]) {
    if let [one] = opened_with(args)[..] {
        let _ = inbox::send_to_inbox(one);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let args: Vec<String> = std::env::args().collect();
    // `--self-test`: run the release checks on the built-in sample library; exit code 1 if any fail.
    if args.iter().any(|a| a == "--self-test") {
        let results = selftest::run_all(&model::sample_library(model::today()));
        let failed = results.iter().filter(|r| !r.passed).count();
        for r in &results {
            println!("[{}] {} / {} {}", if r.passed { "PASS" } else { "FAIL" }, r.group, r.name, r.detail);
        }
        println!("{} of {} checks passed", results.len() - failed, results.len());
        if failed > 0 {
            std::process::exit(1);
        }
        return;
    }
    // `--import <file|link>`: drop it in the inbox for the open window (or the next start) and quit.
    if let Some(i) = args.iter().position(|a| a == "--import") {
        match args.get(i + 1).map(|a| inbox::send_to_inbox(a)) {
            Some(Ok(p)) => println!("Sent to the Tiffin inbox: {}", p.display()),
            Some(Err(e)) => {
                eprintln!("Couldn't send: {e}");
                std::process::exit(1);
            }
            None => {
                eprintln!("Usage: --import <file or link>");
                std::process::exit(1);
            }
        }
        return;
    }
    let data_dir = args.iter().position(|a| a == "--data-dir").and_then(|i| args.get(i + 1)).map(PathBuf::from).unwrap_or_else(storage::data_dir);
    // Opened with one recipe file or link (e.g. "Open with"): it goes to the inbox like the classic app.
    send_opened(&args);
    // dev builds and test copies never move the owner's folders or remove an installed app
    let notes = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
    if !cfg!(debug_assertions) && !args.iter().any(|a| a == "--data-dir") {
        // the inbox moves before the window opens, so the inbox poll never races it
        if let Err(m) = storage::move_old_folder("Inbox") {
            notes.lock().unwrap().push(m);
        }
        // the README is refreshed only once the old inbox is gone, so a retried move never meets a different copy
        if inbox::inbox_dir().join(inbox::README_NAME).exists() && !inbox::old_inbox_dir().exists() {
            let _ = inbox::setup_inbox();
        }
        // PowerShell and the uninstaller are slow, so they never hold up the window
        let notes = notes.clone();
        std::thread::spawn(move || {
            let mut problems: Vec<String> = [storage::move_old_folder("Backups"), storage::move_old_folder("Tracker Exports"), shortcut::remove_old_recipe_box()].into_iter().filter_map(Result::err).collect();
            shortcut::remove_old_shortcuts();
            notes.lock().unwrap().append(&mut problems);
        });
    }
    ingredients::init_decimal_separator();
    // build the food index now, so the first keystroke on the Foods page doesn't pay for it
    std::thread::spawn(recipe_box::foods::warm);
    let photos = storage::photos_dir(&data_dir);
    tauri::Builder::default()
        // One window per user: a second launch hands over what it was opened with and brings this one forward.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            send_opened(&argv);
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(LaunchNotes(notes))
        .manage(tiffin_commands::foods::OffState::new(&data_dir))
        .manage(Mutex::new(load_state(data_dir)))
        // photo://<file name> serves pictures from the library's photos folder, and nothing else.
        .register_uri_scheme_protocol("photo", move |_ctx, req| {
            let name = req.uri().path().trim_start_matches('/').to_string();
            let bytes = if safe_photo_name(&name) { std::fs::read(photos.join(&name)).ok() } else { None };
            let res = tauri::http::Response::builder();
            match bytes {
                Some(b) => res.header("Content-Type", if name.ends_with(".png") { "image/png" } else { "image/jpeg" }).header("Cache-Control", "max-age=31536000").body(b),
                None => res.status(404).body(vec![]),
            }
            .unwrap()
        })
        .setup(|app| {
            let w = app.get_webview_window("main");
            // Mica goes on while the window is still hidden, so it never shows see-through (G-02-11).
            #[cfg(windows)]
            let on = w.as_ref().is_some_and(|w| window_vibrancy::apply_mica(w, None).is_ok());
            #[cfg(not(windows))]
            let on = false;
            app.manage(MicaOn(on));
            if let Some(w) = w {
                // a page error must never leave the window hidden
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    let _ = w.show();
                });
            }
            Ok(())
        })
        .invoke_handler({
            // one handler: the shared commands by name, everything else from this shell
            type Invoke = tauri::ipc::Invoke<tauri::Wry>;
            // gives the generated handler its argument type, which a plain `let` can't infer
            fn typed(f: impl Fn(Invoke) -> bool + Send + Sync + 'static) -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
                f
            }
            let shared = tiffin_commands::tauri_handler::<tauri::Wry>();
            let local = typed(tauri::generate_handler![
            foods_online::off_search,
            foods_online::barcode_photo,
            add_photo,
            open_link,
            mica,
            launch_notes,
            show_window,
            update::check_update,
            update::install_update,
            update::set_unsaved,
            importing::import_url,
            importing::save_review,
            importing::take_inbox,
            importing::rules_overview,
            importing::rules_from_sync,
            settings::settings_info,
            settings::sync_now,
            settings::auto_sync,
            settings::backup_now,
            settings::delete_health_data,
            settings::send_all_to_tracker,
            settings::setup_inbox,
            settings::open_folder,
            settings::label_lookup,
            settings::label_photo,
            settings::self_tests,
            ]);
            move |invoke: Invoke| if tiffin_commands::NAMES.contains(&invoke.message.command()) { shared(invoke) } else { local(invoke) }
        })
        .run(tauri::generate_context!())
        .expect("Tiffin couldn't start");
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiffin_commands::recipes::{in_category, overview_of, step_pieces};
    use recipe_box::model::{Library, MealType, Nutrients};
    use recipe_box::{nutrition, tracker};
    use tiffin_commands::diary::{log_planned_in, planned_for, Pick};
    use recipe_box::tracker::diary;

    fn with_my_food(d: &std::path::Path) -> Shared {
        let shared = Mutex::new(load_state(d.to_path_buf()));
        shared.lock().unwrap().lib.my_foods.push(model::MyFood { id: 7, name: "Vanilla whey".into(), serving: "1 scoop".into(), serving_g: 31.0, per_serving: Nutrients { calories: Some(120.0), ..Default::default() }, ..Default::default() });
        shared
    }

    fn lib_bytes(d: &std::path::Path) -> Vec<u8> {
        std::fs::read(d.join("library.json")).unwrap()
    }

    /// The planner's Today (what MealDialog stores as Meal.day) is the diary's local day, not the UTC day (CR-01).
    #[test]
    fn planner_today_is_the_diary_local_day() {
        // (label, UTC y-m-d, hour, minute, offset minutes, local day, month file)
        let cases = [
            ("behind", (2026, 11, 14), 4, 30, -300, "2026-11-13", "2026-11"),
            ("ahead", (2026, 11, 13), 20, 0, 330, "2026-11-14", "2026-11"),
            ("year-end", (2027, 1, 1), 3, 0, -300, "2026-12-31", "2026-12"),
        ];
        for (label, (y, m, dd), h, min, offset, want_day, want_month) in cases {
            let d = tdir(&format!("planner-day-{label}"));
            let shared = with_my_food(&d);
            let utc = model::days_from_civil(y, m, dd) as i64 * 86400 + h * 3600 + min * 60;
            let local = recipe_box::tracker::day::local_day(utc, offset);
            assert_eq!(local.as_str(), want_day, "{label}: local day");
            let planner = overview_of(&shared.lock().unwrap(), local.as_str()).unwrap().today;
            assert_eq!(planner, diary::day_number(want_day).unwrap(), "{label}: planner Today is the diary's day number");
            assert_ne!(planner, utc.div_euclid(86400) as i32, "{label}: the UTC day differs from the local day");
            {
                let mut st = shared.lock().unwrap();
                let a = st.lib.recipes.iter().find(|r| r.nutrition.is_some_and(|n| n.calories.is_some()) && !r.uid.is_empty()).cloned().unwrap();
                st.lib.meals = vec![model::Meal { id: 201, day: planner, kind: MealType::Lunch, recipe_id: Some(a.id), ..Default::default() }];
            }
            let listed = planned_for(&shared, local.as_str()).unwrap();
            assert_eq!(listed.iter().map(|p| (p.meal_id, p.loggable)).collect::<Vec<_>>(), [(201, true)], "{label}: listed");
            let r = log_planned_in(&shared, local.as_str(), &[Pick { meal_id: 201, servings: 1.0 }]).unwrap();
            assert_eq!((r.count, r.view.day.as_str(), r.view.slots[1].entries.len()), (1, want_day, 1), "{label}: logged into the local day");
            assert_eq!(shared.lock().unwrap().journal.month(want_month).unwrap().entries.len(), 1, "{label}: month {want_month}");
            if label == "year-end" {
                assert!(!d.join("journal").join("2027-01.json").exists(), "year-end: no 2027-01 month file");
            }
            let _ = std::fs::remove_dir_all(&d);
        }
        let d = tdir("planner-day-invalid");
        let shared = with_my_food(&d);
        let err = overview_of(&shared.lock().unwrap(), "2026-02-30").err().unwrap();
        assert!(err.contains("isn't a valid date"), "an invalid day is refused, never the UTC day: {err}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A Meal.day saved before the fix keeps its number and its calendar date: nothing migrates it (data safety).
    #[test]
    fn stored_meal_days_keep_their_calendar_date() {
        let d = tdir("planner-stored");
        let shared = with_my_food(&d);
        let lib_before = lib_bytes(&d);
        let utc = model::days_from_civil(2026, 11, 14) as i64 * 86400 + 4 * 3600 + 30 * 60;
        let old_today = utc.div_euclid(86400) as i32; // what the UTC "Today" stored at local 2026-11-13 23:30
        shared.lock().unwrap().lib.meals = vec![model::Meal { id: 301, day: old_today, kind: MealType::Lunch, name: "Soup".into(), ..Default::default() }];
        let planner = overview_of(&shared.lock().unwrap(), "2026-11-13").unwrap().today;
        assert_ne!(planner, old_today);
        assert_eq!(shared.lock().unwrap().lib.meals[0].day, old_today, "the stored day is untouched");
        assert!(planned_for(&shared, "2026-11-13").unwrap().is_empty());
        assert_eq!(planned_for(&shared, "2026-11-14").unwrap().iter().map(|p| p.meal_id).collect::<Vec<_>>(), [301]);
        assert_eq!(lib_bytes(&d), lib_before, "reading the planner and the diary writes nothing");
        let _ = std::fs::remove_dir_all(&d);
    }


    /// Command names `invoke` calls in a screen file: `invoke('name'` or `invoke<Type>('name'`.
    fn invoked(src: &str) -> Vec<String> {
        src.split("invoke").skip(1).filter_map(|rest| {
            let at = rest.find("('")?;
            let between = &rest[..at];
            let typed = between.is_empty() || (between.starts_with('<') && between.ends_with('>') && !between.contains('\n'));
            typed.then(|| rest[at + 2..].split('\'').next().unwrap_or_default().to_string())
        }).collect()
    }

    /// Every command the window calls is answered by exactly one handler: the shared list
    /// (tiffin-commands) or this shell's own. A command moved out of one and not into the other
    /// would only show up as "not found" in the window.
    #[test]
    fn every_command_the_window_calls_has_one_handler() {
        let me = include_str!("lib.rs");
        let start = me.find("let local = typed(tauri::generate_handler![").unwrap();
        let block = &me[start..start + me[start..].find("]);").unwrap()];
        let local: Vec<&str> = block.lines().skip(1).map(|l| l.trim().trim_end_matches(',')).filter(|l| !l.is_empty() && !l.starts_with("//")).map(|l| l.rsplit("::").next().unwrap()).collect();
        // read for real, to its end: show_window needs the desktop window, so it never leaves the shell's
        // list, and right after the list's close comes the routing closure, so the read didn't stop early
        assert!(local.contains(&"show_window") && local.contains(&"add_photo"), "the shell's list was read: {local:?}");
        assert!(me[start + block.len()..].trim_start_matches("]);").trim_start().starts_with("move |invoke"), "the shell's list was read to its end");
        for n in &local {
            assert!(!tiffin_commands::NAMES.contains(n), "{n} is in both handlers");
        }
        let called: Vec<String> = [include_str!("../../src/lib/api.ts"), include_str!("../../src/main.ts")].iter().flat_map(|f| invoked(f)).collect();
        assert!(called.len() > 100, "the screens' calls were read: {}", called.len());
        for c in &called {
            assert!(local.contains(&c.as_str()) || tiffin_commands::NAMES.contains(&c.as_str()), "the window calls {c}, which no handler answers");
        }
    }

    /// A fresh temp folder of its own for one test.
    fn tdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rb-desktop-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn damaged_copies(d: &std::path::Path) -> Vec<PathBuf> {
        std::fs::read_dir(d).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("library-damaged-")).collect()
    }

    #[test]
    fn damaged_library_is_never_replaced_by_samples() {
        let d = tdir("nosamples");
        let lib = d.join("library.json");
        std::fs::write(&lib, "{\"recipes\": [").unwrap();
        let st = load_state(d.clone());
        assert!(st.lib.recipes.is_empty());
        assert!(matches!(st.store.problem, Some(storage::Problem::Damaged { .. })));
        let shared = Mutex::new(st);
        assert_eq!(mutate(&shared, |_| Ok(())), Err(storage::DAMAGED.to_string()));
        assert_eq!(std::fs::read(&lib).unwrap(), b"{\"recipes\": [");
        assert!(!d.join("library.json.bak").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recovery_unlocks_a_damaged_library() {
        let d = tdir("unlock");
        std::fs::write(d.join("library.json"), "{\"recipes\": [").unwrap();
        let shared = Mutex::new(load_state(d.clone()));
        replace_library(&shared, |st| {
            st.lib = Library::default();
            Ok(())
        })
        .unwrap();
        assert!(shared.lock().unwrap().store.problem.is_none());
        assert!(matches!(storage::Store::new(d.clone(), true).load(), storage::LoadOutcome::Loaded { .. }));
        let c = damaged_copies(&d);
        assert_eq!(c.len(), 1);
        assert_eq!(std::fs::read(&c[0]).unwrap(), b"{\"recipes\": [");
        assert!(!d.join("library.json.bak").exists(), "the damaged file never reaches .bak");
        mutate(&shared, |st| {
            st.lib.settings.recipe_sort = 1;
            Ok(())
        })
        .unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn journal_write_lands_in_its_month_file_and_leaves_the_library() {
        use recipe_box::model::Nutrients;
        use tracker::journal::{journal_dir, DiaryEntry, Journal};
        let d = tdir("journal");
        drop(load_state(d.clone()));
        let shared = Mutex::new(load_state(d.clone()));
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();
        // 2026-02-01 04:30 UTC is 23:30 on 31 January at UTC-5
        let utc = model::days_from_civil(2026, 2, 1) as i64 * 86400 + 4 * 3600 + 30 * 60;
        let day = tracker::day::local_day(utc, -300);
        mutate_journal(&shared, |st| {
            let e = DiaryEntry { day: day.as_str().into(), name: "Oat porridge".into(), basis: Nutrients { calories: Some(150.0), ..Default::default() }, ..Default::default() };
            st.journal.month_mut(day.month())?.entries.push(e);
            Ok(())
        })
        .unwrap();
        assert!(d.join("journal").join("2026-01.json").exists());
        assert!(!d.join("journal").join("2026-02.json").exists());
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        let mut fresh = Journal::open(journal_dir(&d));
        let back = fresh.month("2026-01").unwrap();
        assert_eq!((back.entries.len(), back.entries[0].name.as_str(), back.entries[0].basis.sugar_g), (1, "Oat porridge", None));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_library_refuses_every_change() {
        let d = tdir("newer");
        let file = d.join("library.json");
        std::fs::write(&file, format!(r#"{{"format": {}, "app_version": "9.0.0", "future": 1, "recipes": [{{"id": 1, "name": "Soup A"}}, {{"id": 2, "name": "Soup B"}}]}}"#, model::LIBRARY_FORMAT + 1)).unwrap();
        let before = std::fs::read(&file).unwrap();
        let st = load_state(d.clone());
        assert!(matches!(st.store.problem, Some(storage::Problem::TooNew { .. })));
        assert_eq!(st.lib.recipes.len(), 2);
        assert!(st.store.problem.as_ref().unwrap().banner().contains("9.0.0"));
        assert_eq!(settings::backup_parts(&st).map(|_| ()), Err(storage::READ_ONLY.to_string()));
        let shared = Mutex::new(st);
        assert_eq!(mutate(&shared, |_| Ok(())), Err(storage::READ_ONLY.to_string()));
        assert_eq!(replace_library(&shared, |_| Ok(())), Err(storage::READ_ONLY.to_string()));
        assert_eq!(std::fs::read(&file).unwrap(), before);
        assert!(!d.join("library.json.bak").exists() && !d.join("history").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn older_backup_is_saved_in_this_format() {
        let d = tdir("oldformat");
        let shared = Mutex::new(load_state(d.clone()));
        replace_library(&shared, |st| {
            st.lib = model::sample_library(model::today());
            st.lib.format = 1;
            Ok(())
        })
        .unwrap();
        let text = std::fs::read_to_string(d.join("library.json")).unwrap();
        assert!(text.contains(&format!("\"format\": {}", model::LIBRARY_FORMAT)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_library_gets_the_samples() {
        let d = tdir("firstrun");
        let st = load_state(d.clone());
        assert!(!st.lib.recipes.is_empty());
        assert!(st.store.problem.is_none());
        assert!(d.join("library.json").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mutate_after_outside_damage_keeps_the_bak() {
        let d = tdir("outside");
        drop(load_state(d.clone())); // first run writes the sample library
        std::fs::copy(d.join("library.json"), d.join("library.json.bak")).unwrap();
        let shared = Mutex::new(load_state(d.clone()));
        let bak = std::fs::read(d.join("library.json.bak")).unwrap();
        std::fs::write(d.join("library.json"), "{\"recipes\": [").unwrap();
        mutate(&shared, |st| {
            st.lib.settings.recipe_sort = 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(std::fs::read(d.join("library.json.bak")).unwrap(), bak);
        let c = damaged_copies(&d);
        assert_eq!(c.len(), 1);
        assert_eq!(std::fs::read(&c[0]).unwrap(), b"{\"recipes\": [");
        assert!(matches!(storage::Store::new(d.clone(), true).load(), storage::LoadOutcome::Loaded { .. }));
        assert_eq!(shared.lock().unwrap().store.take_notice(), Some(storage::Notice::Replaced { kept: c[0].clone() }));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn estimates_are_refreshed_once_on_start() {
        let d = tdir("estimates");
        let mut lib = model::sample_library(model::today());
        lib.recipes.truncate(1);
        let r = &mut lib.recipes[0];
        r.ingredients = "1 cup flour
2 eggs".into();
        r.nutrition = None;
        r.nutrition_edited = false;
        r.modified = 1234;
        lib.nutrition_estimator = 0;
        storage::Store::new(d.clone(), false).save(&lib).unwrap();
        let st = load_state(d.clone());
        let r = &st.lib.recipes[0];
        assert!(r.nutrition.as_ref().is_some_and(|n| n.calories.is_some_and(|c| c > 0.0)));
        assert_eq!(st.lib.nutrition_estimator, nutrition::ESTIMATOR_VERSION);
        assert_eq!(r.modified, 1234, "modified is left alone");
        // counted by reason: the test's own first save also snapshots, and same-second snapshots no longer overwrite each other
        let before = |s: &AppState| s.store.snapshots().iter().filter(|x| x.reason == "before updating nutrition estimates").count();
        assert_eq!(before(&st), 1, "one snapshot before the update");
        // second start: nothing to redo, no second snapshot
        let again = load_state(d.clone());
        assert_eq!(again.lib.nutrition_estimator, nutrition::ESTIMATOR_VERSION);
        assert_eq!(before(&again), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn old_library_nutrition_loads_and_hand_typed_stays() {
        let d = tdir("old-plain-nutrition");
        let mut lib = model::sample_library(model::today());
        lib.recipes.truncate(2);
        let (est_id, typed_id) = (lib.recipes[0].id, lib.recipes[1].id);
        lib.recipes[0].ingredients = "200 g chicken breast
2 tbsp butter".into();
        lib.recipes[0].servings = "1".into();
        lib.recipes[0].nutrition_edited = false;
        lib.recipes[1].nutrition_edited = true;
        // the shape of an old file: format 1, estimator 2, seven plain numbers per recipe
        let plain = |v: [f64; 7]| {
            let k = ["calories", "protein_g", "fat_g", "carbs_g", "fiber_g", "sugar_g", "sodium_mg"];
            serde_json::Value::Object(k.iter().zip(v).map(|(k, v)| (k.to_string(), serde_json::json!(v))).collect())
        };
        let mut v = serde_json::to_value(&lib).unwrap();
        v["format"] = 1.into();
        v["nutrition_estimator"] = 2.into();
        v["recipes"][0]["nutrition"] = plain([1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        v["recipes"][1]["nutrition"] = plain([300.0, 20.0, 10.0, 30.0, 0.0, 4.0, 500.0]);
        std::fs::write(d.join("library.json"), serde_json::to_string(&v).unwrap()).unwrap();
        let st = load_state(d.clone());
        let by_id = |lib: &Library, id: u64| lib.recipes.iter().find(|r| r.id == id).unwrap().nutrition.unwrap();
        let typed = by_id(&st.lib, typed_id);
        assert_eq!(
            (typed.calories, typed.protein_g, typed.fat_g, typed.carbs_g, typed.fiber_g, typed.sugar_g, typed.sodium_mg),
            (Some(300.0), Some(20.0), Some(10.0), Some(30.0), Some(0.0), Some(4.0), Some(500.0)),
            "hand-typed nutrition is never recomputed or reinterpreted"
        );
        let est = by_id(&st.lib, est_id);
        assert_eq!((est.fiber_g, est.sugar_g), (None, None));
        assert!(est.calories.is_some_and(|c| c > 1.0));
        assert_eq!(st.lib.format, model::LIBRARY_FORMAT);
        assert_eq!(st.lib.nutrition_estimator, nutrition::ESTIMATOR_VERSION);
        assert!(st.store.snapshots().iter().any(|s| s.reason == "before updating nutrition estimates"));
        drop(st);
        let disk: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d.join("library.json")).unwrap()).unwrap();
        let on_disk = |id: u64| disk["recipes"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap()["nutrition"].clone();
        assert!(on_disk(est_id).get("fiber_g").is_none(), "an unknown value is left out of the file");
        assert_eq!(on_disk(typed_id)["fiber_g"], 0.0, "a stored 0 stays 0 on disk");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn failed_snapshot_keeps_the_old_estimates() {
        let d = tdir("nosnapshot-estimates");
        let mut lib = model::sample_library(model::today());
        lib.app_version = model::APP_VERSION.into();
        lib.format = model::LIBRARY_FORMAT;
        lib.nutrition_estimator = nutrition::ESTIMATOR_VERSION - 1;
        let old = Nutrients { calories: Some(1.0), protein_g: Some(0.0), fat_g: Some(0.0), carbs_g: Some(0.0), fiber_g: Some(0.0), sugar_g: Some(0.0), sodium_mg: Some(0.0) };
        lib.recipes[0].nutrition_edited = false;
        lib.recipes[0].nutrition = Some(old);
        storage::Store::new(d.clone(), false).save(&lib).unwrap();
        let _ = std::fs::remove_dir_all(d.join("history"));
        std::fs::write(d.join("history"), "a file where the history folder should be").unwrap();
        // no snapshot, no rewrite: in memory and on disk the old estimate and estimator number stay
        let st = load_state(d.clone());
        assert_eq!(st.lib.nutrition_estimator, nutrition::ESTIMATOR_VERSION - 1);
        assert_eq!(st.lib.recipes[0].nutrition.as_ref(), Some(&old));
        let storage::LoadOutcome::Loaded { lib: on_disk } = storage::Store::new(d.clone(), true).load() else { panic!("expected Loaded") };
        assert_eq!(on_disk.nutrition_estimator, nutrition::ESTIMATOR_VERSION - 1);
        assert_eq!(on_disk.recipes[0].nutrition.as_ref(), Some(&old));
        // the next start with a writable history folder redoes them, after its snapshot
        drop(st);
        std::fs::remove_file(d.join("history")).unwrap();
        let st = load_state(d.clone());
        assert_eq!(st.lib.nutrition_estimator, nutrition::ESTIMATOR_VERSION);
        assert_ne!(st.lib.recipes[0].nutrition.as_ref(), Some(&old));
        assert!(st.store.snapshots().iter().any(|s| s.reason == "before updating nutrition estimates"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn opened_with_skips_flags_and_the_data_dir() {
        let a: Vec<String> = ["app.exe", "--data-dir", "D:/lib", "soup.json"].map(String::from).into();
        assert_eq!(super::opened_with(&a), [&"soup.json".to_string()]);
        assert!(super::opened_with(&a[..3]).is_empty());
    }

    #[test]
    fn photo_names_cannot_leave_the_folder() {
        assert!(super::safe_photo_name("carrot-cake-1730000000.jpg"));
        for bad in ["", "../library.json", "..\\x.jpg", "a/b.jpg", "C:x.jpg", "%2e%2e"] {
            assert!(!super::safe_photo_name(bad), "{bad}");
        }
    }

    #[test]
    fn step_pieces_mark_times_and_keep_all_text() {
        let p = step_pieces("Simmer for 10 minutes, then rest 1 hour.");
        let joined: String = p.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(joined, "Simmer for 10 minutes, then rest 1 hour.");
        let t: Vec<_> = p.iter().filter_map(|x| x.seconds.map(|s| (x.text.as_str(), s))).collect();
        assert_eq!(t, [("10 minutes", 600), ("1 hour", 3600)]);
    }

    #[test]
    fn only_web_addresses_open() {
        assert!(super::web_address("https://example.com/a?b=1"));
        for bad in ["file:///C:/Windows/notepad.exe", "C:\\x.exe", "calc", "https://a.com\" b", "javascript:alert(1)"] {
            assert!(!super::web_address(bad), "{bad}");
        }
    }

    #[test]
    fn category_match_takes_children_not_lookalikes() {
        let r = recipe_box::model::Recipe { categories: vec!["Desserts/Cakes".into()], ..Default::default() };
        assert!(in_category(&r, "Desserts"));
        assert!(in_category(&r, "Desserts/Cakes"));
        assert!(!in_category(&r, "Dessert"));
    }
}
