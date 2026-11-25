//! The commands Tiffin's screens call, shared by the desktop window (Tauri, `tauri` feature) and
//! the web version (WebAssembly, through [`dispatch`]). Each command is a plain function taking the
//! shared state; the [`commands!`] list at the bottom names each one once for both.

pub mod body;
pub mod diary;
pub mod exercise;
pub mod foods;
pub mod goals;
pub mod grocery;
pub mod home;
pub mod importing;
pub mod meals;
pub mod menus;
pub mod pantry;
pub mod recipes;
pub mod reports;
pub mod settings;

use recipe_box::model::{self, GroceryList, Library, MealType, PantryItem};
use recipe_box::{nutrition, storage, tracker};
use std::path::PathBuf;
use std::sync::Mutex;

/// Everything the commands work on: the library, its store, and the diary.
pub struct AppState {
    pub lib: Library,
    pub store: storage::Store,
    /// Grocery lists and pantry as they were before the last removal, for Undo.
    pub undo: Option<(Vec<GroceryList>, Vec<PantryItem>)>,
    /// Diary month files and the profile, in the journal folder beside library.json.
    pub journal: tracker::journal::Journal,
    /// A page holds typed input that is not saved yet (set by the page through set_unsaved); an update install waits for it.
    pub unsaved: bool,
}

pub type Shared = Mutex<AppState>;

/// What the web version's dispatcher runs commands against: the state, and the Open Food Facts
/// search memory (kept apart from the library lock, as on desktop).
pub struct Services {
    pub state: Shared,
    pub off: foods::OffState,
}

pub const GONE: &str = "That recipe isn't in the library any more.";

pub fn web_address(u: &str) -> bool {
    (u.starts_with("https://") || u.starts_with("http://")) && !u.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"')
}

/// Open the library and the diary in `data_dir`: a first run gets the sample library, older
/// libraries get cuisines, label colours and newer nutrition estimates, and a damaged or newer
/// library stays as it is on disk.
pub fn load_state(data_dir: PathBuf) -> AppState {
    let journal = tracker::journal::Journal::open(tracker::journal::journal_dir(&data_dir));
    let mut store = storage::Store::new(data_dir, false);
    // Samples only for a true first run. A damaged file gives an empty placeholder in memory and no save at all.
    let (mut lib, first_run) = match store.load() {
        storage::LoadOutcome::Loaded { lib } => (lib, false),
        storage::LoadOutcome::Missing => (model::sample_library(model::today()), true),
        storage::LoadOutcome::Damaged => (Library::default(), false),
        // A newer library stays in memory so browsing, cook mode and timers work; the store refuses every save.
        storage::LoadOutcome::TooNew { lib } => (lib, false),
    };
    let cuisines = lib.ensure_cuisines();
    let labels = model::assign_label_colors(&mut lib);
    nutrition::set_my_foods(&lib.my_foods);
    // Newer nutrition estimator: redo the saved estimates once (never ones typed in by hand), with a
    // snapshot first so the old numbers can be brought back. `modified` stays as it is, so this device
    // does not win every later sync conflict.
    let mut estimates = false;
    if store.problem.is_none() && lib.nutrition_estimator < nutrition::ESTIMATOR_VERSION {
        // no snapshot, no rewrite: the old estimates stay and the next start tries again
        if first_run || store.snapshot(&lib, "before updating nutrition estimates").is_ok() {
            for r in lib.recipes.iter_mut().filter(|r| !r.nutrition_edited) {
                r.nutrition = Some(nutrition::calculate(r).total);
            }
            lib.nutrition_estimator = nutrition::ESTIMATOR_VERSION;
            estimates = true;
        }
    }
    if (first_run || cuisines || labels || estimates) && store.problem.is_none() {
        let _ = store.save(&lib);
    }
    AppState { lib, store, undo: None, journal, unsaved: false }
}

/// Change the library and save. This app is the only writer; a change is refused only while the
/// library is damaged (DAMAGED) or from a newer version (READ_ONLY).
pub fn mutate<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
    // Any other change ends the grocery/pantry Undo, so Undo can't throw that change away.
    mutate_keeping_undo(state, |st| {
        let out = f(st)?;
        st.undo = None;
        Ok(out)
    })
}

/// Swap the whole library (restore a backup or a saved version, start empty). The only way out of
/// the damaged state: it refuses unless the damaged file has a verified copy, runs `f` while the
/// store is still read-only (so no "before restore" snapshot of the empty placeholder is taken),
/// then unlocks and saves. The damaged file is overwritten only because its copy exists, and it
/// never reaches `.bak` (the save gate sees the copied stamp).
pub fn replace_library<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
    replace_library_then(state, f, |_| Ok(()))
}

/// [`replace_library`] plus a step that runs only after the library file was saved, under the same lock
/// (a restore replaces the diary here, so a failed library save can't leave the diary replaced and the library not).
pub fn replace_library_then<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>, after: impl FnOnce(&mut AppState) -> Result<(), String>) -> Result<T, String> {
    // Not through mutate on purpose: mutate refuses while the library is damaged and this is the
    // only way out of that state. It keeps mutate's single lock and gated save.
    let mut guard = state.lock().unwrap();
    let st = &mut *guard;
    st.store.can_replace()?;
    let out = f(st)?;
    st.lib.format = model::LIBRARY_FORMAT; // a restored or rolled-back older library is saved in this build's format, so an older build opens it read-only (newer ones were refused by ensure_not_newer)
    st.store.clear_damage();
    st.undo = None;
    st.store.save(&st.lib)?;
    after(st)?;
    Ok(out)
}

/// [`mutate`] for the removals that set up Undo, and for Undo itself.
pub fn mutate_keeping_undo<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
    let mut guard = state.lock().unwrap();
    let st = &mut *guard;
    st.store.check_unlocked()?;
    let out = f(st)?;
    st.store.save(&st.lib)?;
    Ok(out)
}

/// Change the diary or profile and save the journal files that changed. It never touches
/// library.json (no save, no library snapshot), and it doesn't check the library's lock: a damaged
/// or newer library must not block the diary. Each journal file guards itself. When `f` or the save
/// fails, the unsaved change is dropped from memory too (and the pending undo goes back), so what the
/// next read shows is what is on disk.
pub fn mutate_journal<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
    let mut guard = state.lock().unwrap();
    let st = &mut *guard;
    let out = f(st).and_then(|v| st.journal.save_dirty().map(|_| v));
    if out.is_err() {
        st.journal.discard_unsaved();
    }
    out
}

/// Names each command once, by what it works on: `state { module::name(arg: Type, …) -> Ok; }` for
/// `module::name(&Shared, arg, …) -> Result<Ok, String>`, `off` for `(&OffState, …)`, `state_off` for
/// `(&Shared, &OffState, …)` and `pure` for `(…)`. Gives [`NAMES`], the web's [`dispatch`] (arguments
/// by their camelCase names, as Tauri sends them), and with the `tauri` feature the desktop's Tauri
/// commands and [`tauri_handler`].
macro_rules! commands {
    (
        state { $( $sm:ident :: $sn:ident ( $( $sa:ident : $st:ty ),* $(,)? ) -> $so:ty ; )* }
        off { $( $om:ident :: $on:ident ( $( $oa:ident : $ot:ty ),* $(,)? ) -> $oo:ty ; )* }
        state_off { $( $bm:ident :: $bn:ident ( $( $ba:ident : $bt:ty ),* $(,)? ) -> $bo:ty ; )* }
        pure { $( $pm:ident :: $pn:ident ( $( $pa:ident : $pt:ty ),* $(,)? ) -> $po:ty ; )* }
    ) => {
        /// Every command in the shared list, by the name the screens call it.
        pub const NAMES: &[&str] = &[$( stringify!($sn), )* $( stringify!($on), )* $( stringify!($bn), )* $( stringify!($pn), )*];

        /// Run one command by name with its arguments as JSON, and answer as JSON: the web
        /// version's stand-in for Tauri's invoke.
        pub fn dispatch(app: &Services, name: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
            let args = if args.is_null() { serde_json::json!({}) } else { args };
            let bad = |e: serde_json::Error| format!("Bad arguments for {name}: {e}");
            let reply = |v: Result<serde_json::Value, serde_json::Error>| v.map_err(|e| e.to_string());
            match name {
                $( stringify!($sn) => {
                    #[derive(serde::Deserialize)]
                    #[serde(rename_all = "camelCase")]
                    struct Args { $( $sa: $st ),* }
                    let Args { $( $sa ),* } = serde_json::from_value(args).map_err(bad)?;
                    reply(serde_json::to_value($sm::$sn(&app.state, $( $sa ),*)?))
                } )*
                $( stringify!($on) => {
                    #[derive(serde::Deserialize)]
                    #[serde(rename_all = "camelCase")]
                    struct Args { $( $oa: $ot ),* }
                    let Args { $( $oa ),* } = serde_json::from_value(args).map_err(bad)?;
                    reply(serde_json::to_value($om::$on(&app.off, $( $oa ),*)?))
                } )*
                $( stringify!($bn) => {
                    #[derive(serde::Deserialize)]
                    #[serde(rename_all = "camelCase")]
                    struct Args { $( $ba: $bt ),* }
                    let Args { $( $ba ),* } = serde_json::from_value(args).map_err(bad)?;
                    reply(serde_json::to_value($bm::$bn(&app.state, &app.off, $( $ba ),*)?))
                } )*
                $( stringify!($pn) => {
                    #[derive(serde::Deserialize)]
                    #[serde(rename_all = "camelCase")]
                    struct Args { $( $pa: $pt ),* }
                    let Args { $( $pa ),* } = serde_json::from_value(args).map_err(bad)?;
                    reply(serde_json::to_value($pm::$pn($( $pa ),*)?))
                } )*
                _ => Err(format!("Unknown command {name}")),
            }
        }

        #[cfg(feature = "tauri")]
        mod tauri_commands {
            use super::*;
            // private on purpose: a public #[tauri::command] exports its helper macros at the crate root
            $(
                #[tauri::command]
                fn $sn(state: tauri::State<'_, Shared>, $( $sa: $st ),*) -> Result<$so, String> {
                    $sm::$sn(&state, $( $sa ),*)
                }
            )*
            $(
                #[tauri::command]
                fn $on(off: tauri::State<'_, foods::OffState>, $( $oa: $ot ),*) -> Result<$oo, String> {
                    $om::$on(&off, $( $oa ),*)
                }
            )*
            $(
                #[tauri::command]
                fn $bn(state: tauri::State<'_, Shared>, off: tauri::State<'_, foods::OffState>, $( $ba: $bt ),*) -> Result<$bo, String> {
                    $bm::$bn(&state, &off, $( $ba ),*)
                }
            )*
            $(
                #[tauri::command]
                fn $pn($( $pa: $pt ),*) -> Result<$po, String> {
                    $pm::$pn($( $pa ),*)
                }
            )*

            pub fn handler<R: tauri::Runtime>() -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static {
                tauri::generate_handler![$( $sn, )* $( $on, )* $( $bn, )* $( $pn, )*]
            }
        }

        /// The desktop's Tauri handler for every command in the shared list.
        #[cfg(feature = "tauri")]
        pub use tauri_commands::handler as tauri_handler;
    };
}

commands! {
    state {
        meals::meals(from: i32, to: i32) -> Vec<meals::MealOut>;
        meals::update_meal(id: u64, day: Option<i32>, kind: Option<MealType>) -> ();
        meals::remove_meals(ids: Vec<u64>, from: Option<i32>, to: Option<i32>) -> Vec<model::Meal>;
        meals::restore_meals(meals: Vec<model::Meal>) -> ();
        menus::menus() -> Vec<menus::MenuOut>;
        menus::new_menu() -> u64;
        menus::save_menu(id: u64, name: String, days: u32, entries: Vec<model::MenuEntry>) -> ();
        menus::delete_menu(id: u64) -> model::Menu;
        menus::restore_menu(menu: model::Menu) -> ();
        menus::apply_menu(id: u64, start: i32) -> usize;
        reports::report_view(range: tracker::body::BodyRange, today: String) -> tracker::reports::Report;
        grocery::groceries(list: u64) -> grocery::Groceries;
        grocery::grocery_preview(id: u64, times: u32, skip: bool) -> grocery::Preview;
        grocery::add_recipe_to_list(id: u64, list: u64, times: u32, skip: bool) -> String;
        grocery::plan_preview(from: i32, to: i32, skip: bool) -> grocery::Preview;
        grocery::add_plan_to_list(list: u64, from: i32, to: i32, skip: bool) -> String;
        grocery::add_item(list: u64, text: String) -> ();
        grocery::update_item(list: u64, id: u64, change: grocery::ItemChange) -> ();
        grocery::remove_item(list: u64, id: u64) -> Option<String>;
        grocery::new_list(name: String) -> u64;
        grocery::rename_list(list: u64, name: String) -> ();
        grocery::delete_list(list: u64) -> Option<String>;
        grocery::clear_items(list: u64, all_lists: bool, pantry: bool, which: u8) -> Option<String>;
        grocery::remove_pantry_items(list: u64) -> Option<String>;
        grocery::move_bought_to_pantry(list: u64) -> Option<String>;
        grocery::undo_groceries() -> ();
        grocery::set_grocery_by_recipe(on: bool) -> ();
        pantry::pantry() -> pantry::Pantry;
        pantry::add_pantry(text: String) -> ();
        pantry::update_pantry(item: pantry::PantryEdit) -> ();
        pantry::delete_pantry(id: u64) -> Option<String>;
        pantry::remove_expired() -> Option<String>;
        pantry::remove_out_of_stock() -> Option<String>;
        pantry::pantry_to_list(ids: Vec<u64>, list: u64) -> String;
        pantry::set_pantry_sort(sort: u8) -> ();
        diary::diary_day(day: String, today: String) -> tracker::diary::DayView;
        diary::quick_add(day: String, today: String, slot: MealType, label: String, calories: Option<f32>, protein: Option<f32>, carbs: Option<f32>, fat: Option<f32>) -> diary::Logged;
        diary::copy_entries(from: String, day: String, today: String, slot: Option<MealType>) -> diary::Copied;
        diary::set_day_complete(day: String, today: String, complete: bool) -> tracker::diary::DayView;
        diary::planned_meals(today: String) -> Vec<tracker::diary::PlannedMeal>;
        diary::log_planned(today: String, picks: Vec<diary::Pick>) -> diary::Copied;
        diary::edit_entry(day: String, today: String, uid: String, amount: f32, unit: String, slot: MealType) -> diary::Changed;
        diary::delete_entry(day: String, today: String, uid: String) -> diary::Changed;
        diary::undo_diary(day: String, today: String) -> diary::Undone;
        foods::food_preview(food: model::MyFood) -> recipe_box::foods::Preview;
        foods::set_off_email(email: String) -> ();
        goals::targets_view(today: String) -> tracker::targets::TargetsView;
        goals::targets_preview(draft: tracker::targets::TargetsDraft, today: String) -> tracker::targets::TargetsResult;
        goals::save_targets(draft: tracker::targets::TargetsDraft, today: String, confirmed: bool) -> goals::SavedTargets;
        goals::accept_target_update(today: String) -> tracker::targets::TargetsView;
        body::body_view(range: tracker::body::BodyRange, today: String) -> tracker::body::BodyView;
        body::undo_body(range: tracker::body::BodyRange, today: String) -> tracker::body::BodyView;
        body::edit_measurements(day: String, today: String, values: Vec<tracker::body::DayValue>, confirmed: bool, range: Option<tracker::body::BodyRange>) -> body::BodyLogged;
        body::log_measure(day: String, today: String, measure: String, value: f64, unit: tracker::body::UnitIn, confirmed: bool, range: Option<tracker::body::BodyRange>) -> body::BodyLogged;
        body::save_measure(name: String, kind: tracker::body::MeasureKind, range: tracker::body::BodyRange, today: String) -> tracker::body::BodyView;
        body::archive_measure(uid: String, archived: bool, range: tracker::body::BodyRange, today: String) -> tracker::body::BodyView;
        exercise::exercise_activities(q: String, today: String) -> tracker::exercise::Activities;
        exercise::exercise_estimate(day: String, activity: tracker::exercise::ActivityRef, minutes: f64) -> tracker::exercise::Estimate;
        exercise::log_exercise(day: String, today: String, activity: tracker::exercise::ActivityRef, minutes: f64, kcal: Option<f64>) -> diary::Logged;
        exercise::edit_exercise(day: String, today: String, uid: String, minutes: f64, kcal: Option<f64>, use_estimate: bool) -> diary::Changed;
        exercise::delete_exercise(day: String, today: String, uid: String) -> diary::Changed;
        exercise::delete_activity(uid: String) -> String;
        exercise::save_activity(activity: tracker::journal::CustomActivity) -> String;
        home::home_view(range: tracker::body::BodyRange, today: String) -> tracker::home::HomeView;
        recipes::overview(today: String) -> recipes::Overview;
        recipes::list(view: String, search: String, sort: u8) -> Vec<recipes::Card>;
        recipes::recipe(id: u64, scale: f64, units: String) -> recipes::Detail;
        recipes::set_favorite(id: u64, on: bool) -> ();
        recipes::set_rating(id: u64, rating: u8) -> ();
        recipes::draft(id: u64) -> recipes::Draft;
        recipes::tidy(draft: recipes::Draft) -> (recipes::Draft, Vec<String>);
        recipes::save_recipe(id: u64, draft: recipes::Draft) -> u64;
        recipes::set_pinned(id: u64, on: bool) -> ();
        recipes::set_trashed(ids: Vec<u64>, on: bool) -> ();
        recipes::delete_forever(ids: Vec<u64>) -> usize;
        recipes::duplicate(id: u64) -> u64;
        recipes::set_nutrition(id: u64, per_serving: Option<model::Nutrients>) -> ();
        recipes::add_meal(recipe_id: u64, day: i32, kind: Option<MealType>) -> ();
        recipes::pending_notice() -> Option<recipes::NoticeView>;
        importing::import_text(text: String) -> importing::Review;
        importing::review_tidy(review: importing::Review) -> importing::Review;
        importing::review_existing(id: u64) -> importing::Review;
        importing::load_rules(text: String) -> String;
        importing::use_builtin_rules() -> ();
        settings::set_sidebar_order(order: Vec<String>) -> ();
        settings::set_launch_page(page: String) -> ();
        settings::set_settings(change: settings::Change) -> ();
        settings::backups(picked: Option<String>) -> Vec<settings::Saved>;
        settings::snapshots() -> Vec<settings::Saved>;
        settings::take_snapshot() -> ();
        settings::restore_backup(path: Option<String>, text: Option<String>, scope: Option<String>) -> String;
        settings::rollback(path: String) -> String;
        settings::start_empty() -> String;
        settings::my_foods() -> Vec<model::MyFood>;
        settings::save_my_food(food: model::MyFood) -> ();
        settings::delete_my_food(id: u64) -> ();
        settings::merge_duplicates() -> String;
    }
    off {
        foods::off_open(code: String) -> ();
        foods::online_food_draft(code: String) -> foods::OnlineDraft;
    }
    state_off {
        diary::log_food(day: String, today: String, slot: MealType, key: String, amount: f32, unit: String) -> diary::Logged;
        diary::diary_foods(q: String, today: String, day: String, slot: MealType) -> diary::DiaryFoods;
        foods::search_foods(q: String) -> foods::FoodResults;
        foods::food_detail(key: String, us: bool) -> recipe_box::foods::FoodDetail;
        foods::food_amount(key: String, amount: Option<f32>, unit: String, from: Option<String>) -> recipe_box::foods::Amount;
    }
    pure {
        diary::quick_add_check(calories: Option<f32>) -> Option<String>;
        importing::check_lines(ing: Vec<String>, steps: Vec<String>) -> importing::Checks;
    }
}

/// A folder tests may write in: the system temp folder, or `/tmp` under WASI, where
/// `std::env::temp_dir()` panics (the test runner maps a real folder there).
#[cfg(test)]
pub(crate) fn scratch_dir() -> PathBuf {
    #[cfg(target_os = "wasi")]
    return PathBuf::from("/tmp");
    #[cfg(not(target_os = "wasi"))]
    std::env::temp_dir()
}

/// The test process's id, for folder names; 0 under WASI, which has none (one process runs there).
#[cfg(test)]
pub(crate) fn test_pid() -> u32 {
    #[cfg(target_os = "wasi")]
    return 0;
    #[cfg(not(target_os = "wasi"))]
    std::process::id()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn state(name: &str) -> Shared {
        let dir = crate::scratch_dir().join(format!("tiffin-commands-{name}-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&dir);
        let journal = tracker::journal::Journal::open(tracker::journal::journal_dir(&dir));
        let mut lib = Library::default();
        lib.recipes.push(recipe_box::model::Recipe { id: 1, name: "Soup".into(), ..Default::default() });
        lib.next_id = 2;
        Mutex::new(AppState { lib, store: storage::Store::new(dir, false), undo: None, journal, unsaved: false })
    }

    fn services(name: &str) -> Services {
        let state = state(name);
        let off = foods::OffState::new(&state.lock().unwrap().store.dir);
        Services { state, off }
    }

    #[test]
    fn the_web_calls_commands_by_name_with_json() {
        let app = services("dispatch");
        let s = &app.state;
        s.lock().unwrap().lib.meals.push(model::Meal { id: 7, day: 10, recipe_id: Some(1), kind: MealType::Lunch, ..Default::default() });
        let got = dispatch(&app, "meals", json!({ "from": 9, "to": 11 })).unwrap();
        assert_eq!(got[0]["id"], 7);
        // camelCase keys like Tauri's, Option arguments may be left out
        dispatch(&app, "update_meal", json!({ "id": 7, "day": 12 })).unwrap();
        assert_eq!(s.lock().unwrap().lib.meals[0].day, 12);
        let removed = dispatch(&app, "remove_meals", json!({ "ids": [7] })).unwrap();
        assert!(s.lock().unwrap().lib.meals.is_empty());
        dispatch(&app, "restore_meals", json!({ "meals": removed })).unwrap();
        assert_eq!(s.lock().unwrap().lib.meals.len(), 1);
        // the change was saved, as through Tauri
        assert!(s.lock().unwrap().store.dir.join("library.json").exists());
    }

    #[test]
    fn a_wrong_call_is_an_error_not_a_panic() {
        let app = services("wrong");
        assert_eq!(dispatch(&app, "no_such_thing", json!({})).unwrap_err(), "Unknown command no_such_thing");
        assert!(dispatch(&app, "meals", json!({ "from": "monday" })).unwrap_err().starts_with("Bad arguments for meals"));
        assert!(dispatch(&app, "update_meal", json!({ "id": 99 })).is_err(), "a missing meal is the command's own error");
        // like Tauri: no arguments may come as null, and keys a command doesn't take are ignored
        assert_eq!(dispatch(&app, "meals", json!({ "from": 0, "to": 1, "extra": true })).unwrap(), json!([]));
        assert!(dispatch(&app, "meals", serde_json::Value::Null).unwrap_err().starts_with("Bad arguments for meals"), "null is no arguments, and meals needs two");
    }

    /// Through Tauri's own IPC (mock runtime): the names and camelCase arguments the screens send.
    #[cfg(feature = "tauri")]
    #[test]
    fn the_desktop_calls_the_same_commands_through_tauri() {
        use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
        use tauri::Manager;
        let app = mock_builder().manage(state("tauri")).invoke_handler(tauri_handler()).build(mock_context(noop_assets())).unwrap();
        let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let call = |cmd: &str, body: serde_json::Value| {
            let req = tauri::webview::InvokeRequest {
                cmd: cmd.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: tauri::ipc::InvokeBody::Json(body),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            };
            get_ipc_response(&w, req).map(|r| r.deserialize::<serde_json::Value>().unwrap())
        };
        app.state::<Shared>().lock().unwrap().lib.meals.push(model::Meal { id: 7, day: 10, recipe_id: Some(1), ..Default::default() });
        assert_eq!(call("meals", json!({ "from": 9, "to": 11 })).unwrap()[0]["id"], 7);
        call("update_meal", json!({ "id": 7, "day": 12 })).unwrap();
        assert_eq!(app.state::<Shared>().lock().unwrap().lib.meals[0].day, 12);
        assert!(call("update_meal", json!({ "id": 99 })).is_err());
    }

    #[test]
    fn a_first_run_opens_the_samples_and_saves_them() {
        let dir = crate::scratch_dir().join(format!("tiffin-commands-first-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut st = load_state(dir.clone());
        assert!(!st.lib.recipes.is_empty(), "the sample library");
        assert!(dir.join("library.json").exists(), "saved on the first run");
        // change what is on disk: the second start must open that, not fresh samples
        st.lib.recipes[0].name = "Only on disk".into();
        st.lib.recipes.truncate(1);
        st.store.save(&st.lib).unwrap();
        let again = load_state(dir.clone());
        assert_eq!(again.lib.recipes.len(), 1, "the second start opens what was saved");
        assert_eq!(again.lib.recipes[0].name, "Only on disk");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn every_listed_name_is_unique() {
        let mut names = NAMES.to_vec();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), NAMES.len());
    }
}
