//! The Diary page commands: log a food into a day's meal and read the day back. Every write goes
//! through `mutate_journal`, so library.json is never touched; the rules live in `recipe_box::tracker::diary`.

use crate::foods::OffState;
use crate::{mutate_journal, AppState, Shared};
use recipe_box::foods::{self, off, FoodRow};
use recipe_box::model::{self, MealType};
use recipe_box::tracker::diary::{self, DayView, PlannedMeal};
use serde::{Deserialize, Serialize};

/// The day after a log: the whole view, plus the name of the food that was added.
#[derive(Serialize)]
pub(crate) struct Logged {
    pub(crate) view: DayView,
    pub(crate) name: String,
}

/// The day after a delete or an edit: the whole view, plus the name of the entry.
#[derive(Serialize)]
pub(crate) struct Changed {
    pub(crate) view: DayView,
    pub(crate) name: String,
}

pub(crate) const NO_UNDO: &str = "Undo isn't available after another change.";

/// The day after an undo: `day` is the day the undone change was made on (the page says "Put back on {date}"); `view` is the day being viewed.
#[derive(Serialize)]
pub(crate) struct Undone {
    pub(crate) day: String,
    pub(crate) view: DayView,
}

/// After a copy or a planned-meal log: the viewed day, and how many entries were added.
#[derive(Serialize)]
pub(crate) struct Copied {
    pub(crate) view: DayView,
    pub(crate) count: usize,
}

/// One planned meal the person ticked in Log Planned Meals, and the servings they ate.
#[derive(Deserialize)]
pub(crate) struct Pick {
    meal_id: u64,
    servings: f32,
}

pub(crate) fn view(st: &mut AppState, day: &str, today: &str) -> Result<DayView, String> {
    let mut v = diary::day_view(&mut st.journal, day, today)?;
    if v.is_today {
        let n = diary::day_number(today)?;
        v.planned_count = st.lib.meals.iter().filter(|m| m.day == n).count();
    }
    Ok(v)
}

fn day_of(shared: &Shared, day: &str, today: &str) -> Result<DayView, String> {
    view(&mut shared.lock().unwrap(), day, today)
}

/// Log `amount` of `unit` of the food `key` (a database food, My Food, opened online product or recipe).
/// The food is resolved before the journal lock is taken, and nothing here reaches the network.
pub(crate) fn log(shared: &Shared, off: &OffState, day: &str, today: &str, slot: MealType, key: &str, amount: f32, unit: &str) -> Result<Logged, String> {
    let entry = diary::from_food(&crate::foods::detail_of(shared, off, key, false)?, amount, unit)?;
    let name = entry.name.clone();
    mutate_journal(shared, |st| {
        st.journal.take_undo(); // a single add ends the pending undo (D-12)
        diary::add(&mut st.journal, day, slot, entry, model::now_secs())?;
        Ok(Logged { view: view(st, day, today)?, name })
    })
}

/// A quick add: calories, optional macros and a label. The crate checks the figures before anything is written.
pub(crate) fn quick(shared: &Shared, day: &str, today: &str, slot: MealType, label: &str, calories: Option<f32>, protein: Option<f32>, carbs: Option<f32>, fat: Option<f32>) -> Result<Logged, String> {
    let entry = diary::quick(label, calories, protein, carbs, fat)?;
    let name = entry.name.clone();
    mutate_journal(shared, |st| {
        st.journal.take_undo(); // a single add ends the pending undo (D-12)
        diary::add(&mut st.journal, day, slot, entry, model::now_secs())?;
        Ok(Logged { view: view(st, day, today)?, name })
    })
}

/// What the add dialog lists: the day's planned recipes first, then recent and frequent foods for an empty box, and the ranked rows for what was typed.
#[derive(Serialize)]
pub(crate) struct DiaryFoods {
    planned: Vec<FoodRow>,
    recent: Vec<FoodRow>,
    frequent: Vec<FoodRow>,
    rows: Vec<FoodRow>,
    online_ready: bool,
    /// the query is a barcode's digits, so an empty answer offers the label photo, not a spelling hint
    barcode: bool,
    /// the barcode itself, cleaned up (a typed UPC-E as its UPC-A), or None when the digits don't add up
    code: Option<String>,
}

/// Local only: the bundled foods, My Foods, recipes and cached online products. The online products are read first so that lock is gone before `Shared` is taken.
pub(crate) fn foods_for(shared: &Shared, off: &OffState, q: &str, today: &str, day: &str, slot: MealType) -> Result<DiaryFoods, String> {
    let online = crate::foods::online_foods(off, false);
    let (mine, recipes, mut history, planned) = {
        let mut guard = shared.lock().unwrap();
        let st = &mut *guard;
        let recipes = crate::foods::recipe_foods(&st.lib);
        let history = diary::history(&mut st.journal, today, &st.lib.my_foods, &online, &recipes)?;
        let planned = diary::planned_rows(&mut st.journal, day, slot, &st.lib.meals, &recipes, |id| st.lib.recipe(id).map(|r| r.uid.clone()))?;
        (st.lib.my_foods.clone(), recipes, history, planned)
    };
    let logged: Vec<String> = history.recent.iter().chain(&history.frequent).map(|r| r.key.clone()).collect();
    // a planned row is listed once, under Planned
    history.recent.retain(|r| !planned.iter().any(|p| p.key == r.key));
    history.frequent.retain(|r| !planned.iter().any(|p| p.key == r.key));
    Ok(DiaryFoods { planned, rows: foods::search_with(q, &mine, &online, &recipes, &logged), online_ready: off::query_ready(q), barcode: off::barcode_in(q).is_some(), code: off::barcode_in(q).and_then(Result::ok), recent: history.recent, frequent: history.frequent })
}

/// One day's meals and totals. `day` and `today` are the page's local days (`YYYY-MM-DD`).
#[tauri::command]
pub(crate) fn diary_day(state: tauri::State<Shared>, day: String, today: String) -> Result<DayView, String> {
    day_of(&state, &day, &today)
}

#[tauri::command]
pub(crate) fn log_food(state: tauri::State<Shared>, off: tauri::State<OffState>, day: String, today: String, slot: MealType, key: String, amount: f32, unit: String) -> Result<Logged, String> {
    log(&state, &off, &day, &today, slot, &key, amount, &unit)
}

#[tauri::command]
pub(crate) fn quick_add(state: tauri::State<Shared>, day: String, today: String, slot: MealType, label: String, calories: Option<f32>, protein: Option<f32>, carbs: Option<f32>, fat: Option<f32>) -> Result<Logged, String> {
    quick(&state, &day, &today, slot, &label, calories, protein, carbs, fat)
}

/// The warning to show beside a very large calorie figure, or nothing. Reads and writes nothing.
#[tauri::command]
pub(crate) fn quick_add_check(calories: Option<f32>) -> Result<Option<String>, String> {
    Ok(diary::quick_add_warning(calories).map(String::from))
}

/// Delete an entry (it stays in the month file) and keep the undo.
pub(crate) fn remove(shared: &Shared, day: &str, today: &str, uid: &str) -> Result<Changed, String> {
    mutate_journal(shared, |st| {
        let (name, u) = diary::delete(&mut st.journal, day, uid, model::now_secs())?;
        st.journal.set_undo(u);
        Ok(Changed { view: view(st, day, today)?, name })
    })
}

/// Put back the last change. When the save fails the rollback gives the slot back, so it can be tried again.
pub(crate) fn undo_last(shared: &Shared, day: &str, today: &str) -> Result<Undone, String> {
    mutate_journal(shared, |st| {
        let u = st.journal.take_undo().ok_or(NO_UNDO)?;
        diary::undo(&mut st.journal, &u, model::now_secs())?;
        Ok(Undone { day: u.day, view: view(st, day, today)? })
    })
}

/// Change an entry's amount, unit and meal against its frozen basis, and keep the undo.
pub(crate) fn change(shared: &Shared, day: &str, today: &str, uid: &str, amount: f32, unit: &str, slot: MealType) -> Result<Changed, String> {
    mutate_journal(shared, |st| {
        let (name, u) = diary::edit(&mut st.journal, day, uid, amount, unit, slot, model::now_secs())?;
        st.journal.set_undo(u);
        Ok(Changed { view: view(st, day, today)?, name })
    })
}

/// Copy a day, or one meal of it, into `day` as new entries, and keep the undo. Nothing already in `day` is touched.
pub(crate) fn copy_in(shared: &Shared, from: &str, day: &str, today: &str, slot: Option<MealType>) -> Result<Copied, String> {
    mutate_journal(shared, |st| {
        let (count, u) = diary::copy(&mut st.journal, from, day, slot, model::now_secs())?;
        st.journal.set_undo(u);
        Ok(Copied { view: view(st, day, today)?, count })
    })
}

/// Mark a day complete or not. The undo slot is left alone.
pub(crate) fn mark_complete(shared: &Shared, day: &str, today: &str, complete: bool) -> Result<DayView, String> {
    mutate_journal(shared, |st| {
        diary::set_complete(&mut st.journal, day, complete)?;
        view(st, day, today)
    })
}

/// The meals planned for today. A recipe in the trash counts as gone.
pub(crate) fn planned_for(shared: &Shared, today: &str) -> Result<Vec<PlannedMeal>, String> {
    let st = shared.lock().unwrap();
    let lookup = |id: u64| st.lib.recipe(id).filter(|r| !r.in_trash).map(|r| (r.name.clone(), recipe_box::health::per_serving(r)));
    Ok(diary::planned(&st.lib.meals, diary::day_number(today)?, lookup))
}

/// Log the picked planned meals into today in one batch (D-10). Each meal's recipe is resolved to its uid now and its per-serving values
/// are frozen through the same recipe detail the add dialog uses. One undo step; any refused pick writes nothing.
pub(crate) fn log_planned_in(shared: &Shared, today: &str, picks: &[Pick]) -> Result<Copied, String> {
    mutate_journal(shared, |st| {
        if picks.is_empty() {
            return Err("Pick at least one meal to log.".into());
        }
        let n = diary::day_number(today)?;
        let mut items = vec![];
        for (i, p) in picks.iter().enumerate() {
            if picks[..i].iter().any(|q| q.meal_id == p.meal_id) {
                return Err("Pick each meal only once.".into());
            }
            let meal = st.lib.meals.iter().find(|m| m.id == p.meal_id && m.day == n).ok_or("That planned meal isn't in the plan any more.")?;
            let recipe = st.lib.recipe(meal.recipe_id.ok_or("No recipe linked. Add it by hand.")?).filter(|r| !r.in_trash && !r.uid.is_empty()).ok_or(crate::GONE)?;
            let detail = crate::foods::recipe_detail_in(&st.lib, &recipe.uid, false)?;
            items.push((meal.kind, diary::from_food(&detail, p.servings, "serving")?));
        }
        let count = items.len();
        let u = diary::add_all(&mut st.journal, today, items, model::now_secs())?;
        st.journal.set_undo(u);
        Ok(Copied { view: view(st, today, today)?, count })
    })
}

#[tauri::command]
pub(crate) fn copy_entries(state: tauri::State<Shared>, from: String, day: String, today: String, slot: Option<MealType>) -> Result<Copied, String> {
    copy_in(&state, &from, &day, &today, slot)
}

#[tauri::command]
pub(crate) fn set_day_complete(state: tauri::State<Shared>, day: String, today: String, complete: bool) -> Result<DayView, String> {
    mark_complete(&state, &day, &today, complete)
}

#[tauri::command]
pub(crate) fn planned_meals(state: tauri::State<Shared>, today: String) -> Result<Vec<PlannedMeal>, String> {
    planned_for(&state, &today)
}

#[tauri::command]
pub(crate) fn log_planned(state: tauri::State<Shared>, today: String, picks: Vec<Pick>) -> Result<Copied, String> {
    log_planned_in(&state, &today, &picks)
}

#[tauri::command]
pub(crate) fn edit_entry(state: tauri::State<Shared>, day: String, today: String, uid: String, amount: f32, unit: String, slot: MealType) -> Result<Changed, String> {
    change(&state, &day, &today, &uid, amount, &unit, slot)
}

#[tauri::command]
pub(crate) fn delete_entry(state: tauri::State<Shared>, day: String, today: String, uid: String) -> Result<Changed, String> {
    remove(&state, &day, &today, &uid)
}

#[tauri::command]
pub(crate) fn undo_diary(state: tauri::State<Shared>, day: String, today: String) -> Result<Undone, String> {
    undo_last(&state, &day, &today)
}

#[tauri::command]
pub(crate) fn diary_foods(state: tauri::State<Shared>, off: tauri::State<OffState>, q: String, today: String, day: String, slot: MealType) -> Result<DiaryFoods, String> {
    foods_for(&state, &off, &q, &today, &day, slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_box::foods;
    use recipe_box::model::{MyFood, Nutrients};
    use std::path::Path;
    use std::sync::Mutex;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-desktop-diary-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn with_my_food(d: &Path) -> Shared {
        let shared = Mutex::new(crate::load_state(d.to_path_buf()));
        shared.lock().unwrap().lib.my_foods.push(MyFood { id: 7, name: "Vanilla whey".into(), serving: "1 scoop".into(), serving_g: 31.0, per_serving: Nutrients { calories: Some(120.0), ..Default::default() }, ..Default::default() });
        shared
    }

    #[test]
    fn logging_a_food_writes_only_the_journal() {
        let d = tdir("food");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();
        let egg = foods::search("egg", &[], &[]).into_iter().find(|r| r.key.starts_with("db:")).unwrap().key;

        let a = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Breakfast, &egg, 100.0, "g").unwrap();
        let b = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Snack, "my:7", 2.0, "serving").unwrap();
        assert_eq!(a.view.slots[0].entries.len(), 1);
        assert_eq!((b.name.as_str(), b.view.slots[3].entries[0].calories), ("Vanilla whey", Some(240.0)));
        assert_eq!(b.view.slots[0].entries.len(), 1, "the view holds both entries");

        assert!(d.join("journal").join("2026-03.json").exists());
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        assert_eq!(day_of(&shared, "2026-03-11", "2026-03-10").unwrap().slots.iter().map(|s| s.entries.len()).sum::<usize>(), 0);
        assert_eq!(log(&shared, &off, "2026-02-30", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").err().map(|e| e.contains("isn't a valid date")), Some(true));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn logging_a_recipe_freezes_its_serving() {
        let d = tdir("recipe");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let (uid, per) = {
            let st = shared.lock().unwrap();
            let r = st.lib.recipes.iter().find(|r| r.nutrition.is_some_and(|n| n.calories.is_some())).unwrap();
            (r.uid.clone(), recipe_box::health::per_serving(r).unwrap())
        };
        let key = format!("recipe:{uid}");
        let a = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Dinner, &key, 1.5, "serving").unwrap();
        let expect = per.calories.unwrap() * 1.5;
        assert!((a.view.totals.calories.unwrap() - expect).abs() <= 1.0, "{:?} vs {expect}", a.view.totals.calories);
        assert_eq!(a.view.slots[2].entries[0].tag, "Recipe");
        // the recipe changes afterwards; the day does not
        shared.lock().unwrap().lib.recipes.iter_mut().for_each(|r| r.nutrition = Some(Nutrients { calories: Some(99999.0), ..Default::default() }));
        assert_eq!(day_of(&shared, "2026-03-10", "2026-03-10").unwrap().totals, a.view.totals);
        assert_eq!(log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Dinner, "recipe:nope", 1.0, "serving").err().as_deref(), Some("That recipe isn't in the library any more."));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn quick_add_writes_only_the_journal() {
        let d = tdir("quick");
        let shared = with_my_food(&d);
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();
        let a = quick(&shared, "2026-03-10", "2026-03-10", MealType::Snack, "", Some(250.0), None, Some(30.0), None).unwrap();
        assert_eq!((a.name.as_str(), a.view.slots[3].entries[0].tag, a.view.totals.protein, a.view.totals.calories), ("Quick Add", "Quick Add", None, Some(250.0)));
        assert!(quick(&shared, "2026-03-10", "2026-03-10", MealType::Snack, "x", None, None, None, None).is_err());
        assert_eq!(quick_add_check(Some(3500.0)).unwrap().as_deref(), Some("That is a lot of calories for one entry. Check the number."));
        assert_eq!(quick_add_check(None).unwrap(), None);
        assert!(d.join("journal").join("2026-03.json").exists());
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn lib_bytes(d: &Path) -> Vec<u8> {
        std::fs::read(d.join("library.json")).unwrap()
    }

    fn lunch(shared: &Shared, day: &str) -> Vec<String> {
        day_of(shared, day, day).unwrap().slots[1].entries.iter().map(|e| e.uid.clone()).collect()
    }

    #[test]
    fn delete_then_undo_puts_the_entry_back() {
        let d = tdir("delete");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let lib_before = lib_bytes(&d);
        let a = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 2.0, "serving").unwrap();
        let uid = a.view.slots[1].entries[0].uid.clone();
        let c = remove(&shared, "2026-03-10", "2026-03-10", &uid).unwrap();
        assert_eq!((c.name.as_str(), c.view.slots[1].entries.len()), ("Vanilla whey", 1));
        // undone while another day is viewed: the answer names the day it changed and carries the viewed day
        let u = undo_last(&shared, "2026-03-11", "2026-03-10").unwrap();
        assert_eq!((u.day.as_str(), u.view.day.as_str()), ("2026-03-10", "2026-03-11"));
        let ids = lunch(&shared, "2026-03-10");
        assert!(ids.len() == 2 && ids.contains(&uid));
        assert_eq!(undo_last(&shared, "2026-03-10", "2026-03-10").err().as_deref(), Some("Undo isn't available after another change."));
        assert_eq!(lib_bytes(&d), lib_before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn edit_then_undo_restores_amount_and_meal() {
        let d = tdir("edit");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let a = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        let uid = a.view.slots[1].entries[0].uid.clone();
        let c = change(&shared, "2026-03-10", "2026-03-10", &uid, 2.0, "serving", MealType::Dinner).unwrap();
        assert_eq!((c.view.slots[1].entries.len(), c.view.slots[2].entries[0].calories), (0, Some(240.0)));
        assert_eq!(change(&shared, "2026-03-10", "2026-03-10", &uid, 0.0, "serving", MealType::Dinner).err().as_deref(), Some("Enter an amount above 0."));
        let u = undo_last(&shared, "2026-03-10", "2026-03-10").unwrap();
        assert_eq!((u.view.slots[1].entries[0].calories, u.view.slots[1].entries[0].amount_text.as_str(), u.view.slots[2].entries.len()), (Some(120.0), "1 serving (31 g)", 0));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_then_undo_removes_only_the_copies() {
        let d = tdir("copy");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let lib_before = lib_bytes(&d);
        log(&shared, &off, "2026-03-14", "2026-03-15", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        log(&shared, &off, "2026-03-15", "2026-03-15", MealType::Lunch, "my:7", 2.0, "serving").unwrap();
        let before = lunch(&shared, "2026-03-15");
        let c = copy_in(&shared, "2026-03-14", "2026-03-15", "2026-03-15", None).unwrap();
        assert_eq!((c.count, c.view.slots[1].entries.len(), c.view.previous_day_count), (1, 2, 1));
        assert_eq!(lunch(&shared, "2026-03-15")[0], before[0], "what was there stays first and untouched");
        assert_eq!(copy_in(&shared, "2026-03-15", "2026-03-15", "2026-03-15", None).err().as_deref(), Some("Pick another day to copy from."));
        undo_last(&shared, "2026-03-15", "2026-03-15").unwrap();
        assert_eq!(lunch(&shared, "2026-03-15"), before);
        assert_eq!(lunch(&shared, "2026-03-14").len(), 1);
        assert_eq!(lib_bytes(&d), lib_before);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Two recipes with a calorie estimate, planned today as lunch and dinner, and a name-only meal as a snack. Planted in memory only.
    fn plan_today(shared: &Shared, today: &str) -> (recipe_box::model::Recipe, recipe_box::model::Recipe) {
        let mut st = shared.lock().unwrap();
        let mut found = st.lib.recipes.iter().filter(|r| r.nutrition.is_some_and(|n| n.calories.is_some()) && !r.uid.is_empty()).cloned();
        let (a, b) = (found.next().unwrap(), found.next().unwrap());
        let day = diary::day_number(today).unwrap();
        let meal = |id, kind, recipe_id: Option<u64>, name: &str| model::Meal { id, day, kind, recipe_id, name: name.into(), ..Default::default() };
        st.lib.meals = vec![meal(101, MealType::Lunch, Some(a.id), ""), meal(102, MealType::Dinner, Some(b.id), ""), meal(103, MealType::Snack, None, "Fruit")];
        (a, b)
    }

    #[test]
    fn log_planned_is_one_undo_step() {
        let d = tdir("planned");
        let shared = with_my_food(&d);
        let (a, b) = plan_today(&shared, "2026-03-10");
        let lib_before = lib_bytes(&d);
        let listed = planned_for(&shared, "2026-03-10").unwrap();
        assert_eq!(listed.iter().map(|p| (p.meal_id, p.loggable)).collect::<Vec<_>>(), [(101, true), (102, true), (103, false)]);
        assert_eq!(day_of(&shared, "2026-03-10", "2026-03-10").unwrap().planned_count, 3);
        assert_eq!(day_of(&shared, "2026-03-11", "2026-03-10").unwrap().planned_count, 0);

        let r = log_planned_in(&shared, "2026-03-10", &[Pick { meal_id: 101, servings: 1.0 }, Pick { meal_id: 102, servings: 1.5 }]).unwrap();
        assert_eq!((r.count, r.view.slots[1].entries.len(), r.view.slots[2].entries.len(), r.view.slots[3].entries.len()), (2, 1, 1, 0));
        let sources: Vec<_> = shared.lock().unwrap().journal.month("2026-03").unwrap().entries.iter().map(|e| e.source.clone()).collect();
        assert_eq!(sources, [recipe_box::tracker::journal::FoodRef::Recipe(a.uid.clone()), recipe_box::tracker::journal::FoodRef::Recipe(b.uid.clone())]);
        undo_last(&shared, "2026-03-10", "2026-03-10").unwrap();
        assert_eq!(day_of(&shared, "2026-03-10", "2026-03-10").unwrap().slots.iter().map(|s| s.entries.len()).sum::<usize>(), 0, "one undo removes both");
        assert_eq!(lib_bytes(&d), lib_before);
        let _ = std::fs::remove_dir_all(&d);
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
            let planner = crate::overview_of(&shared.lock().unwrap(), local.as_str()).unwrap().today;
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
        let err = crate::overview_of(&shared.lock().unwrap(), "2026-02-30").err().unwrap();
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
        let planner = crate::overview_of(&shared.lock().unwrap(), "2026-11-13").unwrap().today;
        assert_ne!(planner, old_today);
        assert_eq!(shared.lock().unwrap().lib.meals[0].day, old_today, "the stored day is untouched");
        assert!(planned_for(&shared, "2026-11-13").unwrap().is_empty());
        assert_eq!(planned_for(&shared, "2026-11-14").unwrap().iter().map(|p| p.meal_id).collect::<Vec<_>>(), [301]);
        assert_eq!(lib_bytes(&d), lib_before, "reading the planner and the diary writes nothing");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn log_planned_refuses_bad_picks() {
        let d = tdir("planned-bad");
        let shared = with_my_food(&d);
        plan_today(&shared, "2026-03-10");
        let pick = |meal_id, servings| Pick { meal_id, servings };
        let said = |picks: &[Pick]| log_planned_in(&shared, "2026-03-10", picks).err();
        assert_eq!(said(&[]).as_deref(), Some("Pick at least one meal to log."));
        assert_eq!(said(&[pick(101, 1.0), pick(999, 1.0)]).as_deref(), Some("That planned meal isn't in the plan any more."));
        assert_eq!(said(&[pick(101, 1.0), pick(101, 1.0)]).as_deref(), Some("Pick each meal only once."));
        assert_eq!(said(&[pick(101, 1.0), pick(102, 0.0)]).as_deref(), Some("Enter an amount above 0."));
        assert_eq!(said(&[pick(103, 1.0)]).as_deref(), Some("No recipe linked. Add it by hand."));
        shared.lock().unwrap().lib.recipes.iter_mut().for_each(|r| r.in_trash = true);
        assert_eq!(said(&[pick(101, 1.0)]).as_deref(), Some(crate::GONE));
        assert_eq!(day_of(&shared, "2026-03-10", "2026-03-10").unwrap().slots.iter().map(|s| s.entries.len()).sum::<usize>(), 0, "every refusal wrote nothing");
        assert!(!d.join("journal").join("2026-03.json").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn complete_toggle_writes_only_the_journal() {
        let d = tdir("complete");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let lib_before = lib_bytes(&d);
        let snaps_before = shared.lock().unwrap().store.snapshots().len();
        let a = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        remove(&shared, "2026-03-10", "2026-03-10", &a.view.slots[1].entries[0].uid).unwrap();
        let v = mark_complete(&shared, "2026-03-10", "2026-03-10", true).unwrap();
        assert!(v.complete);
        assert!(std::fs::read_to_string(d.join("journal").join("2026-03.json")).unwrap().contains("\"complete\""));
        assert!(!mark_complete(&shared, "2026-03-10", "2026-03-10", false).unwrap().complete);
        assert_eq!(undo_last(&shared, "2026-03-10", "2026-03-10").unwrap().view.slots[1].entries.len(), 1, "the toggle leaves the undo alone");
        assert_eq!(lib_bytes(&d), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_refused_change_keeps_the_pending_undo() {
        let d = tdir("refused");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let a = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        let uid = a.view.slots[1].entries[0].uid.clone();
        remove(&shared, "2026-03-10", "2026-03-10", &uid).unwrap();
        assert_eq!(remove(&shared, "2026-03-10", "2026-03-10", "nope").err().as_deref(), Some("That entry isn't in the diary any more."));
        assert_eq!(undo_last(&shared, "2026-03-10", "2026-03-10").unwrap().view.slots[1].entries.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn failed_write_leaves_the_day_as_saved() {
        let d = tdir("failed");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        let bad = diary::NewEntry { name: "Bad".into(), source: recipe_box::tracker::journal::FoodRef::Custom, grams: None, amount: f32::INFINITY, unit: "serving".into(), basis: Nutrients::default(), per_100g: false };
        let r = mutate_journal(&shared, |st| {
            diary::add(&mut st.journal, "2026-03-10", MealType::Lunch, bad, 1)?;
            Ok(())
        });
        assert!(r.is_err());
        assert_eq!(lunch(&shared, "2026-03-10").len(), 1, "the refused entry is not in the day");
        // and a later good change does not write it either
        log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        assert_eq!(lunch(&shared, "2026-03-10").len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn an_add_clears_the_pending_undo() {
        let d = tdir("addclears");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let a = log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        let uid = a.view.slots[1].entries[0].uid.clone();
        remove(&shared, "2026-03-10", "2026-03-10", &uid).unwrap();
        log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, "my:7", 1.0, "serving").unwrap();
        assert_eq!(undo_last(&shared, "2026-03-10", "2026-03-10").err().as_deref(), Some("Undo isn't available after another change."));
        assert_eq!(lunch(&shared, "2026-03-10").len(), 1, "the deleted entry stays deleted");
        remove(&shared, "2026-03-10", "2026-03-10", &lunch(&shared, "2026-03-10")[0]).unwrap();
        quick(&shared, "2026-03-10", "2026-03-10", MealType::Snack, "x", Some(10.0), None, None, None).unwrap();
        assert!(undo_last(&shared, "2026-03-10", "2026-03-10").is_err(), "a quick add clears it too");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn diary_foods_lists_history_and_ranked_rows() {
        let d = tdir("foods");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let egg = foods::search("egg", &[], &[]).into_iter().find(|r| r.key.starts_with("db:")).unwrap().key;
        log(&shared, &off, "2026-03-09", "2026-03-10", MealType::Breakfast, &egg, 100.0, "g").unwrap();
        log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Snack, "my:7", 1.0, "serving").unwrap();
        quick(&shared, "2026-03-10", "2026-03-10", MealType::Snack, "Cake", Some(300.0), None, None, None).unwrap();

        let f = foods_for(&shared, &off, "", "2026-03-10", "2026-03-10", MealType::Snack).unwrap();
        assert!(f.planned.is_empty());
        assert_eq!(f.recent.iter().map(|r| r.key.clone()).collect::<Vec<_>>(), vec!["my:7".to_string(), egg.clone()]);
        assert!(f.frequent.is_empty(), "everything frequent is already in recent");
        assert!(f.rows.iter().all(|r| r.tag == "My Food"), "an empty box lists My Foods");
        assert!(!f.online_ready);
        let f = foods_for(&shared, &off, "egg", "2026-03-10", "2026-03-10", MealType::Snack).unwrap();
        assert_eq!(f.rows[0].key, egg, "a food logged before leads its ties");
        assert!(f.online_ready);
        assert!(!f.barcode);
        // a barcode's digits are flagged, whether or not they add up, so the empty answer offers the label photo
        assert!(foods_for(&shared, &off, "815074022915", "2026-03-10", "2026-03-10", MealType::Snack).unwrap().barcode);
        let bad = foods_for(&shared, &off, "815074022914", "2026-03-10", "2026-03-10", MealType::Snack).unwrap();
        assert!(bad.barcode && bad.code.is_none(), "digits that don't add up are never kept with a food");
        // a typed UPC-E is kept as the UPC-A a scan of the same pack reads
        assert_eq!(foods_for(&shared, &off, "04252614", "2026-03-10", "2026-03-10", MealType::Snack).unwrap().code.as_deref(), Some("042100005264"));
        assert!(foods_for(&shared, &off, "egg", "nope", "2026-03-10", MealType::Snack).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn diary_foods_lead_with_the_days_plan() {
        let d = tdir("foods-plan");
        let shared = with_my_food(&d);
        let off = OffState::new(&d);
        let (a, b) = plan_today(&shared, "2026-03-10");
        let (ka, kb) = (format!("recipe:{}", a.uid), format!("recipe:{}", b.uid));
        let lib_before = lib_bytes(&d);
        log(&shared, &off, "2026-03-09", "2026-03-10", MealType::Dinner, &kb, 1.0, "serving").unwrap();
        let keys = |rows: &[FoodRow]| rows.iter().map(|r| r.key.clone()).collect::<Vec<_>>();
        let f = foods_for(&shared, &off, "", "2026-03-10", "2026-03-10", MealType::Dinner).unwrap();
        assert_eq!(keys(&f.planned), [kb.clone(), ka.clone()]);
        assert!(!keys(&f.recent).contains(&kb), "a planned row is not repeated under recent");
        log(&shared, &off, "2026-03-10", "2026-03-10", MealType::Lunch, &ka, 1.0, "serving").unwrap();
        let f = foods_for(&shared, &off, "", "2026-03-10", "2026-03-10", MealType::Dinner).unwrap();
        assert_eq!(keys(&f.planned), [kb], "an eaten meal is not offered");
        let f = foods_for(&shared, &off, "", "2026-03-10", "2026-05-01", MealType::Dinner).unwrap();
        assert!(f.planned.is_empty());
        assert!(!d.join("journal").join("2026-05.json").exists(), "reading never creates a month");
        assert_eq!(lib_bytes(&d), lib_before);
        let _ = std::fs::remove_dir_all(&d);
    }
}
