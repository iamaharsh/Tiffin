//! Tiffin with a web interface. The window is Svelte (../src); everything it shows comes
//! from the `recipe_box` crate through the commands below.

mod foods;
mod grocery;
mod importing;
mod meals;
mod menus;
mod pantry;
mod settings;
mod update;

use recipe_box::model::{self, GroceryList, Library, MealType, Nutrients, PantryItem, Recipe};
use recipe_box::{cooking, health, importer, inbox, ingredients, nutrition, selftest, shortcut, storage, tracker};
use tauri::Manager;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) struct AppState {
    pub(crate) lib: Library,
    pub(crate) store: storage::Store,
    /// Grocery lists and pantry as they were before the last removal, for Undo.
    pub(crate) undo: Option<(Vec<GroceryList>, Vec<PantryItem>)>,
    /// Diary month files and the profile, in the journal folder beside library.json.
    pub(crate) journal: tracker::journal::Journal,
    /// A page holds typed input that is not saved yet (set by the page through set_unsaved); an update install waits for it.
    pub(crate) unsaved: bool,
}

pub(crate) type Shared = Mutex<AppState>;

pub(crate) const GONE: &str = "That recipe isn't in the library any more.";

#[derive(Serialize)]
struct Category {
    path: String,
    name: String,
    depth: usize,
    label: Option<usize>,
    count: usize,
}

#[derive(Serialize)]
struct Overview {
    sort: u8,
    categories: Vec<Category>,
    /// all, recent, top, favorites, uncategorized, trash
    counts: [usize; 6],
    pinned: Vec<Pinned>,
    /// grocery list names, for "Add to Grocery List"
    lists: Vec<String>,
    /// days since 1970-01-01, for the meal plan
    today: i32,
    /// why the library can't be changed right now (damaged file); shown as a permanent banner
    locked: Option<String>,
}

/// A one-time or standing message about the library file. `kept` is the verified copy's path, or ""
/// when the library file was missing (recovered) or no copy could be made yet (damaged).
#[derive(Serialize)]
pub(crate) struct NoticeView {
    /// recovered | replaced | damaged
    kind: &'static str,
    file: String,
    kept: String,
}

#[derive(Serialize)]
struct Pinned {
    id: u64,
    name: String,
}

#[derive(Serialize)]
struct Card {
    id: u64,
    name: String,
    rating: u8,
    favorite: bool,
    label: String,
    label_index: Option<usize>,
    photo: String,
    seed: u32,
    total_time: String,
    flags: usize,
    pinned: bool,
    in_trash: bool,
}

#[derive(Serialize)]
struct Row {
    amount: Option<String>,
    text: String,
    heading: bool,
}

/// A piece of a step: plain text, or a time phrase ("10 minutes") that can start a timer.
#[derive(Serialize)]
struct Piece {
    text: String,
    seconds: Option<u32>,
}

fn step_pieces(t: &str) -> Vec<Piece> {
    let mut out = vec![];
    let mut at = 0;
    for s in ingredients::find_times(t) {
        if s.start > at {
            out.push(Piece { text: t[at..s.start].to_string(), seconds: None });
        }
        out.push(Piece { text: t[s.start..s.end].to_string(), seconds: Some(s.seconds) });
        at = s.end;
    }
    if at < t.len() {
        out.push(Piece { text: t[at..].to_string(), seconds: None });
    }
    out
}

/// The fields the edit page changes. Everything else on the recipe (nutrition edits, trash,
/// pins, ids) is left as it is on disk.
#[derive(Serialize, Deserialize, Default)]
struct Draft {
    name: String,
    rating: u8,
    servings: String,
    prep_time: String,
    cook_time: String,
    total_time: String,
    difficulty: String,
    source: String,
    source_url: String,
    video_url: String,
    categories: Vec<String>,
    description: String,
    ingredients: String,
    directions: String,
    notes: String,
    photo: String,
    /// read only: picks the painted placeholder when there's no photo
    photo_seed: u32,
}

impl Draft {
    fn of(r: &Recipe) -> Draft {
        Draft {
            name: r.name.clone(),
            rating: r.rating,
            servings: r.servings.clone(),
            prep_time: r.prep_time.clone(),
            cook_time: r.cook_time.clone(),
            total_time: r.total_time.clone(),
            difficulty: r.difficulty.clone(),
            source: r.source.clone(),
            source_url: r.source_url.clone(),
            video_url: r.video_url.clone(),
            categories: r.categories.clone(),
            description: r.description.clone(),
            ingredients: r.ingredients.clone(),
            directions: r.directions.clone(),
            notes: r.notes.clone(),
            photo: r.photo.clone(),
            photo_seed: r.photo_seed,
        }
    }
    fn apply(&self, r: &mut Recipe) {
        let d = self;
        r.name = d.name.clone();
        r.rating = d.rating.min(5);
        r.servings = d.servings.clone();
        r.prep_time = d.prep_time.clone();
        r.cook_time = d.cook_time.clone();
        r.total_time = d.total_time.clone();
        r.difficulty = d.difficulty.clone();
        r.source = d.source.clone();
        r.source_url = d.source_url.clone();
        r.video_url = d.video_url.clone();
        r.categories = d.categories.clone();
        r.description = d.description.clone();
        r.ingredients = d.ingredients.clone();
        r.directions = d.directions.clone();
        r.notes = d.notes.clone();
        r.photo = d.photo.clone();
    }
}

#[derive(Serialize)]
struct Detail {
    card: Card,
    description: String,
    notes: String,
    source: String,
    source_url: String,
    prep_time: String,
    cook_time: String,
    servings: String,
    difficulty: String,
    rows: Vec<Row>,
    steps: Vec<Vec<Piece>>,
    /// where the video button goes (starts at 0:00), if the recipe has a video link
    video: Option<String>,
    per_serving: Nutrients,
    nutrition_edited: bool,
    nutrition_counted: usize,
    nutrition_skipped: usize,
    flags: Vec<String>,
}

fn in_category(r: &Recipe, cat: &str) -> bool {
    r.categories.iter().any(|x| x == cat || x.strip_prefix(cat).is_some_and(|rest| rest.starts_with('/')))
}

fn card(lib: &Library, r: &Recipe) -> Card {
    let (label, label_index) = model::recipe_label(&lib.label_colors, &r.categories);
    let total_time = if r.total_time.trim().is_empty() { ingredients::sum_times(&r.prep_time, &r.cook_time) } else { r.total_time.clone() };
    Card {
        id: r.id,
        name: r.name.clone(),
        rating: r.rating,
        favorite: r.favorite,
        label,
        label_index,
        photo: r.photo.clone(),
        seed: r.photo_seed,
        total_time,
        flags: r.review_flags.len(),
        pinned: r.pinned,
        in_trash: r.in_trash,
    }
}

#[tauri::command]
fn overview(state: tauri::State<Shared>) -> Overview {
    let s = state.lock().unwrap();
    let live: Vec<&Recipe> = s.lib.recipes.iter().filter(|r| !r.in_trash).collect();
    let mut paths = s.lib.categories.clone();
    paths.sort_by_key(|p| p.to_lowercase());
    let categories = paths
        .iter()
        .map(|p| Category {
            name: p.rsplit('/').next().unwrap_or(p).to_string(),
            depth: p.matches('/').count(),
            label: model::label_index(&s.lib.label_colors, p),
            count: live.iter().filter(|r| in_category(r, p)).count(),
            path: p.clone(),
        })
        .collect();
    let n = |f: &dyn Fn(&Recipe) -> bool| live.iter().filter(|r| f(r)).count();
    Overview {
        sort: s.lib.settings.recipe_sort,
        categories,
        counts: [
            live.len(),
            live.len(),
            n(&|r| r.rating >= 4),
            n(&|r| r.favorite),
            n(&|r| r.categories.is_empty()),
            s.lib.recipes.len() - live.len(),
        ],
        pinned: live.iter().filter(|r| r.pinned).map(|r| Pinned { id: r.id, name: r.name.clone() }).collect(),
        lists: s.lib.grocery_lists.iter().map(|l| l.name.clone()).collect(),
        today: model::today(),
        locked: s.store.problem.as_ref().map(|p| p.banner()),
    }
}

/// `view`: all | recent | top | favorites | uncategorized | trash | cat:<path>.
/// `sort`: 0 name, 1 rating, 2 most recent (the classic app's settings values).
#[tauri::command]
fn list(state: tauri::State<Shared>, view: String, search: String, sort: u8) -> Vec<Card> {
    let s = state.lock().unwrap();
    let words = model::search_words(&search);
    let mut v: Vec<&Recipe> = s
        .lib
        .recipes
        .iter()
        .filter(|r| r.in_trash == (view == "trash"))
        .filter(|r| match view.as_str() {
            "top" => r.rating >= 4,
            "favorites" => r.favorite,
            "uncategorized" => r.categories.is_empty(),
            v => v.strip_prefix("cat:").map_or(true, |c| in_category(r, c)),
        })
        .filter(|r| model::recipe_matches(r, &words))
        .collect();
    let sort = match view.as_str() {
        "recent" => 2,
        "top" => 1,
        _ => sort,
    };
    match sort {
        1 => v.sort_by(|a, b| b.rating.cmp(&a.rating).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))),
        2 => v.sort_by(|a, b| b.created.cmp(&a.created)),
        _ => v.sort_by_key(|r| r.name.to_lowercase()),
    }
    v.iter().map(|r| card(&s.lib, r)).collect()
}

/// `units`: original | metric | us.
#[tauri::command]
fn recipe(state: tauri::State<Shared>, id: u64, scale: f64, units: String) -> Result<Detail, String> {
    let s = state.lock().unwrap();
    let r = s.lib.recipe(id).ok_or(GONE)?;
    let rows = r
        .ingredient_lines()
        .iter()
        .map(|l| {
            let (amount, text, heading) = ingredients::display_row(l, scale, units == "metric", units == "us");
            Row { amount, text, heading }
        })
        .collect();
    let calc = nutrition::calculate(r);
    let total = r.nutrition.unwrap_or(calc.total);
    Ok(Detail {
        card: card(&s.lib, r),
        description: r.description.clone(),
        notes: r.notes.clone(),
        source: r.source.clone(),
        source_url: r.source_url.clone(),
        prep_time: r.prep_time.clone(),
        cook_time: r.cook_time.clone(),
        servings: r.servings.clone(),
        difficulty: r.difficulty.clone(),
        rows,
        steps: r.steps().iter().map(|t| step_pieces(&ingredients::strip_step_number(t))).collect(),
        video: cooking::video_play_url(&r.video_url, 0),
        per_serving: total.scaled(1.0 / r.servings_count()),
        nutrition_edited: r.nutrition_edited,
        nutrition_counted: calc.counted,
        nutrition_skipped: calc.skipped,
        flags: r.review_flags.clone(),
    })
}

/// Change the library and save. This app is the only writer; a change is refused only while the
/// library is damaged (DAMAGED) or from a newer version (READ_ONLY).
pub(crate) fn mutate<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
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
pub(crate) fn replace_library<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
    replace_library_then(state, f, |_| Ok(()))
}

/// [`replace_library`] plus a step that runs only after the library file was saved, under the same lock
/// (a restore replaces the diary here, so a failed library save can't leave the diary replaced and the library not).
pub(crate) fn replace_library_then<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>, after: impl FnOnce(&mut AppState) -> Result<(), String>) -> Result<T, String> {
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
pub(crate) fn mutate_keeping_undo<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
    let mut guard = state.lock().unwrap();
    let st = &mut *guard;
    st.store.check_unlocked()?;
    let out = f(st)?;
    st.store.save(&st.lib)?;
    Ok(out)
}

/// Change the diary or profile and save the journal files that changed. It never touches
/// library.json (no save, no library snapshot), and it doesn't check the library's lock: a damaged
/// or newer library must not block the diary. Each journal file guards itself.
#[cfg_attr(not(test), allow(dead_code))] // first caller: the diary commands in Phase 3
pub(crate) fn mutate_journal<T>(state: &Shared, f: impl FnOnce(&mut AppState) -> Result<T, String>) -> Result<T, String> {
    let mut guard = state.lock().unwrap();
    let st = &mut *guard;
    let out = f(st)?;
    st.journal.save_dirty()?;
    Ok(out)
}

/// What every recipe change ends with (the classic save_recipe_changes): a new modified time,
/// and the tracker file refreshed when auto-send is on.
pub(crate) fn touched(r: &mut Recipe, set: &model::Settings) {
    r.modified = model::now_secs();
    if set.tracker_auto_send && r.nutrition.is_some() && health::export(r, &set.tracker_service, &set.tracker_folder).is_ok() {
        r.sent_to_tracker = Some(model::now_secs());
    }
}

/// Change one recipe and save.
fn change(state: &Shared, id: u64, f: impl FnOnce(&mut Recipe)) -> Result<(), String> {
    mutate(state, |st| {
        let set = st.lib.settings.clone();
        let r = st.lib.recipe_mut(id).ok_or(GONE)?;
        f(r);
        touched(r, &set);
        Ok(())
    })
}

#[tauri::command]
fn set_favorite(state: tauri::State<Shared>, id: u64, on: bool) -> Result<(), String> {
    change(&state, id, |r| r.favorite = on)
}

#[tauri::command]
fn set_rating(state: tauri::State<Shared>, id: u64, rating: u8) -> Result<(), String> {
    change(&state, id, |r| r.rating = rating.min(5))
}

#[tauri::command]
fn draft(state: tauri::State<Shared>, id: u64) -> Result<Draft, String> {
    if id == 0 {
        return Ok(Draft::default());
    }
    let s = state.lock().unwrap();
    s.lib.recipe(id).map(Draft::of).ok_or_else(|| GONE.into())
}

/// Fix amounts, units, numbering and capitals (the classic "Tidy formatting"); nothing is saved.
#[tauri::command]
fn tidy(state: tauri::State<Shared>, draft: Draft) -> (Draft, Vec<String>) {
    let short = state.lock().unwrap().lib.settings.unit_style_short;
    let mut r = Recipe::default();
    draft.apply(&mut r);
    let changes = importer::tidy_recipe(&mut r, short);
    (Draft { photo_seed: draft.photo_seed, ..Draft::of(&r) }, changes)
}

/// Save the edit page (id 0 = a new recipe) and return the recipe's id. Same steps as the
/// classic app: blank name becomes "Untitled Recipe", nutrition is recalculated unless typed in
/// by hand, import warnings are re-checked if there were any, and the tracker file is refreshed
/// when auto-send is on.
#[tauri::command]
fn save_recipe(state: tauri::State<Shared>, id: u64, draft: Draft) -> Result<u64, String> {
    mutate(&state, |st| {
        let mut r = if id == 0 { Recipe::default() } else { st.lib.recipe(id).cloned().ok_or(GONE)? };
        draft.apply(&mut r);
        if r.name.trim().is_empty() {
            r.name = "Untitled Recipe".into();
        }
        if !r.nutrition_edited {
            r.nutrition = Some(nutrition::calculate(&r).total);
        }
        if !r.review_flags.is_empty() {
            r.review_flags = importer::review_summary(&r);
        }
        touched(&mut r, &st.lib.settings);
        Ok(if id == 0 {
            st.lib.add_recipe(r)
        } else {
            *st.lib.recipe_mut(id).ok_or(GONE)? = r;
            id
        })
    })
}

#[tauri::command]
fn set_pinned(state: tauri::State<Shared>, id: u64, on: bool) -> Result<(), String> {
    change(&state, id, |r| r.pinned = on)
}

/// Move to Trash, or back out of it.
#[tauri::command]
fn set_trashed(state: tauri::State<Shared>, ids: Vec<u64>, on: bool) -> Result<(), String> {
    mutate(&state, |st| {
        let t = model::now_secs();
        for r in st.lib.recipes.iter_mut().filter(|r| ids.contains(&r.id)) {
            r.in_trash = on;
            r.modified = t;
        }
        Ok(())
    })
}

/// Remove for good. Like the classic app: only recipes already in Trash go, a Version History
/// snapshot is taken first, and a tombstone stops sync bringing them back.
#[tauri::command]
fn delete_forever(state: tauri::State<Shared>, ids: Vec<u64>) -> Result<usize, String> {
    mutate(&state, |st| {
        let ids: Vec<u64> = ids.into_iter().filter(|id| st.lib.recipe(*id).is_some_and(|r| r.in_trash)).collect();
        if ids.is_empty() {
            return Ok(0);
        }
        st.store.snapshot(&st.lib, "before deleting recipes")?;
        let t = model::now_secs();
        let gone: Vec<(String, u64)> = st.lib.recipes.iter().filter(|r| ids.contains(&r.id)).map(|r| (r.uid.clone(), t)).collect();
        st.lib.tombstones.extend(gone);
        st.lib.recipes.retain(|r| !ids.contains(&r.id));
        Ok(ids.len())
    })
}

#[tauri::command]
fn duplicate(state: tauri::State<Shared>, id: u64) -> Result<u64, String> {
    mutate(&state, |st| {
        let mut copy = st.lib.recipe(id).cloned().ok_or(GONE)?;
        copy.uid = String::new();
        copy.name = format!("{} (copy)", copy.name);
        copy.pinned = false;
        copy.created = model::now_secs();
        touched(&mut copy, &st.lib.settings);
        Ok(st.lib.add_recipe(copy))
    })
}

/// Nutrition typed in by hand (per serving), or `None` to go back to the calculated estimate.
#[tauri::command]
fn set_nutrition(state: tauri::State<Shared>, id: u64, per_serving: Option<Nutrients>) -> Result<(), String> {
    change(&state, id, |r| match per_serving {
        Some(n) => {
            r.nutrition = Some(nutrition::hand_typed(&n, r.servings_count()));
            r.nutrition_edited = true;
        }
        None => {
            r.nutrition_edited = false;
            r.nutrition = Some(nutrition::calculate(r).total);
        }
    })
}

#[tauri::command]
fn add_meal(state: tauri::State<Shared>, recipe_id: u64, day: i32, kind: Option<MealType>) -> Result<(), String> {
    mutate(&state, |st| {
        let name = st.lib.recipe(recipe_id).ok_or(GONE)?.name.clone();
        let kind = kind.unwrap_or_else(|| meals::free_kind(&st.lib.meals, day));
        let id = st.lib.new_id();
        st.lib.meals.push(model::Meal { id, day, kind, recipe_id: Some(recipe_id), name, modified: model::now_secs() });
        Ok(())
    })
}

/// What the page should tell the user about the library file, if anything. Recovered and replaced
/// come once; damaged keeps coming until the user picks a way to recover.
#[tauri::command]
fn pending_notice(state: tauri::State<Shared>) -> Result<Option<NoticeView>, String> {
    let mut s = state.lock().unwrap();
    let main = storage::library_path(&s.store.dir).display().to_string();
    let path = |p: Option<PathBuf>| p.map(|p| p.display().to_string()).unwrap_or_default();
    Ok(s.store.take_notice().map(|n| match n {
        storage::Notice::Recovered { kept } => NoticeView { kind: "recovered", file: main, kept: path(kept) },
        storage::Notice::Replaced { kept } => NoticeView { kind: "replaced", file: main, kept: path(Some(kept)) },
        storage::Notice::Damaged { file, kept } => NoticeView { kind: "damaged", file: path(Some(file)), kept: path(kept) },
    }))
}

/// The picture's bytes arrive as the raw request body (not a JSON number array); the `stem`
/// header names the file. Returns the new file name in the photos folder.
#[tauri::command]
async fn add_photo(state: tauri::State<'_, Shared>, request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("No picture was sent.".into());
    };
    let stem = request.headers().get("stem").and_then(|v| v.to_str().ok()).unwrap_or("photo").to_lowercase();
    let dir = {
        let s = state.lock().unwrap();
        s.store.check_unlocked()?;
        storage::photos_dir(&s.store.dir)
    };
    storage::save_photo_bytes(bytes, &dir, &stem)
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

pub(crate) fn web_address(u: &str) -> bool {
    (u.starts_with("https://") || u.starts_with("http://")) && !u.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"')
}

/// Photo file names are made by storage::photo_name: letters, digits, '-' and one ".jpg".
fn safe_photo_name(name: &str) -> bool {
    !name.is_empty() && !name.contains("..") && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
}

fn load_state(data_dir: PathBuf) -> AppState {
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
    remove_instance_file();
    AppState { lib, store, undo: None, journal, unsaved: false }
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
        .manage(foods::OffState::new(&data_dir))
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
        .invoke_handler(tauri::generate_handler![
            overview,
            list,
            recipe,
            set_favorite,
            set_rating,
            draft,
            tidy,
            save_recipe,
            add_photo,
            pending_notice,
            open_link,
            mica,
            launch_notes,
            show_window,
            set_pinned,
            set_trashed,
            delete_forever,
            duplicate,
            set_nutrition,
            add_meal,
            update::check_update,
            update::install_update,
            update::set_unsaved,
            importing::import_url,
            importing::import_text,
            importing::check_lines,
            importing::review_tidy,
            importing::review_existing,
            importing::save_review,
            importing::take_inbox,
            importing::rules_overview,
            importing::load_rules,
            importing::use_builtin_rules,
            importing::rules_from_sync,
            grocery::groceries,
            grocery::grocery_preview,
            grocery::add_recipe_to_list,
            grocery::plan_preview,
            grocery::add_plan_to_list,
            grocery::add_item,
            grocery::update_item,
            grocery::remove_item,
            grocery::new_list,
            grocery::rename_list,
            grocery::delete_list,
            grocery::clear_items,
            grocery::remove_pantry_items,
            grocery::move_bought_to_pantry,
            grocery::undo_groceries,
            grocery::set_grocery_by_recipe,
            pantry::pantry,
            pantry::add_pantry,
            pantry::update_pantry,
            pantry::delete_pantry,
            pantry::remove_expired,
            pantry::remove_out_of_stock,
            pantry::pantry_to_list,
            pantry::set_pantry_sort,
            meals::meals,
            meals::update_meal,
            meals::remove_meals,
            meals::restore_meals,
            menus::menus,
            menus::new_menu,
            menus::save_menu,
            menus::delete_menu,
            menus::restore_menu,
            menus::apply_menu,
            settings::settings_info,
            settings::set_settings,
            settings::sync_now,
            settings::auto_sync,
            settings::backup_now,
            settings::backups,
            settings::snapshots,
            settings::take_snapshot,
            settings::restore_backup,
            settings::rollback,
            settings::start_empty,
            settings::send_all_to_tracker,
            settings::setup_inbox,
            settings::open_folder,
            settings::my_foods,
            settings::save_my_food,
            settings::delete_my_food,
            settings::label_lookup,
            settings::label_photo,
            settings::self_tests,
            settings::merge_duplicates,
            foods::off_search,
            foods::set_off_email,
            foods::search_foods,
            foods::food_detail,
            foods::food_amount,
            foods::food_preview,
            foods::off_open,
            foods::online_food_draft,
        ])
        .run(tauri::generate_context!())
        .expect("Tiffin couldn't start");
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let p = super::step_pieces("Simmer for 10 minutes, then rest 1 hour.");
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
        assert!(super::in_category(&r, "Desserts"));
        assert!(super::in_category(&r, "Desserts/Cakes"));
        assert!(!super::in_category(&r, "Dessert"));
    }
}
