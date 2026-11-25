//! The recipe library's own screens: the overview and sidebar, the recipe list and cards, a recipe
//! with its pieces, the edit form, and the recipe actions (favorite, rating, pin, trash, duplicate,
//! nutrition, add to the meal plan), plus the notice after a damaged file was replaced.

use crate::{mutate, AppState, Shared, GONE};
use recipe_box::model::{self, Library, MealType, Nutrients, Recipe};
use recipe_box::{cooking, health, importer, ingredients, nutrition, storage, tracker};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize)]
pub struct Category {
    path: String,
    name: String,
    depth: usize,
    label: Option<usize>,
    count: usize,
}

#[derive(Serialize)]
pub struct Overview {
    sort: u8,
    categories: Vec<Category>,
    /// all, recent, top, favorites, uncategorized, trash
    counts: [usize; 6],
    pinned: Vec<Pinned>,
    /// grocery list names, for "Add to Grocery List"
    lists: Vec<String>,
    /// the viewer's local calendar day as days since 1970-01-01, the number the diary compares Meal.day with (never `model::today()`, the UTC day)
    pub today: i32,
    /// why the library can't be changed right now (damaged file); shown as a permanent banner
    locked: Option<String>,
    /// the sidebar's tabs in the person's order, and the tab Tiffin opens on (see `recipe_box::sidebar`)
    sidebar: Vec<&'static str>,
    launch: &'static str,
}

/// A one-time or standing message about the library file. `kept` is the verified copy's path, or ""
/// when the library file was missing (recovered) or no copy could be made yet (damaged).
#[derive(Serialize)]
pub struct NoticeView {
    /// recovered | replaced | damaged
    kind: &'static str,
    file: String,
    kept: String,
}

#[derive(Serialize)]
pub struct Pinned {
    id: u64,
    name: String,
}

#[derive(Serialize)]
pub struct Card {
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
pub struct Row {
    amount: Option<String>,
    text: String,
    heading: bool,
}

/// A piece of a step: plain text, or a time phrase ("10 minutes") that can start a timer.
#[derive(Serialize)]
pub struct Piece {
    pub text: String,
    pub seconds: Option<u32>,
}

pub fn step_pieces(t: &str) -> Vec<Piece> {
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
pub struct Draft {
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
pub struct Detail {
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

pub fn in_category(r: &Recipe, cat: &str) -> bool {
    r.categories.iter().any(|x| x == cat || x.strip_prefix(cat).is_some_and(|rest| rest.starts_with('/')))
}

pub fn card(lib: &Library, r: &Recipe) -> Card {
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

pub fn overview(state: &Shared, today: String) -> Result<Overview, String> {
    overview_of(&state.lock().unwrap(), &today)
}

pub fn overview_of(s: &AppState, today: &str) -> Result<Overview, String> {
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
    Ok(Overview {
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
        today: tracker::diary::day_number(today)?,
        locked: s.store.problem.as_ref().map(|p| p.banner()),
        sidebar: recipe_box::sidebar::order(&s.lib.settings),
        launch: recipe_box::sidebar::launch_page(&s.lib.settings),
    })
}

/// `view`: all | recent | top | favorites | uncategorized | trash | cat:<path>.
/// `sort`: 0 name, 1 rating, 2 most recent (the classic app's settings values).
pub fn list(state: &Shared, view: String, search: String, sort: u8) -> Result<Vec<Card>, String> {
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
    Ok(v.iter().map(|r| card(&s.lib, r)).collect())
}

/// `units`: original | metric | us.
pub fn recipe(state: &Shared, id: u64, scale: f64, units: String) -> Result<Detail, String> {
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

/// What every recipe change ends with (the classic save_recipe_changes): a new modified time,
/// and the tracker file refreshed when auto-send is on.
pub fn touched(r: &mut Recipe, set: &model::Settings) {
    r.modified = model::now_secs();
    if set.tracker_auto_send && r.nutrition.is_some() && health::export(r, &set.tracker_service, &set.tracker_folder).is_ok() {
        r.sent_to_tracker = Some(model::now_secs());
    }
}

/// Change one recipe and save.
pub fn change(state: &Shared, id: u64, f: impl FnOnce(&mut Recipe)) -> Result<(), String> {
    mutate(state, |st| {
        let set = st.lib.settings.clone();
        let r = st.lib.recipe_mut(id).ok_or(GONE)?;
        f(r);
        touched(r, &set);
        Ok(())
    })
}

pub fn set_favorite(state: &Shared, id: u64, on: bool) -> Result<(), String> {
    change(&state, id, |r| r.favorite = on)
}

pub fn set_rating(state: &Shared, id: u64, rating: u8) -> Result<(), String> {
    change(&state, id, |r| r.rating = rating.min(5))
}

pub fn draft(state: &Shared, id: u64) -> Result<Draft, String> {
    if id == 0 {
        return Ok(Draft::default());
    }
    let s = state.lock().unwrap();
    s.lib.recipe(id).map(Draft::of).ok_or_else(|| GONE.into())
}

/// Fix amounts, units, numbering and capitals (the classic "Tidy formatting"); nothing is saved.
pub fn tidy(state: &Shared, draft: Draft) -> Result<(Draft, Vec<String>), String> {
    let short = state.lock().unwrap().lib.settings.unit_style_short;
    let mut r = Recipe::default();
    draft.apply(&mut r);
    let changes = importer::tidy_recipe(&mut r, short);
    Ok((Draft { photo_seed: draft.photo_seed, ..Draft::of(&r) }, changes))
}

/// Save the edit page (id 0 = a new recipe) and return the recipe's id. Same steps as the
/// classic app: blank name becomes "Untitled Recipe", nutrition is recalculated unless typed in
/// by hand, import warnings are re-checked if there were any, and the tracker file is refreshed
/// when auto-send is on.
pub fn save_recipe(state: &Shared, id: u64, draft: Draft) -> Result<u64, String> {
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

pub fn set_pinned(state: &Shared, id: u64, on: bool) -> Result<(), String> {
    change(&state, id, |r| r.pinned = on)
}

/// Move to Trash, or back out of it.
pub fn set_trashed(state: &Shared, ids: Vec<u64>, on: bool) -> Result<(), String> {
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
pub fn delete_forever(state: &Shared, ids: Vec<u64>) -> Result<usize, String> {
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

pub fn duplicate(state: &Shared, id: u64) -> Result<u64, String> {
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
pub fn set_nutrition(state: &Shared, id: u64, per_serving: Option<Nutrients>) -> Result<(), String> {
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

pub fn add_meal(state: &Shared, recipe_id: u64, day: i32, kind: Option<MealType>) -> Result<(), String> {
    mutate(&state, |st| {
        let name = st.lib.recipe(recipe_id).ok_or(GONE)?.name.clone();
        let kind = kind.unwrap_or_else(|| crate::meals::free_kind(&st.lib.meals, day));
        let id = st.lib.new_id();
        st.lib.meals.push(model::Meal { id, day, kind, recipe_id: Some(recipe_id), name, modified: model::now_secs() });
        Ok(())
    })
}

/// What the page should tell the user about the library file, if anything. Recovered and replaced
/// come once; damaged keeps coming until the user picks a way to recover.
pub fn pending_notice(state: &Shared) -> Result<Option<NoticeView>, String> {
    let mut s = state.lock().unwrap();
    let main = storage::library_path(&s.store.dir).display().to_string();
    let path = |p: Option<PathBuf>| p.map(|p| p.display().to_string()).unwrap_or_default();
    Ok(s.store.take_notice().map(|n| match n {
        storage::Notice::Recovered { kept } => NoticeView { kind: "recovered", file: main, kept: path(kept) },
        storage::Notice::Replaced { kept } => NoticeView { kind: "replaced", file: main, kept: path(Some(kept)) },
        storage::Notice::Damaged { file, kept } => NoticeView { kind: "damaged", file: path(Some(file)), kept: path(kept) },
    }))
}

/// Save raw picture bytes into the library's photos folder after verifying the library isn't
/// locked. Returns the saved photo file name.
pub fn add_photo_bytes(state: &Shared, bytes: &[u8], stem: &str) -> Result<String, String> {
    let dir = {
        let s = state.lock().unwrap();
        s.store.check_unlocked()?;
        storage::photos_dir(&s.store.dir)
    };
    storage::save_photo_bytes(bytes, &dir, &stem.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load_state;
    use std::sync::Mutex;

    /// Valid 1×1 24-bit BMP (58 bytes), no checksums needed.
    const ONE_PX_BMP: [u8; 58] = [
        b'B', b'M', 58, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0,
        40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 24, 0,
        0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0x20, 0x80, 0xe0, 0x00,
    ];

    #[test]
    fn add_photo_bytes_saves_valid_image_and_respects_lock() {
        let dir = crate::scratch_dir().join(format!("tiffin-commands-photo-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = Mutex::new(load_state(dir.clone()));

        let name = add_photo_bytes(&state, &ONE_PX_BMP, "Carrot-Cake").unwrap();
        assert!(name.starts_with("carrot-cake-") && name.ends_with(".jpg"), "{name}");
        assert!(storage::photos_dir(&dir).join(&name).is_file());
        assert!(add_photo_bytes(&state, b"not an image", "bad").is_err());

        // damaged library refuses before touching photos_dir
        let bad_dir = crate::scratch_dir().join(format!("tiffin-commands-photo-bad-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&bad_dir);
        std::fs::create_dir_all(&bad_dir).unwrap();
        std::fs::write(bad_dir.join("library.json"), "{\"recipes\": [").unwrap();
        let bad_state = Mutex::new(load_state(bad_dir.clone()));
        assert_eq!(add_photo_bytes(&bad_state, &ONE_PX_BMP, "locked").unwrap_err(), storage::DAMAGED);
        assert!(!storage::photos_dir(&bad_dir).exists());

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&bad_dir);
    }
}

