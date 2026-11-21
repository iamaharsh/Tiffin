//! The Foods page commands: local search on every keystroke, and Open Food Facts only on demand.
//! Network work never runs under a lock; the rules live in `recipe_box::foods`.

use recipe_box::foods::{self, off, Amount, FoodDetail, FoodRow, OnlineFood, RecipeFood};
use recipe_box::model::{self, Library, MyFood, Recipe};
use recipe_box::{health, nutrition};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

use crate::Shared;

/// Online search state: the send limiter, the hits of the last search and the cache of opened products. Managed beside `Shared`, never inside it.
pub(crate) struct Off {
    pub(crate) limiter: off::Limiter,
    pub(crate) hits: Vec<OnlineFood>,
    pub(crate) cache: off::OffCache,
}

pub(crate) struct OffState(pub(crate) Mutex<Off>);

impl OffState {
    pub(crate) fn new(data_dir: &Path) -> OffState {
        OffState(Mutex::new(Off { limiter: off::Limiter::default(), hits: vec![], cache: off::OffCache::open(off::cache_path(data_dir)) }))
    }
}

/// The cached products, plus the hits of the last online search when asked (a hit replaces its cached copy).
pub(crate) fn online_foods(off: &OffState, with_hits: bool) -> Vec<OnlineFood> {
    let o = off.0.lock().unwrap();
    let mut out = if with_hits { o.hits.clone() } else { vec![] };
    let seen: Vec<String> = out.iter().map(|f| f.code.clone()).collect();
    out.extend(o.cache.items().iter().filter(|f| !seen.contains(&f.code)).cloned());
    out
}

/// A product by code from the last hits or the cache.
fn find_online(off: &OffState, code: &str) -> Result<OnlineFood, String> {
    online_foods(off, true).into_iter().find(|f| f.code == code).ok_or_else(|| "That product isn't in the online results any more. Search online again.".to_string())
}

/// Keep an opened online product for offline search.
fn open_online(off: &OffState, code: &str) -> Result<(), String> {
    let food = find_online(off, code)?;
    off.0.lock().unwrap().cache.remember(food, model::now_secs())
}

/// Opening an online result keeps it in the local cache so later searches find it offline.
#[tauri::command]
pub(crate) fn off_open(off: tauri::State<OffState>, code: String) -> Result<(), String> {
    open_online(&off, &code)
}

/// An online food turned into a My Food draft; `ready` is false when its values need a look first. Saving stays with `save_my_food`.
#[derive(Serialize)]
pub(crate) struct OnlineDraft {
    food: MyFood,
    ready: bool,
}

#[tauri::command]
pub(crate) fn online_food_draft(off: tauri::State<OffState>, code: String) -> Result<OnlineDraft, String> {
    let (food, ready) = off::as_my_food(&find_online(&off, &code)?);
    Ok(OnlineDraft { food, ready })
}

fn recipe_food(r: &Recipe) -> RecipeFood {
    RecipeFood { uid: r.uid.clone(), name: r.name.clone(), per_serving: health::per_serving(r) }
}

/// The recipes that can be logged as foods: not in the trash, with a uid.
pub(crate) fn recipe_foods(lib: &Library) -> Vec<RecipeFood> {
    lib.recipes.iter().filter(|r| !r.in_trash && !r.uid.is_empty()).map(recipe_food).collect()
}

/// A recipe opened as a food. Takes the library, so a journal write can use it under its own lock.
pub(crate) fn recipe_detail_in(lib: &Library, uid: &str, us: bool) -> Result<FoodDetail, String> {
    let r = lib.recipes.iter().find(|r| !r.in_trash && r.uid == uid).ok_or(crate::GONE)?;
    // grams are offered only when every ingredient weighs (D-08)
    let serving_g = nutrition::recipe_grams(r).map(|g| g / r.servings_count());
    Ok(foods::recipe_detail(&recipe_food(r), serving_g, us))
}

/// Rank the My Foods, recipes and online foods for `q` (each lock is dropped before the next is taken).
fn rows(shared: &Shared, off: &OffState, q: &str, with_hits: bool) -> Vec<FoodRow> {
    let (mine, recipes) = {
        let st = shared.lock().unwrap();
        (st.lib.my_foods.clone(), recipe_foods(&st.lib))
    };
    let online = online_foods(off, with_hits);
    foods::search_with(q, &mine, &online, &recipes, &[])
}

/// How an online search went. `status` is `ok`, `empty`, `offline`, `slow`, `limited`, `wait` or `needs_email` (nothing was sent: no contact email is saved); the rows are always the local results plus any hits.
#[derive(Serialize)]
pub(crate) struct OnlineResult {
    status: &'static str,
    retry_in: u64,
    added: usize,
    rows: Vec<FoodRow>,
}

/// `Some(seconds to wait)` when the limiter or a hard stop refuses a send now; otherwise the send is counted.
fn begin(off: &OffState, now: u64) -> Option<u64> {
    off.0.lock().unwrap().limiter.allow(now).err()
}

fn finish(shared: &Shared, off: &OffState, q: &str, answer: off::OffAnswer, now: u64) -> OnlineResult {
    let (status, retry_in, added) = {
        let mut o = off.0.lock().unwrap();
        match answer {
            off::OffAnswer::Found(hits) => {
                let n = hits.len();
                o.hits = hits;
                ("ok", 0, n)
            }
            off::OffAnswer::Empty => {
                o.hits.clear();
                ("empty", 0, 0)
            }
            off::OffAnswer::Offline => ("offline", 0, 0),
            off::OffAnswer::Slow => ("slow", 0, 0),
            off::OffAnswer::NeedsEmail => ("needs_email", 0, 0),
            off::OffAnswer::Limited(s) => {
                o.limiter.block(now, s);
                ("limited", s, 0)
            }
        }
    };
    OnlineResult { status, retry_in, added, rows: rows(shared, off, q, true) }
}

/// The rows for the box on the Foods page, and whether Search Online may be pressed for the same text.
#[derive(Serialize)]
pub(crate) struct FoodResults {
    rows: Vec<FoodRow>,
    online_ready: bool,
}

/// Local search on every keystroke: the bundled foods, My Foods and the cached online products. No network.
#[tauri::command]
pub(crate) fn search_foods(state: tauri::State<Shared>, off: tauri::State<OffState>, q: String) -> Result<FoodResults, String> {
    // the hits of the last online search show only in its own result, so they go when the query changes
    Ok(FoodResults { rows: rows(&state, &off, &q, false), online_ready: off::query_ready(&q) })
}

/// Open a food (a row's key) with its units. Looks in the last online hits too, so an opened online row resolves.
pub(crate) fn detail_of(shared: &Shared, off: &OffState, key: &str, us: bool) -> Result<FoodDetail, String> {
    if let Some(uid) = key.strip_prefix("recipe:") {
        return recipe_detail_in(&shared.lock().unwrap().lib, uid, us);
    }
    // a logged entry: measured against its own frozen basis, never the live food (TRK-04)
    if let Some(rest) = key.strip_prefix("entry:") {
        let (day, uid) = rest.split_once(':').ok_or(foods::GONE)?;
        return recipe_box::tracker::diary::entry_detail(&mut shared.lock().unwrap().journal, day, uid, us);
    }
    let mine = shared.lock().unwrap().lib.my_foods.clone();
    let online = online_foods(off, true);
    foods::detail(key, &mine, &online, us)
}

#[tauri::command]
pub(crate) fn food_detail(state: tauri::State<Shared>, off: tauri::State<OffState>, key: String, us: bool) -> Result<FoodDetail, String> {
    detail_of(&state, &off, &key, us)
}

/// What `amount` of `unit` weighs and holds. `from` is the unit the amount was typed in when the page changes the unit; the crate converts and rounds.
fn amount_of(shared: &Shared, off: &OffState, key: &str, amount: Option<f32>, unit: &str, from: Option<&str>) -> Result<Amount, String> {
    foods::amount(&detail_of(shared, off, key, false)?, amount, unit, from)
}

#[tauri::command]
pub(crate) fn food_amount(state: tauri::State<Shared>, off: tauri::State<OffState>, key: String, amount: Option<f32>, unit: String, from: Option<String>) -> Result<Amount, String> {
    amount_of(&state, &off, &key, amount, &unit, from.as_deref())
}

/// The crate's preview plus a taken name as a name problem. The rule stays in the crate; the preview is where the form looks for problems.
pub(crate) fn preview_of(state: &Shared, food: &MyFood) -> foods::Preview {
    let mut p = foods::preview(food);
    if p.problem.is_none() && recipe_box::nutrition::check_my_food_name(&state.lock().unwrap().lib, food).is_err() {
        p.problem = Some(foods::FoodProblem { field: "name", message: foods::NAME_TAKEN });
    }
    p
}

/// The My Food form's live line, warnings and blocking field. Reads the open library and saves nothing.
#[tauri::command]
pub(crate) fn food_preview(state: tauri::State<Shared>, food: MyFood) -> Result<foods::Preview, String> {
    Ok(preview_of(&state, &food))
}

/// Search Online: one request with only the typed words. The limit is checked here, not only by the button.
#[tauri::command]
pub(crate) async fn off_search(state: tauri::State<'_, Shared>, off: tauri::State<'_, OffState>, q: String) -> Result<OnlineResult, String> {
    if !off::query_ready(&q) {
        return Err("Type at least two letters to search online.".into());
    }
    // no email, no send: the page asks for one, and the limiter doesn't count a request that never left
    if off::valid_email(&state.lock().unwrap().lib.settings.off_email).is_err() {
        return Ok(finish(&state, &off, &q, off::OffAnswer::NeedsEmail, model::now_secs()));
    }
    if let Some(wait) = begin(&off, model::now_secs()) {
        return Ok(OnlineResult { status: "wait", retry_in: wait, added: 0, rows: rows(&state, &off, &q, true) });
    }
    let text = q.trim().to_string();
    let email = state.lock().unwrap().lib.settings.off_email.clone();
    let answer = tauri::async_runtime::spawn_blocking(move || off::search(&text, &email)).await.map_err(|e| e.to_string())?;
    Ok(finish(&state, &off, &q, answer, model::now_secs()))
}

/// Save, change or clear (empty) the contact email Open Food Facts is told about. The rule is `off::valid_email`; the error is its message.
#[tauri::command]
pub(crate) fn set_off_email(state: tauri::State<Shared>, email: String) -> Result<(), String> {
    set_email(&state, &email)
}

fn set_email(state: &Shared, email: &str) -> Result<(), String> {
    let value = if email.trim().is_empty() { String::new() } else { off::valid_email(email)? };
    crate::mutate(state, |st| {
        st.lib.settings.off_email = value;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_box::model::{MyFood, Nutrients};

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-desktop-foods-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn with_my_food(d: &Path) -> Shared {
        let shared = Mutex::new(crate::load_state(d.to_path_buf()));
        shared.lock().unwrap().lib.my_foods.push(MyFood { id: 7, name: "Vanilla whey".into(), serving: "1 scoop".into(), serving_g: 31.0, per_serving: Nutrients { calories: Some(120.0), ..Default::default() }, ..Default::default() });
        shared
    }

    fn hit(code: &str, name: &str) -> OnlineFood {
        OnlineFood { code: code.into(), name: name.into(), brands: "Fage".into(), per100: Nutrients { calories: Some(59.0), ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn food_commands_read_my_foods_without_saving() {
        let d = tdir("read");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();

        // "whey" alone ranks the database's plain whey first (precision); both words put the My Food on top
        let r = rows(&shared, &off, "vanilla whey", false);
        assert_eq!((r[0].key.as_str(), r[0].tag), ("my:7", "My Food"));
        assert!(rows(&shared, &off, "", false).iter().all(|x| x.tag == "My Food"));

        let egg = rows(&shared, &off, "egg", false).into_iter().find(|x| x.key.starts_with("db:")).unwrap().key;
        let mine = detail_of(&shared, &off, "my:7", false).unwrap();
        assert_eq!((mine.unit.as_str(), mine.amount), ("serving", 1.0));
        let db = detail_of(&shared, &off, &egg, false).unwrap();
        assert_eq!((db.unit.as_str(), db.amount), ("g", 100.0));
        assert_eq!(detail_of(&shared, &off, "db:nope", false).unwrap_err(), "That food isn't in the list any more.");

        assert_eq!(amount_of(&shared, &off, "my:7", Some(2.0), "serving", None).unwrap().grams, Some(62.0));
        assert_eq!(amount_of(&shared, &off, &egg, Some(100.0), "oz", Some("g")).unwrap().amount, Some(3.5));

        let f = shared.lock().unwrap().lib.my_foods[0].clone();
        assert_eq!(preview_of(&shared, &f).per100.unwrap().calories, Some(387.0));

        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn preview_reports_a_taken_name() {
        let d = tdir("taken");
        let shared = with_my_food(&d);
        let oat = MyFood { id: 8, name: "Oat milk".into(), serving: "1 cup".into(), serving_g: 240.0, per_serving: Nutrients { calories: Some(120.0), ..Default::default() }, ..Default::default() };
        shared.lock().unwrap().lib.my_foods.push(oat.clone());
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();

        let taken = preview_of(&shared, &MyFood { id: 0, name: "oat MILK".into(), ..oat.clone() }).problem.expect("a taken name blocks");
        assert_eq!((taken.field, taken.message), ("name", foods::NAME_TAKEN));
        assert!(preview_of(&shared, &oat).problem.is_none(), "a food keeps its own name");
        let empty = preview_of(&shared, &MyFood { id: 0, name: "  ".into(), ..oat.clone() }).problem.expect("an empty name blocks");
        assert_eq!(empty.field, "name");
        assert_ne!(empty.message, foods::NAME_TAKEN, "the crate's own name problem, not the taken-name one");

        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn opened_online_food_is_found_offline() {
        let d = tdir("open");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        finish(&shared, &off, "greek yogurt", off::OffAnswer::Found(vec![hit("0123", "Greek Yogurt")]), 1000);
        assert_eq!(open_online(&off, "9999").unwrap_err(), "That product isn't in the online results any more. Search online again.");
        open_online(&off, "0123").unwrap();
        let listed = |off: &OffState| rows(&shared, off, "greek yogurt", false).iter().any(|r| r.key == "off:0123" && r.tag == "Open Food Facts");
        // the hits are gone (a new query), yet the opened product is still found, and after a restart too
        off.0.lock().unwrap().hits.clear();
        assert!(listed(&off));
        assert!(listed(&OffState::new(&d)));
        assert!(!rows(&shared, &OffState::new(&tdir("open-empty")), "greek yogurt", false).iter().any(|r| r.key == "off:0123"));
        let draft = off::as_my_food(&find_online(&off, "0123").unwrap());
        assert_eq!((draft.0.name.as_str(), draft.0.serving.as_str(), draft.1), ("Fage Greek Yogurt", "100 g", true));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn online_search_respects_the_limit_and_keeps_local_results() {
        let d = tdir("limit");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        for i in 0..8 {
            assert_eq!(begin(&off, 1000 + i), None);
        }
        assert_eq!(begin(&off, 1010), Some(50), "the ninth send is refused with the seconds to wait");
        assert_eq!(begin(&off, 1060), None, "and allowed once the oldest has left the minute");

        let local = |r: &OnlineResult| r.rows.iter().any(|x| x.key == "my:7");
        let r = finish(&shared, &off, "whey", off::OffAnswer::Offline, 1100);
        assert_eq!((r.status, r.added, local(&r)), ("offline", 0, true));
        let r = finish(&shared, &off, "whey", off::OffAnswer::Slow, 1100);
        assert_eq!((r.status, local(&r)), ("slow", true));
        let r = finish(&shared, &off, "greek yogurt", off::OffAnswer::Found(vec![hit("0123", "Greek Yogurt")]), 1100);
        assert_eq!((r.status, r.added), ("ok", 1));
        let found = r.rows.iter().find(|x| x.key == "off:0123").expect("the hit joins the list");
        assert_eq!(found.tag, "Open Food Facts");

        let fresh = OffState::new(&d);
        let r = finish(&shared, &fresh, "whey", off::OffAnswer::Limited(30), 2000);
        assert_eq!((r.status, r.retry_in, local(&r)), ("limited", 30, true));
        assert_eq!(begin(&fresh, 2010), Some(20), "a hard stop refuses later sends without a request");
        assert_eq!(begin(&fresh, 2030), None);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn contact_email_is_saved_checked_and_cleared() {
        let d = tdir("email");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let saved = |s: &Shared| s.lock().unwrap().lib.settings.off_email.clone();
        assert_eq!(saved(&shared), "", "nothing is saved until the person gives one");
        let r = finish(&shared, &off, "whey", off::OffAnswer::NeedsEmail, 1000);
        assert_eq!((r.status, r.added, r.retry_in), ("needs_email", 0, 0));
        assert_eq!(begin(&off, 1000), None, "a request that never left uses no slot");

        assert!(set_email(&shared, "nope").unwrap_err().contains("email address"));
        assert!(set_email(&shared, "me@example.com\r\nX: 1").is_err());
        assert_eq!(saved(&shared), "");
        set_email(&shared, "  me@example.com ").unwrap();
        assert_eq!(saved(&shared), "me@example.com");
        assert!(set_email(&shared, "bad").is_err());
        assert_eq!(saved(&shared), "me@example.com", "a refused value changes nothing");
        set_email(&shared, "   ").unwrap();
        assert_eq!(saved(&shared), "", "clearing means the next online search asks again");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn entry_key_measures_against_the_frozen_basis() {
        let d = tdir("entrykey");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        crate::diary::log(&shared, &off, "2026-03-10", "2026-03-10", model::MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        let uid = recipe_box::tracker::diary::day_view(&mut shared.lock().unwrap().journal, "2026-03-10", "2026-03-10").unwrap().slots[1].entries[0].uid.clone();
        // the live My Food is edited afterwards; the entry keeps what it was logged with
        shared.lock().unwrap().lib.my_foods[0].per_serving.calories = Some(999.0);
        let key = format!("entry:2026-03-10:{uid}");
        let det = detail_of(&shared, &off, &key, false).unwrap();
        assert_eq!(det.units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), ["g", "oz", "serving"]);
        let m = amount_of(&shared, &off, &key, Some(62.0), "g", None).unwrap();
        assert_eq!((m.grams, m.nutrients.calories), (Some(62.0), Some(240.0)), "62 g of the frozen 120 kcal per 31 g");
        assert_eq!(detail_of(&shared, &off, "entry:nope", false).unwrap_err(), "That food isn't in the list any more.");
        assert!(detail_of(&shared, &off, "entry:2026-02-30:x", false).unwrap_err().contains("isn't a valid date"));
        assert_eq!(detail_of(&shared, &off, "entry:2026-03-10:nope", false).unwrap_err(), "That entry isn't in the diary any more.");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn recipes_are_listed_and_opened_as_foods() {
        let d = tdir("recipes");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let (uid, name) = {
            let st = shared.lock().unwrap();
            let r = st.lib.recipes.iter().find(|r| r.nutrition.is_some()).unwrap();
            (r.uid.clone(), r.name.clone())
        };
        let key = format!("recipe:{uid}");
        assert!(rows(&shared, &off, &name, false).iter().any(|r| r.key == key && r.tag == "Recipe"), "typed search lists the recipe");
        assert!(rows(&shared, &off, "", false).iter().all(|r| r.tag == "My Food"), "a blank search is still My Foods only");
        assert!(rows(&shared, &off, "egg", false).iter().any(|r| r.key.starts_with("db:")));
        let opened = detail_of(&shared, &off, &key, false).unwrap();
        assert_eq!((opened.tag, opened.unit.as_str()), ("Recipe", "serving"));
        assert_eq!(detail_of(&shared, &off, "recipe:nope", false).unwrap_err(), "That recipe isn't in the library any more.");
        shared.lock().unwrap().lib.recipes.iter_mut().for_each(|r| r.in_trash = true);
        assert!(recipe_foods(&shared.lock().unwrap().lib).is_empty(), "a recipe in the trash is not a food");
        assert!(detail_of(&shared, &off, &key, false).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
