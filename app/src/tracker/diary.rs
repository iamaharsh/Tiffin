//! The diary: foods logged into a day's meal, kept in the journal month files. An entry freezes its
//! nutrition at log time and totals are computed on read, so editing a food or recipe later never
//! changes a day that is already logged. The day always comes from the shell (see `day.rs`).

use crate::foods::{self, FoodRow, OnlineFood, RecipeFood};
use crate::model::{civil_from_days, days_from_civil, Meal, MealType, MyFood, Nutrients};
use crate::tracker::day::Day;
use crate::tracker::journal::{BodyEntry, DiaryEntry, DiaryUndo, ExerciseBasis, ExerciseEntry, ExerciseSource, FoodRef, Journal};
use crate::tracker::{body, exercise, goals};
use serde::Serialize;
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

pub const BAD_AMOUNT: &str = "Enter an amount above 0.";
pub const NOT_THERE: &str = "That entry isn't in the diary any more.";

/// A fresh entry id: 32 hex characters, so two devices never mint the same one.
pub fn new_uid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let part = || {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(nanos);
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.finish()
    };
    // shortcut: ids come from std's randomly keyed hashers (128 bits), not a CSPRNG; fine for unique record ids, switch to getrandom if an id ever has to be secret
    format!("{:016x}{:016x}", part(), part())
}

/// What the diary needs to log one food: its source, name, how much was eaten and the nutrition it was logged with.
#[derive(Clone, Debug, PartialEq)]
pub struct NewEntry {
    pub name: String,
    pub source: FoodRef,
    pub grams: Option<f32>,
    pub amount: f32,
    pub unit: String,
    pub basis: Nutrients,
    pub per_100g: bool,
}

/// An entry for `amount` of `unit` of the opened food `d`, with the food's nutrition frozen into it.
pub fn from_food(d: &foods::FoodDetail, amount: f32, unit: &str) -> Result<NewEntry, String> {
    if !amount.is_finite() || amount <= 0.0 {
        return Err(BAD_AMOUNT.into());
    }
    let m = foods::amount(d, Some(amount), unit, None)?;
    let (kind, id) = d.key.split_once(':').ok_or(foods::GONE)?;
    let source = match kind {
        "db" => FoodRef::Bundled(id.into()),
        // a My Food is known by its name, as the library's own sync knows it
        "my" => FoodRef::MyFood(d.name.trim().into()),
        "off" => FoodRef::Off(id.into()),
        "recipe" => FoodRef::Recipe(id.into()),
        _ => return Err(foods::GONE.into()),
    };
    // a unit with no weight (a recipe serving of unknown size): the basis is one serving and the entry counts servings
    let weightless = d.units.iter().find(|u| u.id == unit).is_some_and(|u| u.grams <= 0.0);
    let (basis, per_100g) = match d.per_serving.filter(|_| weightless) {
        Some(per_serving) => (per_serving, false),
        None => (d.per100, true),
    };
    Ok(NewEntry { name: d.name.clone(), source, grams: m.grams, amount, unit: unit.into(), basis, per_100g })
}

/// More calories than this in one entry gets a non-blocking warning (a typo such as 5000 for 500).
const QUICK_ADD_LOTS: f32 = 3000.0;

/// A quick add: calories (required) and optional protein, carbs and fat. A blank figure is unknown, never 0.
pub fn quick(label: &str, calories: Option<f32>, protein: Option<f32>, carbs: Option<f32>, fat: Option<f32>) -> Result<NewEntry, String> {
    let calories = calories.ok_or("Enter the calories for this entry.")?;
    if [Some(calories), protein, carbs, fat].into_iter().flatten().any(|v| !v.is_finite() || v < 0.0) {
        return Err("Nutrition numbers can't be negative.".into());
    }
    let label: String = label.trim().chars().take(80).collect();
    let name = if label.trim().is_empty() { "Quick Add".to_string() } else { label.trim().to_string() };
    let basis = Nutrients { calories: Some(calories), protein_g: protein, carbs_g: carbs, fat_g: fat, ..Default::default() };
    Ok(NewEntry { name, source: FoodRef::Custom, grams: None, amount: 1.0, unit: "serving".into(), basis, per_100g: false })
}

/// The sentence to show beside a very large calorie figure; it never blocks the add.
pub fn quick_add_warning(calories: Option<f32>) -> Option<&'static str> {
    calories.filter(|c| *c > QUICK_ADD_LOTS).map(|_| "That is a lot of calories for one entry. Check the number.")
}

/// Put `e` into `day`'s month and return its uid. The day is checked before any month is touched.
pub fn add(j: &mut Journal, day: &str, slot: MealType, e: NewEntry, now: u64) -> Result<String, String> {
    put(j, &Day::parse(day)?, slot, e, now)
}

fn put(j: &mut Journal, d: &Day, slot: MealType, e: NewEntry, at: u64) -> Result<String, String> {
    let uid = new_uid();
    let entry = DiaryEntry { uid: uid.clone(), day: d.as_str().into(), slot, logged_at: at, modified: at, deleted_at: None, name: e.name, source: e.source, grams: e.grams, amount: e.amount, unit: e.unit, basis: e.basis, per_100g: e.per_100g };
    j.month_mut(d.month())?.entries.push(entry);
    Ok(uid)
}

/// Add several entries to one day in one step (a planned-meal batch). `logged_at` rises by item so the log order holds.
/// The day is checked first; the undo removes exactly what was added.
pub fn add_all(j: &mut Journal, day: &str, items: Vec<(MealType, NewEntry)>, now: u64) -> Result<DiaryUndo, String> {
    let d = Day::parse(day)?;
    let mut added = vec![];
    for (i, (slot, e)) in items.into_iter().enumerate() {
        added.push(put(j, &d, slot, e, now + i as u64)?);
    }
    Ok(DiaryUndo { day: d.as_str().into(), added, ..Default::default() })
}

/// Copy the live entries of `from` (only meal `slot` when given; a whole day also carries its workouts) into the same meals of `to`, as new entries with the same frozen values.
/// Nothing that is already in `to` is changed. Returns how many were copied, and the undo that removes exactly those.
pub fn copy(j: &mut Journal, from: &str, to: &str, slot: Option<MealType>, now: u64) -> Result<(usize, DiaryUndo), String> {
    let (f, t) = (Day::parse(from)?, Day::parse(to)?);
    if f == t {
        return Err("Pick another day to copy from.".into());
    }
    let source: Vec<DiaryEntry> = j.month(f.month())?.entries.iter().filter(|e| e.deleted_at.is_none() && e.day == f.as_str() && slot.map_or(true, |s| e.slot == s)).cloned().collect();
    // a whole day carries its workouts too; one meal has none (exercise has no meal)
    let workouts: Vec<ExerciseEntry> = if slot.is_none() { j.month(f.month())?.exercise.iter().filter(|e| e.deleted_at.is_none() && e.day == f.as_str()).cloned().collect() } else { vec![] };
    if source.is_empty() && workouts.is_empty() {
        return Err("There's nothing to copy from that day.".into());
    }
    let copies: Vec<DiaryEntry> = source.into_iter().map(|e| DiaryEntry { uid: new_uid(), day: t.as_str().into(), logged_at: now, modified: now, deleted_at: None, ..e }).collect();
    let added: Vec<String> = copies.iter().map(|e| e.uid.clone()).collect();
    let workout_copies: Vec<ExerciseEntry> = workouts.into_iter().map(|e| ExerciseEntry { uid: new_uid(), day: t.as_str().into(), logged_at: now, modified: now, deleted_at: None, ..e }).collect();
    let added_exercise: Vec<String> = workout_copies.iter().map(|e| e.uid.clone()).collect();
    let month = j.month_mut(t.month())?;
    month.entries.extend(copies);
    month.exercise.extend(workout_copies);
    Ok((added.len() + added_exercise.len(), DiaryUndo { day: t.as_str().into(), added, added_exercise, ..Default::default() }))
}

/// Mark a day complete or not. The day stays editable. A toggle has no undo.
pub fn set_complete(j: &mut Journal, day: &str, on: bool) -> Result<(), String> {
    let d = Day::parse(day)?;
    let month = j.month_mut(d.month())?;
    if on {
        month.complete.insert(d.as_str().into());
    } else {
        month.complete.remove(d.as_str());
    }
    Ok(())
}

/// A day as the number `Meal.day` counts in.
pub fn day_number(day: &str) -> Result<i32, String> {
    let d = Day::parse(day)?;
    let s = d.as_str();
    Ok(days_from_civil(s[..4].parse().unwrap_or(1970), s[5..7].parse().unwrap_or(1), s[8..].parse().unwrap_or(1)))
}

/// One meal planned for the day, as the Log Planned Meals dialog lists it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlannedMeal {
    pub meal_id: u64,
    pub name: String,
    pub slot: MealType,
    pub slot_label: &'static str,
    pub kcal: Option<f32>,
    /// "520 kcal", when known.
    pub kcal_text: Option<String>,
    pub loggable: bool,
    pub note: Option<&'static str>,
}

/// The meals planned for `day` in meal order, then by id. `recipe` gives a recipe's name and per-serving values by id (None when it is gone).
/// A meal with no recipe or a missing one can't be logged; a recipe with no estimate can, with every value unknown.
pub fn planned(meals: &[Meal], day: i32, recipe: impl Fn(u64) -> Option<(String, Option<Nutrients>)>) -> Vec<PlannedMeal> {
    let mut mine: Vec<&Meal> = meals.iter().filter(|m| m.day == day).collect();
    mine.sort_by_key(|m| (MealType::ALL.iter().position(|s| *s == m.kind), m.id));
    mine.into_iter()
        .map(|m| {
            let (name, kcal, loggable, note) = match m.recipe_id.map(|id| recipe(id)) {
                None => (m.name.clone(), None, false, Some("No recipe linked. Add it by hand.")),
                Some(None) => (m.name.clone(), None, false, Some("This recipe isn't in your library any more.")),
                Some(Some((name, None))) => (name, None, true, Some("No nutrition estimate yet. It will be logged with values unknown.")),
                Some(Some((name, Some(per)))) => (name, foods::display(&per).calories, true, None),
            };
            PlannedMeal { meal_id: m.id, name, slot: m.kind, slot_label: slot_label(m.kind), kcal_text: kcal.map(|k| format!("{} kcal", goals::kcal_text(k.round() as i32))), kcal, loggable, note }
        })
        .collect()
}

/// `Meal.day` back as `YYYY-MM-DD`, formatted like `shift`.
fn day_text(n: i32) -> String {
    let (y, m, d) = civil_from_days(n);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The ids of the planned meals the diary confirms were eaten. Nothing is stored for this: a meal counts as eaten when its day has a live
/// entry whose source is that meal's recipe. Each entry confirms one meal (so a recipe planned twice and logged once gets one check),
/// the same slot is matched first, then any slot. Reads months only (never marks one dirty); a month that won't open confirms nothing.
pub fn eaten(j: &mut Journal, meals: &[&Meal], uid_of: impl Fn(u64) -> Option<String>) -> Vec<u64> {
    let mut days: Vec<i32> = meals.iter().map(|m| m.day).collect();
    days.sort_unstable();
    days.dedup();
    let mut out = vec![];
    for day in days {
        let text = day_text(day);
        let Ok(month) = j.month(&text[..7]) else { continue };
        let mut pool: Vec<(String, MealType)> = month
            .entries
            .iter()
            .filter(|e| e.deleted_at.is_none() && e.day == text)
            .filter_map(|e| if let FoodRef::Recipe(uid) = &e.source { Some((uid.clone(), e.slot)) } else { None })
            .collect();
        let mut mine: Vec<(&Meal, String)> = meals.iter().filter(|m| m.day == day).filter_map(|m| Some((*m, uid_of(m.recipe_id?)?))).collect();
        mine.sort_by_key(|(m, _)| (MealType::ALL.iter().position(|s| *s == m.kind), m.id));
        let mut done = vec![false; mine.len()];
        for same_slot in [true, false] {
            for (i, (m, uid)) in mine.iter().enumerate() {
                if done[i] {
                    continue;
                }
                if let Some(p) = pool.iter().position(|(u, s)| u == uid && (!same_slot || *s == m.kind)) {
                    pool.remove(p);
                    done[i] = true;
                    out.push(m.id);
                }
            }
        }
    }
    out.sort_unstable();
    out
}

/// Food rows for the recipes planned on `day`, not yet eaten, for the add dialog to offer first: the meal being added to (`slot`) first,
/// then the rest of the day in meal order. Recipes that are gone or trashed (absent from `recipes`) drop out; each food appears once.
pub fn planned_rows(j: &mut Journal, day: &str, slot: MealType, meals: &[Meal], recipes: &[RecipeFood], uid_of: impl Fn(u64) -> Option<String>) -> Result<Vec<FoodRow>, String> {
    let n = day_number(day)?;
    let todays: Vec<&Meal> = meals.iter().filter(|m| m.day == n).collect();
    let eaten = eaten(j, &todays, &uid_of);
    let mut left: Vec<&Meal> = todays.into_iter().filter(|m| !eaten.contains(&m.id)).collect();
    left.sort_by_key(|m| (m.kind != slot, MealType::ALL.iter().position(|s| *s == m.kind), m.id));
    let mut rows: Vec<FoodRow> = vec![];
    for m in left {
        let Some(uid) = m.recipe_id.and_then(&uid_of) else { continue };
        let Some(row) = recipes.iter().find(|r| r.uid == uid).map(foods::recipe_row) else { continue };
        if !rows.iter().any(|r| r.key == row.key) {
            rows.push(row);
        }
    }
    Ok(rows)
}

pub(crate) fn slot_label(slot: MealType) -> &'static str {
    if slot == MealType::Snack {
        "Snacks"
    } else {
        slot.label()
    }
}

/// A live (not deleted) entry of `day` by uid, for changing. A miss after `month_mut` is harmless: the shell's `mutate_journal` discards unsaved changes on any error.
fn live_entry<'a>(j: &'a mut Journal, d: &Day, uid: &str) -> Result<&'a mut DiaryEntry, String> {
    j.month_mut(d.month())?.entries.iter_mut().find(|e| e.uid == uid && e.deleted_at.is_none() && e.day == d.as_str()).ok_or_else(|| NOT_THERE.to_string())
}

/// Delete an entry the soft way (D-15): it stays in the month file with `deleted_at` set. Returns its name and the undo.
pub fn delete(j: &mut Journal, day: &str, uid: &str, now: u64) -> Result<(String, DiaryUndo), String> {
    let d = Day::parse(day)?;
    let e = live_entry(j, &d, uid)?;
    let before = e.clone();
    e.deleted_at = Some(now);
    e.modified = now;
    Ok((before.name.clone(), DiaryUndo { day: d.as_str().into(), before: vec![before], ..Default::default() }))
}

/// Put back what `u` replaced and remove what it added. A restored entry gets a new `modified`, so a later sync merge sees it as the newest.
pub fn undo(j: &mut Journal, u: &DiaryUndo, now: u64) -> Result<(), String> {
    let d = Day::parse(&u.day)?;
    let month = j.month_mut(d.month())?;
    for b in &u.before {
        let back = DiaryEntry { modified: now, ..b.clone() };
        match month.entries.iter_mut().find(|e| e.uid == back.uid) {
            Some(e) => *e = back,
            None => month.entries.push(back),
        }
    }
    month.entries.retain(|e| !u.added.contains(&e.uid));
    // exercise and body ride in the same step and the same month write, so one undo covers them all
    for b in &u.before_exercise {
        let back = ExerciseEntry { modified: now, ..b.clone() };
        match month.exercise.iter_mut().find(|e| e.uid == back.uid) {
            Some(e) => *e = back,
            None => month.exercise.push(back),
        }
    }
    month.exercise.retain(|e| !u.added_exercise.contains(&e.uid));
    for b in &u.before_body {
        let back = BodyEntry { modified: now, ..b.clone() };
        match month.body.iter_mut().find(|e| e.uid == back.uid) {
            Some(e) => *e = back,
            None => month.body.push(back),
        }
    }
    month.body.retain(|e| !u.added_body.contains(&e.uid));
    Ok(())
}

/// An entry opened as a food, for the Edit dialog and `food_amount`: units and figures come from the entry's own frozen basis, never from a live food (D-11, TRK-04).
/// A weighed entry offers g, oz and the unit it was logged in; one with no weight (a recipe serving of unknown size, or a quick add) offers its one serving.
pub fn entry_detail(j: &mut Journal, day: &str, uid: &str, us: bool) -> Result<foods::FoodDetail, String> {
    let d = Day::parse(day)?;
    let e = j.month(d.month())?.entries.iter().find(|e| e.uid == uid && e.deleted_at.is_none() && e.day == d.as_str()).ok_or(NOT_THERE)?.clone();
    let unit = if e.unit.is_empty() { "serving".to_string() } else { e.unit.clone() };
    let (units, per100, per_serving) = if e.per_100g {
        let mut units = foods::portions::units(None, None, &[], us);
        let each = e.grams.filter(|_| e.amount > 0.0).map(|g| g / e.amount).filter(|g| g.is_finite() && *g > 0.0);
        if let (Some(grams), false) = (each, units.iter().any(|u| u.id == unit)) {
            units.push(foods::portions::Unit { id: unit.clone(), label: format!("{} ({} g)", unit.strip_prefix("piece:").unwrap_or(&unit), foods::num1(grams)), grams });
        }
        (units, e.basis, None)
    } else {
        (vec![foods::portions::Unit { id: unit.clone(), label: "serving".into(), grams: 0.0 }], Nutrients::default(), Some(e.basis))
    };
    Ok(foods::FoodDetail { key: format!("entry:{}:{}", d.as_str(), e.uid), name: e.name.clone(), brand: String::new(), tag: tag_of(&e.source), online: false, suspect: false, units, unit, amount: e.amount, per100, per_serving, warning: None, note: None })
}

/// Change an entry's amount, unit and meal (D-11), measured against its own frozen basis. The basis, its kind, the source and the uid are never touched.
pub fn edit(j: &mut Journal, day: &str, uid: &str, amount: f32, unit: &str, slot: MealType, now: u64) -> Result<(String, DiaryUndo), String> {
    if !amount.is_finite() || amount <= 0.0 {
        return Err(BAD_AMOUNT.into());
    }
    let m = foods::amount(&entry_detail(j, day, uid, false)?, Some(amount), unit, None)?;
    let d = Day::parse(day)?;
    let e = live_entry(j, &d, uid)?;
    let before = e.clone();
    e.amount = m.amount.unwrap_or(amount);
    e.unit = unit.into();
    if e.per_100g {
        e.grams = m.grams;
    }
    e.slot = slot;
    e.modified = now;
    Ok((before.name.clone(), DiaryUndo { day: d.as_str().into(), before: vec![before], ..Default::default() }))
}

/// What the entry holds, from its frozen basis only. Unknown stays unknown.
pub fn entry_nutrients(e: &DiaryEntry) -> Nutrients {
    if e.per_100g {
        e.grams.map_or(Nutrients::default(), |g| e.basis.scaled(g / 100.0))
    } else {
        e.basis.scaled(e.amount)
    }
}

// D-16: the owner's chosen exception to TRK-03, for net carbs only: an unreported fiber counts as 0, so net carbs is known whenever carbs is.
pub fn net_carbs(n: &Nutrients) -> Option<f32> {
    n.carbs_g.map(|c| c - n.fiber_g.unwrap_or(0.0))
}

/// Sums for a meal or a day, rounded for display. `None` is "not reported for some food", never a partial sum.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Totals {
    pub calories: Option<f32>,
    pub protein: Option<f32>,
    pub carbs: Option<f32>,
    pub fat: Option<f32>,
    pub fiber: Option<f32>,
    pub net_carbs: Option<f32>,
}

pub(crate) fn totals(entries: &[&DiaryEntry]) -> Totals {
    let (mut sum, mut net) = (Nutrients::ZERO, Some(0.0f32));
    for e in entries {
        let n = entry_nutrients(e);
        sum = sum.plus(&n);
        // folded on its own: deriving it from the summed fiber would count a known fiber as 0 whenever another entry lacks it
        net = net.zip(net_carbs(&n)).map(|(a, b)| a + b);
    }
    let s = foods::display(&sum);
    Totals { calories: s.calories, protein: s.protein_g, carbs: s.carbs_g, fat: s.fat_g, fiber: s.fiber_g, net_carbs: net.map(|x| (x * 10.0).round() / 10.0) }
}

/// One logged food as the page lists it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EntryView {
    pub uid: String,
    pub name: String,
    pub tag: &'static str,
    pub amount_text: String,
    pub calories: Option<f32>,
    pub protein: Option<f32>,
    pub carbs: Option<f32>,
    pub fat: Option<f32>,
}

fn tag_of(s: &FoodRef) -> &'static str {
    match s {
        FoodRef::Bundled(id) if id.starts_with('C') => "Canada",
        FoodRef::Bundled(_) => "USDA",
        FoodRef::MyFood(_) => "My Food",
        FoodRef::Off(_) => "Open Food Facts",
        FoodRef::Recipe(_) => "Recipe",
        FoodRef::Custom => "Quick Add",
    }
}

fn amount_text(e: &DiaryEntry) -> String {
    if e.source == FoodRef::Custom {
        return if e.amount == 1.0 { "Quick add".into() } else { format!("Quick add × {}", foods::num1(e.amount)) };
    }
    let a = foods::num1(e.amount);
    if e.unit == "g" {
        return format!("{a} g");
    }
    let unit = e.unit.strip_prefix("piece:").unwrap_or(&e.unit);
    match e.grams {
        Some(g) => format!("{a} {unit} ({} g)", foods::num1(g)),
        None => format!("{a} {unit}"),
    }
}

fn entry_view(e: &DiaryEntry) -> EntryView {
    let n = foods::display(&entry_nutrients(e));
    EntryView { uid: e.uid.clone(), name: e.name.clone(), tag: tag_of(&e.source), amount_text: amount_text(e), calories: n.calories, protein: n.protein_g, carbs: n.carbs_g, fat: n.fat_g }
}

/// One meal of a day: its entries in log order and their subtotal.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SlotView {
    pub slot: MealType,
    pub label: &'static str,
    pub totals: Totals,
    pub entries: Vec<EntryView>,
}

/// Which of the budget's cases the day is in. Anything but `Set` leaves the day's foods working with no budget.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BudgetState {
    /// A target is in force on the day.
    Set,
    /// No target has ever been saved.
    Never,
    /// Targets exist but the first one starts after this day.
    Before,
    /// profile.json is damaged or from a newer build.
    Unavailable,
}

pub const NO_TARGET_DAY: &str = "No target was set for this day.";
pub const FOOD_UNKNOWN: &str = "not known: a food has no calories";
pub const EXERCISE_NO_WEIGHT: &str = "Add a weight to estimate exercise calories.";

/// One macro goal against what was eaten. `unit_text` is "/ 150 g", or "over by 12 g" once the goal is passed.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MacroGoal {
    pub goal: i32,
    pub eaten: Option<f32>,
    pub unit_text: String,
    pub over: bool,
    /// The screen-reader text, "96 of 150 g protein".
    pub said: String,
}

/// The day's calorie budget: `remaining = target + exercise credited - food`, worked from the whole numbers
/// that are printed, so the formula line always adds up. Unknown food calories make Remaining unknown (never 0).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Budget {
    pub target: i32,
    pub remaining: Option<i32>,
    pub over: bool,
    /// "Remaining", or "Over by" with `figure` positive (never a minus sign).
    pub label: &'static str,
    pub figure: Option<i32>,
    /// `figure` as printed: "1,268".
    pub figure_text: Option<String>,
    pub said: String,
    /// "Target 2,100 + Exercise 240 − Food 1,700"
    pub formula: String,
    /// Shown when the weekday's number was raised to the floor.
    pub raised_note: Option<String>,
    pub exercise_raw: Option<i32>,
    pub exercise_counted: Option<i32>,
    pub exercise_note: Option<&'static str>,
    pub template: Option<String>,
    /// "Training: +300 kcal from your base target"
    pub template_title: Option<String>,
    pub protein: MacroGoal,
    pub carbs: MacroGoal,
    pub fat: MacroGoal,
}

fn macro_goal(row: &goals::MacroRow, eaten: Option<f32>, name: &str) -> MacroGoal {
    let goal = row.grams.round() as i32;
    let over = eaten.is_some_and(|e| e > goal as f32);
    let by = eaten.map_or(0, |e| ((e - goal as f32).round() as i32).max(1));
    let (unit_text, said) = match eaten {
        Some(e) if over => (format!("over by {by} g"), format!("{} of {goal} g {name}, over by {by} g", foods::num1(e))),
        Some(e) => (format!("/ {goal} g"), format!("{} of {goal} g {name}", foods::num1(e))),
        None => (format!("/ {goal} g"), format!("{goal} g {name} goal, amount eaten not known")),
    };
    MacroGoal { goal, eaten, unit_text, over, said }
}

/// The budget for `day` from the record in force that day. The day's weight is the latest weigh-in on or before
/// it, so a Grams per kg protein goal follows the weight logged since and a later weigh-in never changes an
/// earlier day. The profile is only read here, never marked changed.
fn budget(j: &mut Journal, day: &str, exercise: &[ExerciseEntry], t: &Totals) -> (Option<Budget>, BudgetState, Option<String>) {
    let (set, any) = match j.profile() {
        Ok(p) => (goals::target_for_day(&p.targets, day).cloned(), p.targets.iter().any(|s| Day::parse(&s.effective_from).is_ok())),
        Err(_) => return (None, BudgetState::Unavailable, None),
    };
    let Some(set) = set else {
        return if any { (None, BudgetState::Before, Some(NO_TARGET_DAY.into())) } else { (None, BudgetState::Never, None) };
    };
    let Ok(dt) = goals::day_target(&set, day, body::weight_basis(j, day).map(|w| w.kg)) else { return (None, BudgetState::Unavailable, None) };
    let kt = goals::kcal_text;

    // an entry with no calories is left out and said so; with no entries the credit is a known 0
    let known: Vec<f64> = exercise.iter().filter_map(|e| e.kcal_override.or(e.est_kcal)).collect();
    let unknown = known.len() < exercise.len();
    let raw = if !exercise.is_empty() && known.is_empty() { None } else { Some(known.iter().sum::<f64>().round() as i32) };
    let pct = i32::from(set.addback_pct);
    let counted = raw.map(|r| (f64::from(r) * f64::from(pct) / 100.0).round() as i32);
    let food = t.calories.map(|c| c.round() as i32);
    let remaining = food.map(|f| dt.kcal + counted.unwrap_or(0) - f);
    let over = remaining.is_some_and(|r| r < 0);
    let said = match remaining {
        None => FOOD_UNKNOWN.to_string(),
        Some(r) if r < 0 => format!("Over by {} kcal", kt(-r)),
        Some(r) => format!("Remaining {} kcal", kt(r)),
    };
    let exercise_term = match (raw, counted) {
        (Some(r), Some(c)) if pct < 100 && r > 0 => format!("Exercise {} ({pct}% of {})", kt(c), kt(r)),
        (_, Some(c)) => format!("Exercise {}", kt(c)),
        _ => "Exercise –".to_string(),
    };
    let food_term = food.map_or("Food –".to_string(), |f| format!("Food {}", kt(f)));
    let rows = &dt.macros.rows;
    let b = Budget {
        target: dt.kcal,
        remaining,
        over,
        label: if over { "Over by" } else { "Remaining" },
        figure: remaining.map(i32::abs),
        figure_text: remaining.map(|r| kt(r.abs())),
        said,
        formula: format!("Target {} + {exercise_term} \u{2212} {food_term}", kt(dt.kcal)),
        raised_note: dt.note.clone(),
        exercise_raw: raw,
        exercise_counted: counted,
        exercise_note: unknown.then_some(EXERCISE_NO_WEIGHT),
        template_title: dt.template.as_ref().map(|(_, n)| format!("{n}: {} kcal from your base target", goals::signed_kcal_text(dt.offset))),
        template: dt.template.map(|(_, n)| n),
        protein: macro_goal(&rows[0], t.protein, "protein"),
        carbs: macro_goal(&rows[1], t.net_carbs, "net carbs"),
        fat: macro_goal(&rows[2], t.fat, "fat"),
    };
    (Some(b), BudgetState::Set, None)
}

/// One logged workout as the day shows it. The kcal is the person's own number when there is one, else the frozen estimate; None is unknown.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ExerciseView {
    pub uid: String,
    pub name: String,
    /// "Compendium", "Custom" or "Workout".
    pub tag: &'static str,
    /// The calories are the person's own number.
    pub edited: bool,
    /// The duration as a number, for the Edit Exercise box.
    pub minutes: f64,
    /// "45 min"
    pub minutes_text: String,
    /// What Edit Exercise says is kept from log time: the MET and weight, or the person's own rate.
    pub frozen_text: String,
    /// "MET 7.5", or "your estimate" for a kcal-per-hour activity.
    pub basis_text: String,
    /// "at 82.4 kg", in the person's weight unit; None when no weight was on record.
    pub weight_text: Option<String>,
    pub kcal: Option<i32>,
    /// "−320" with a true minus; None when unknown.
    pub kcal_text: Option<String>,
}

/// The day's Exercise section: its rows in log order and the net total.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct ExerciseSection {
    pub entries: Vec<ExerciseView>,
    /// "−560 kcal", "–" when every row is unknown, None with no rows.
    pub total_text: Option<String>,
    /// "counts as −280", only when the day's add-back is under 100%.
    pub counted_text: Option<String>,
}

/// Calories burned as a negative figure with a true minus sign: 320 is "−320".
fn burned_text(kcal: i32) -> String {
    if kcal == 0 {
        "0".into()
    } else {
        format!("\u{2212}{}", goals::kcal_text(kcal.abs()))
    }
}

fn exercise_view(e: &ExerciseEntry, unit: goals::MassUnit) -> ExerciseView {
    let kcal = e.kcal_override.or(e.est_kcal).map(|k| k.round() as i32);
    ExerciseView {
        uid: e.uid.clone(),
        name: e.name.clone(),
        tag: match e.source {
            ExerciseSource::Compendium(_) => "Compendium",
            ExerciseSource::Custom(_) => "Custom",
            ExerciseSource::Workout(_) => "Workout",
        },
        edited: e.kcal_override.is_some(),
        minutes: e.minutes,
        minutes_text: exercise::minutes_text(e.minutes),
        frozen_text: match (&e.basis, e.weight_kg) {
            (ExerciseBasis::Met(m), Some(kg)) => format!("MET {} and {} are kept as they were when you logged this, even if your weight has changed since.", exercise::met_str(*m), goals::mass_text(kg, unit)),
            (ExerciseBasis::Met(m), None) => format!("MET {} is kept as it was when you logged this. No weight was on record, so the calories stay unknown unless you enter your own.", exercise::met_str(*m)),
            (ExerciseBasis::KcalPerHour(k), _) => format!("Your own rate, {} kcal an hour, is kept as you saved it.", exercise::trim(*k)),
        },
        basis_text: match e.basis {
            ExerciseBasis::Met(m) => format!("MET {}", exercise::met_str(m)),
            ExerciseBasis::KcalPerHour(_) => "your estimate".into(),
        },
        weight_text: e.weight_kg.map(|kg| format!("at {}", goals::mass_text(kg, unit))),
        kcal,
        kcal_text: kcal.map(burned_text),
    }
}

/// The Exercise section for `day` from its live entries (log order). The total is net and leaves unknown rows out, summed the way the budget sums it.
fn exercise_section(j: &mut Journal, day: &str, entries: &[ExerciseEntry]) -> ExerciseSection {
    let mut sorted: Vec<&ExerciseEntry> = entries.iter().collect();
    sorted.sort_by_key(|e| e.logged_at); // stable: file order breaks a tie
    let (unit, pct) = match j.profile() {
        Ok(p) => (p.units.weight, goals::target_for_day(&p.targets, day).map_or(100, |t| i32::from(t.addback_pct))),
        Err(_) => (goals::MassUnit::default(), 100),
    };
    let known: Vec<f64> = entries.iter().filter_map(|e| e.kcal_override.or(e.est_kcal)).collect();
    let raw = (!known.is_empty()).then(|| known.iter().sum::<f64>().round() as i32);
    let total_text = match (entries.is_empty(), raw) {
        (true, _) => None,
        (false, Some(r)) => Some(format!("{} kcal", burned_text(r))),
        (false, None) => Some("–".to_string()),
    };
    let counted_text = raw.filter(|_| pct < 100).map(|r| format!("counts as {}", burned_text((f64::from(r) * f64::from(pct) / 100.0).round() as i32)));
    ExerciseSection { entries: sorted.into_iter().map(|e| exercise_view(e, unit)).collect(), total_text, counted_text }
}

/// A day as the diary page shows it. `prev` and `next` are the neighbouring days, across month and year ends.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DayView {
    pub day: String,
    pub prev: String,
    pub next: String,
    pub is_today: bool,
    pub totals: Totals,
    pub slots: Vec<SlotView>,
    /// The fifth section, after Snacks.
    pub exercise: ExerciseSection,
    /// The day is marked complete (it stays editable).
    pub complete: bool,
    /// Live entries and workouts on the day before, for Copy Yesterday.
    pub previous_day_count: usize,
    /// Meals planned for today; 0 here, the shell fills it for today.
    pub planned_count: usize,
    pub budget: Option<Budget>,
    pub budget_state: BudgetState,
    /// "No target was set for this day." when the day is before the first target.
    pub budget_note: Option<String>,
}

pub(crate) fn shift(d: &Day, n: i32) -> String {
    let s = d.as_str();
    // a Day is already checked, so these parts always parse
    let (y, m, dd): (i32, u32, u32) = (s[..4].parse().unwrap_or(1970), s[5..7].parse().unwrap_or(1), s[8..].parse().unwrap_or(1));
    let (y, m, dd) = civil_from_days(days_from_civil(y, m, dd) + n);
    format!("{y:04}-{m:02}-{dd:02}")
}

/// The four meals of `day` with their entries and totals. `today` is the shell's local day; this crate never reads the clock for it.
pub fn day_view(j: &mut Journal, day: &str, today: &str) -> Result<DayView, String> {
    let (d, t) = (Day::parse(day)?, Day::parse(today)?);
    let month = j.month(d.month())?;
    let complete = month.complete.contains(d.as_str());
    let live: Vec<DiaryEntry> = month.entries.iter().filter(|e| e.deleted_at.is_none() && e.day == d.as_str()).cloned().collect();
    let workouts: Vec<ExerciseEntry> = month.exercise.iter().filter(|e| e.deleted_at.is_none() && e.day == d.as_str()).cloned().collect();
    let prev = shift(&d, -1);
    let previous_day_count = j.month(&prev[..7]).map_or(0, |m| m.entries.iter().filter(|e| e.deleted_at.is_none() && e.day == prev).count() + m.exercise.iter().filter(|e| e.deleted_at.is_none() && e.day == prev).count());
    let slots = MealType::ALL
        .iter()
        .map(|&slot| {
            let mine: Vec<&DiaryEntry> = live.iter().filter(|e| e.slot == slot).collect();
            let label = slot_label(slot);
            SlotView { slot, label, totals: totals(&mine), entries: mine.into_iter().map(entry_view).collect() }
        })
        .collect();
    let all: Vec<&DiaryEntry> = live.iter().collect();
    let day_totals = totals(&all);
    let (budget, budget_state, budget_note) = budget(j, d.as_str(), &workouts, &day_totals);
    let exercise = exercise_section(j, d.as_str(), &workouts);
    Ok(DayView { day: d.as_str().into(), prev, next: shift(&d, 1), is_today: d == t, totals: day_totals, slots, exercise, complete, previous_day_count, planned_count: 0, budget, budget_state, budget_note })
}

/// The foods the add dialog offers before anything is typed.
#[derive(Serialize, Clone, Debug)]
pub struct History {
    pub recent: Vec<FoodRow>,
    pub frequent: Vec<FoodRow>,
}

/// Rows listed in each of recent and frequent.
const HISTORY_ROWS: usize = 8;
/// Days of diary the history looks back over, today included.
pub(crate) const HISTORY_DAYS: i32 = 60;

/// Recent foods (most recently logged first) and frequent foods (most entries, minus the recent ones) over the last 60 days up to
/// `today`. Deleted entries, quick adds and foods that no longer resolve are left out. A month that won't open is skipped.
pub fn history(j: &mut Journal, today: &str, mine: &[MyFood], online: &[OnlineFood], recipes: &[RecipeFood]) -> Result<History, String> {
    let end = Day::parse(today)?;
    let start = Day::parse(&shift(&end, 1 - HISTORY_DAYS))?;
    let mut months: Vec<String> = vec![];
    for i in 0..HISTORY_DAYS {
        let m = shift(&start, i)[..7].to_string();
        if !months.contains(&m) {
            months.push(m);
        }
    }
    let mut seen: Vec<(u64, usize, DiaryEntry)> = vec![];
    for m in &months {
        let Ok(month) = j.month(m) else { continue };
        for e in &month.entries {
            if e.deleted_at.is_none() && e.source != FoodRef::Custom && e.day.as_str() >= start.as_str() && e.day.as_str() <= end.as_str() {
                seen.push((e.logged_at, seen.len(), e.clone()));
            }
        }
    }
    seen.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    // each source is looked up once; one that no longer resolves is dropped, and sources that resolve to the same food count together
    let mut by_source: HashMap<String, Option<FoodRow>> = HashMap::new();
    let mut rows: HashMap<String, FoodRow> = HashMap::new();
    let mut order: Vec<String> = vec![];
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_, _, e) in &seen {
        let Some(r) = by_source.entry(format!("{:?}", e.source)).or_insert_with(|| resolve(&e.source, mine, online, recipes)).clone() else { continue };
        let n = counts.entry(r.key.clone()).or_insert(0);
        if *n == 0 {
            order.push(r.key.clone());
            rows.insert(r.key.clone(), r);
        }
        *n += 1;
    }
    // `order` is newest first, so a stable sort by count keeps the newer log ahead on a tie
    let recent: Vec<&String> = order.iter().take(HISTORY_ROWS).collect();
    let mut rest: Vec<&String> = order.iter().filter(|k| !recent.contains(k)).collect();
    rest.sort_by_key(|k| std::cmp::Reverse(counts[*k]));
    Ok(History { recent: recent.into_iter().map(|k| rows[k].clone()).collect(), frequent: rest.into_iter().take(HISTORY_ROWS).map(|k| rows[k].clone()).collect() })
}

/// The row for a logged food's source today, or None when the food is gone.
fn resolve(source: &FoodRef, mine: &[MyFood], online: &[OnlineFood], recipes: &[RecipeFood]) -> Option<FoodRow> {
    match source {
        FoodRef::Bundled(id) => crate::nutrition::db_foods().iter().find(|f| f.id == id).map(foods::db_row),
        FoodRef::MyFood(name) => {
            let name = name.trim();
            let same = |s: &str| s.trim().eq_ignore_ascii_case(name);
            mine.iter().find(|f| same(&f.name)).or_else(|| mine.iter().find(|f| f.aliases.split(',').any(same))).map(foods::my_row)
        }
        FoodRef::Off(code) => online.iter().find(|f| &f.code == code).map(foods::online_row),
        FoodRef::Recipe(uid) => recipes.iter().find(|r| &r.uid == uid).map(foods::recipe_row),
        FoodRef::Custom => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MyFood;
    use std::fs;
    use std::path::PathBuf;

    fn tdir(name: &str) -> PathBuf {
        let d = crate::scratch_dir().join(format!("rb-diary-{}-{}", name, crate::test_pid()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// A My Food whose serving is 100 g, so its per-serving numbers are also its per-100 g numbers.
    fn per100(id: u64, n: Nutrients) -> MyFood {
        MyFood { id, name: format!("Food {id}"), serving: "1 serving".into(), serving_g: 100.0, per_serving: n, ..Default::default() }
    }

    fn detail(f: &MyFood) -> foods::FoodDetail {
        foods::detail(&format!("my:{}", f.id), std::slice::from_ref(f), &[], false).unwrap()
    }

    fn log(j: &mut Journal, day: &str, slot: MealType, f: &MyFood, amount: f32, unit: &str) -> String {
        add(j, day, slot, from_food(&detail(f), amount, unit).unwrap(), 1000).unwrap()
    }

    fn n(calories: f32, carbs: Option<f32>, fiber: Option<f32>) -> Nutrients {
        Nutrients { calories: Some(calories), carbs_g: carbs, fiber_g: fiber, ..Default::default() }
    }

    #[test]
    fn day_totals_carry_unknown() {
        let d = tdir("totals");
        let mut j = Journal::open(d.clone());
        let a = per100(1, Nutrients { calories: Some(150.0), protein_g: Some(10.0), carbs_g: Some(30.0), fiber_g: Some(5.0), fat_g: Some(2.0), ..Default::default() });
        let b = per100(2, n(100.0, Some(20.0), None));
        log(&mut j, "2026-03-10", MealType::Breakfast, &a, 100.0, "g");
        log(&mut j, "2026-03-10", MealType::Breakfast, &b, 200.0, "g");
        let v = day_view(&mut j, "2026-03-10", "2026-03-10").unwrap();
        let t = &v.totals;
        assert_eq!((t.calories, t.carbs, t.fiber, t.net_carbs), (Some(350.0), Some(70.0), None, Some(65.0)));
        assert_eq!(t.protein, None, "the second food does not report protein");
        assert_eq!(v.slots[0].totals, v.totals);
        // an entry whose carbs are unknown makes net carbs unknown for its meal and the day
        let c = per100(3, n(50.0, None, None));
        log(&mut j, "2026-03-10", MealType::Lunch, &c, 100.0, "g");
        let v = day_view(&mut j, "2026-03-10", "2026-03-10").unwrap();
        assert_eq!((v.slots[1].totals.net_carbs, v.slots[1].totals.calories, v.totals.net_carbs), (None, Some(50.0), None));
        assert_eq!(v.slots[0].totals.net_carbs, Some(65.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn net_carbs_counts_unreported_fiber_as_zero() {
        assert_eq!(net_carbs(&n(1.0, Some(30.0), None)), Some(30.0));
        assert_eq!(net_carbs(&n(1.0, Some(30.0), Some(5.0))), Some(25.0));
        assert_eq!(net_carbs(&n(1.0, None, Some(5.0))), None);
    }

    #[test]
    fn late_evening_entry_files_on_its_local_day() {
        let d = tdir("late");
        let mut j = Journal::open(d.clone());
        log(&mut j, "2026-01-31", MealType::Dinner, &per100(1, n(80.0, Some(1.0), None)), 100.0, "g");
        j.save_dirty().unwrap();
        assert!(d.join("2026-01.json").exists() && !d.join("2026-02.json").exists());
        assert_eq!(day_view(&mut j, "2026-01-31", "2026-02-01").unwrap().slots[2].entries.len(), 1);
        assert!(day_view(&mut j, "2026-02-01", "2026-02-01").unwrap().slots.iter().all(|s| s.entries.is_empty()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn day_view_lists_four_slots_in_order() {
        let d = tdir("slots");
        let mut j = Journal::open(d.clone());
        let v = day_view(&mut j, "2026-12-31", "2026-12-31").unwrap();
        assert_eq!((v.prev.as_str(), v.next.as_str(), v.is_today), ("2026-12-30", "2027-01-01", true));
        assert_eq!(v.slots.iter().map(|s| s.label).collect::<Vec<_>>(), ["Breakfast", "Lunch", "Dinner", "Snacks"]);
        let zero = Totals { calories: Some(0.0), protein: Some(0.0), carbs: Some(0.0), fat: Some(0.0), fiber: Some(0.0), net_carbs: Some(0.0) };
        assert!(v.slots.iter().all(|s| s.totals == zero && s.entries.is_empty()) && v.totals == zero);
        let m = day_view(&mut j, "2024-03-01", "2026-12-31").unwrap();
        assert_eq!((m.prev.as_str(), m.is_today), ("2024-02-29", false));
        // entries keep log order inside a meal
        let f = per100(1, n(10.0, None, None));
        for amount in [100.0, 200.0, 300.0] {
            log(&mut j, "2026-12-31", MealType::Snack, &f, amount, "g");
        }
        let v = day_view(&mut j, "2026-12-31", "2026-12-31").unwrap();
        assert_eq!(v.slots[3].entries.iter().map(|e| e.amount_text.as_str()).collect::<Vec<_>>(), ["100 g", "200 g", "300 g"]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn logged_values_stay_frozen() {
        let d = tdir("frozen");
        let mut j = Journal::open(d.clone());
        let mut f = MyFood { id: 7, name: "Whey".into(), serving: "1 scoop".into(), serving_g: 31.0, per_serving: n(120.0, None, None), piece: "slice".into(), piece_g: 28.0, ..Default::default() };
        log(&mut j, "2026-03-10", MealType::Snack, &f, 1.0, "serving");
        f.per_serving.calories = Some(999.0);
        f.name = "Renamed".into();
        let again = detail(&f);
        assert_eq!(foods::amount(&again, Some(1.0), "serving", None).unwrap().nutrients.calories, Some(999.0));
        let v = day_view(&mut j, "2026-03-10", "2026-03-10").unwrap();
        assert_eq!((v.totals.calories, v.slots[3].entries[0].name.as_str(), v.slots[3].entries[0].tag), (Some(120.0), "Whey", "My Food"));
        assert_eq!(v.slots[3].entries[0].amount_text, "1 serving (31 g)");
        // a piece unit reads with its weight, grams read plainly, a Canada id gets its tag
        f.per_serving.calories = Some(120.0);
        log(&mut j, "2026-03-10", MealType::Snack, &f, 2.0, "piece:slice");
        let v = day_view(&mut j, "2026-03-10", "2026-03-10").unwrap();
        assert_eq!(v.slots[3].entries[1].amount_text, "2 slice (56 g)");
        let mut e = DiaryEntry { source: FoodRef::Bundled("C1234".into()), unit: "g".into(), amount: 150.0, grams: Some(150.0), ..Default::default() };
        assert_eq!((tag_of(&e.source), amount_text(&e).as_str()), ("Canada", "150 g"));
        e.source = FoodRef::Bundled("F321358".into());
        assert_eq!(tag_of(&e.source), "USDA");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn amounts_and_bad_days_are_refused() {
        let d = tdir("refused");
        let mut j = Journal::open(d.clone());
        let f = per100(1, n(10.0, None, None));
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(from_food(&detail(&f), bad, "g").unwrap_err(), BAD_AMOUNT);
        }
        assert_eq!(from_food(&detail(&f), 1.0, "cup").unwrap_err(), "That unit doesn't work for this food.");
        let mut gone = detail(&f);
        gone.key = "zzz:abc".into();
        assert!(from_food(&gone, 1.0, "g").is_err());
        let e = from_food(&detail(&f), 1.0, "g").unwrap();
        assert!(add(&mut j, "2026-02-30", MealType::Lunch, e, 1).unwrap_err().contains("isn't a valid date"));
        j.save_dirty().unwrap();
        assert!(!d.join("2026-02.json").exists(), "a refused day touches no month");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn recipe_logs_by_serving_with_unknowns() {
        let d = tdir("recipe");
        let mut j = Journal::open(d.clone());
        let per = Nutrients { calories: Some(400.0), protein_g: Some(20.0), carbs_g: Some(50.0), ..Default::default() };
        let soup = foods::RecipeFood { uid: "u1".into(), name: "Tomato soup".into(), per_serving: Some(per) };
        // with a known weight it is measured in grams too
        let e = from_food(&foods::recipe_detail(&soup, Some(250.0), false), 1.5, "serving").unwrap();
        assert_eq!((e.source.clone(), e.per_100g, e.grams), (FoodRef::Recipe("u1".into()), true, Some(375.0)));
        add(&mut j, "2026-03-10", MealType::Dinner, e, 1).unwrap();
        // without one the basis is a serving and the entry counts servings
        let e = from_food(&foods::recipe_detail(&soup, None, false), 2.0, "serving").unwrap();
        assert_eq!((e.per_100g, e.grams, e.unit.as_str(), e.basis), (false, None, "serving", per));
        add(&mut j, "2026-03-10", MealType::Lunch, e, 2).unwrap();
        let v = day_view(&mut j, "2026-03-10", "2026-03-10").unwrap();
        assert_eq!((v.slots[2].totals.calories, v.slots[1].totals.calories, v.slots[1].totals.carbs), (Some(600.0), Some(800.0), Some(100.0)));
        assert_eq!((v.slots[1].entries[0].tag, v.slots[1].entries[0].amount_text.as_str(), v.slots[2].entries[0].amount_text.as_str()), ("Recipe", "2 serving", "1.5 serving (375 g)"));
        assert_eq!(v.slots[1].totals.fiber, None, "an unreported value stays unknown while the known ones are kept");
        // a recipe with no estimate still logs, with every value unknown
        let e = from_food(&foods::recipe_detail(&foods::RecipeFood { per_serving: None, ..soup }, None, false), 1.0, "serving").unwrap();
        add(&mut j, "2026-03-11", MealType::Lunch, e, 3).unwrap();
        let v = day_view(&mut j, "2026-03-11", "2026-03-10").unwrap();
        assert_eq!((v.slots[1].entries.len(), v.totals.calories, v.slots[1].entries[0].calories), (1, None, None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn quick_add_blank_macros_stay_unknown() {
        let d = tdir("quick");
        let mut j = Journal::open(d.clone());
        let e = quick("", Some(250.0), None, Some(30.0), None).unwrap();
        assert_eq!((e.name.as_str(), e.source.clone(), e.per_100g, e.amount, e.grams), ("Quick Add", FoodRef::Custom, false, 1.0, None));
        assert_eq!(e.basis, Nutrients { calories: Some(250.0), carbs_g: Some(30.0), ..Default::default() });
        add(&mut j, "2026-03-10", MealType::Snack, e, 1).unwrap();
        let v = day_view(&mut j, "2026-03-10", "2026-03-10").unwrap();
        assert_eq!((v.totals.calories, v.totals.protein, v.totals.fat, v.totals.net_carbs), (Some(250.0), None, None, Some(30.0)));
        assert_eq!((v.slots[3].entries[0].tag, v.slots[3].entries[0].amount_text.as_str()), ("Quick Add", "Quick add"));
        assert_eq!(quick(&"x".repeat(120), Some(1.0), None, None, None).unwrap().name.len(), 80);
        assert_eq!(quick("  Cake  ", Some(0.0), Some(0.0), None, None).unwrap().basis.protein_g, Some(0.0), "a typed 0 stays 0");
        assert_eq!(quick_add_warning(Some(3500.0)), Some("That is a lot of calories for one entry. Check the number."));
        assert_eq!((quick_add_warning(Some(800.0)), quick_add_warning(None)), (None, None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn quick_add_needs_calories() {
        assert_eq!(quick("Office cake", None, Some(1.0), None, None).unwrap_err(), "Enter the calories for this entry.");
        for bad in [Some(-1.0), Some(f32::NAN), Some(f32::INFINITY)] {
            assert_eq!(quick("x", Some(100.0), bad, None, None).unwrap_err(), "Nutrition numbers can't be negative.");
            assert_eq!(quick("x", bad, None, None, None).unwrap_err(), "Nutrition numbers can't be negative.");
        }
    }

    fn put(j: &mut Journal, day: &str, name: &str, at: u64) {
        let e = NewEntry { name: name.into(), source: FoodRef::MyFood(name.into()), grams: Some(100.0), amount: 100.0, unit: "g".into(), basis: n(10.0, None, None), per_100g: true };
        add(j, day, MealType::Snack, e, at).unwrap();
    }

    fn keys(rows: &[FoodRow]) -> Vec<String> {
        rows.iter().map(|r| r.key.clone()).collect()
    }

    #[test]
    fn recent_and_frequent_foods() {
        let d = tdir("history");
        let mut j = Journal::open(d.clone());
        let mine: Vec<MyFood> = (1..=16).map(|i| per100(i, n(10.0, None, None))).collect();
        for i in 1..=12 {
            put(&mut j, "2026-11-01", &format!("Food {i}"), 100 + i);
        }
        // older and repeated logs make 1 and 2 frequent without changing the recent order
        for (day, name, at) in [("2026-10-05", "Food 1", 10), ("2026-10-06", "Food 1", 11), ("2026-10-07", "Food 2", 12)] {
            put(&mut j, day, name, at);
        }
        put(&mut j, "2026-09-17", "Food 16", 1); // the first day of the window
        put(&mut j, "2026-09-16", "Food 15", 2); // one day too old
        put(&mut j, "2026-11-16", "Food 14", 3); // after today
        put(&mut j, "2026-11-02", "Food 13", 300);
        j.month_mut("2026-11").unwrap().entries.last_mut().unwrap().deleted_at = Some(400);
        put(&mut j, "2026-11-02", "Gone", 301); // no such My Food any more
        add(&mut j, "2026-11-02", MealType::Snack, quick("Cake", Some(300.0), None, None, None).unwrap(), 302).unwrap();

        let h = history(&mut j, "2026-11-15", &mine, &[], &[]).unwrap();
        let id = |i: u64| format!("my:{i}");
        assert_eq!(keys(&h.recent), (5..=12).rev().map(id).collect::<Vec<_>>(), "newest first, at most 8");
        assert_eq!(keys(&h.frequent), [1, 2, 4, 3, 16].map(id), "most entries first, a tie goes to the newer log, recent foods skipped");
        let _ = fs::remove_dir_all(&d);
    }

    fn eat(j: &mut Journal, day: &str, slot: MealType, uid: &str, at: u64) -> String {
        let e = NewEntry { name: uid.into(), source: FoodRef::Recipe(uid.into()), grams: None, amount: 1.0, unit: "serving".into(), basis: Nutrients::default(), per_100g: false };
        add(j, day, slot, e, at).unwrap()
    }

    fn meal(id: u64, day: &str, kind: MealType, recipe_id: Option<u64>) -> Meal {
        Meal { id, day: day_number(day).unwrap(), kind, recipe_id, ..Default::default() }
    }

    fn uid_of(id: u64) -> Option<String> {
        Some(format!("r{id}"))
    }

    #[test]
    fn eaten_confirms_one_planned_meal_per_entry() {
        let d = tdir("eaten");
        let mut j = Journal::open(d.clone());
        let day = "2026-03-10";
        let meals = [
            meal(1, day, MealType::Lunch, Some(1)),
            meal(2, day, MealType::Dinner, Some(1)),
            meal(3, day, MealType::Breakfast, Some(2)),
            meal(4, day, MealType::Lunch, Some(3)),
            meal(5, day, MealType::Lunch, Some(4)),
            meal(6, day, MealType::Lunch, None),
            meal(7, "2026-04-01", MealType::Lunch, Some(5)),
        ];
        eat(&mut j, day, MealType::Dinner, "r1", 1);
        eat(&mut j, day, MealType::Snack, "r2", 2);
        eat(&mut j, "2026-03-11", MealType::Lunch, "r3", 3);
        let gone = eat(&mut j, day, MealType::Lunch, "r4", 4);
        delete(&mut j, day, &gone, 5).unwrap();
        eat(&mut j, "2026-04-01", MealType::Lunch, "r5", 6);
        let refs: Vec<&Meal> = meals.iter().collect();
        assert_eq!(eaten(&mut j, &refs, uid_of), [2, 3, 7]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn planned_rows_lead_with_the_slot() {
        let d = tdir("planned-rows");
        let mut j = Journal::open(d.clone());
        let day = "2026-03-10";
        let meals = [
            meal(1, day, MealType::Lunch, Some(1)),
            meal(2, day, MealType::Dinner, Some(2)),
            meal(3, day, MealType::Breakfast, Some(3)),
            meal(4, day, MealType::Snack, None),
            meal(5, day, MealType::Dinner, Some(2)),
            meal(6, day, MealType::Lunch, Some(9)),
            meal(7, "2026-03-11", MealType::Lunch, Some(1)),
        ];
        let recipes: Vec<RecipeFood> = (1..=3).map(|i| RecipeFood { uid: format!("r{i}"), name: format!("Recipe {i}"), per_serving: None }).collect();
        eat(&mut j, day, MealType::Breakfast, "r3", 1);
        let rows = |j: &mut Journal, slot| keys(&planned_rows(j, day, slot, &meals, &recipes, |id| if id == 9 { Some("r9".into()) } else { uid_of(id) }).unwrap());
        assert_eq!(rows(&mut j, MealType::Dinner), ["recipe:r2", "recipe:r1"]);
        assert_eq!(rows(&mut j, MealType::Breakfast), ["recipe:r1", "recipe:r2"], "r3 is eaten, r9 is not in the library");
        assert!(planned_rows(&mut j, "2026-02-30", MealType::Lunch, &meals, &recipes, uid_of).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_hides_and_undo_restores_the_same_entry() {
        let d = tdir("delete");
        let mut j = Journal::open(d.clone());
        let f = per100(1, n(150.0, None, None));
        let uid = log(&mut j, "2026-03-10", MealType::Lunch, &f, 100.0, "g");
        j.save_dirty().unwrap();
        let original = j.month("2026-03").unwrap().entries[0].clone();
        let (name, u) = delete(&mut j, "2026-03-10", &uid, 5000).unwrap();
        j.save_dirty().unwrap();
        assert_eq!((name.as_str(), u.day.as_str(), u.added.len()), ("Food 1", "2026-03-10", 0));
        assert!(day_view(&mut j, "2026-03-10", "2026-03-10").unwrap().slots.iter().all(|s| s.entries.is_empty()));
        let kept = Journal::open(d.clone()).month("2026-03").unwrap().entries[0].clone();
        assert_eq!(kept.deleted_at, Some(5000), "the record stays in the month file");
        assert_eq!(delete(&mut j, "2026-03-10", &uid, 1).unwrap_err(), NOT_THERE, "a deleted entry is not there");
        undo(&mut j, &u, 6000).unwrap();
        let back = &j.month("2026-03").unwrap().entries[0];
        assert_eq!((&back.uid, back.deleted_at, back.basis, back.amount, back.grams), (&original.uid, None, original.basis, original.amount, original.grams));
        assert_eq!(day_view(&mut j, "2026-03-10", "2026-03-10").unwrap().slots[1].entries.len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn undo_removes_only_what_was_added() {
        let d = tdir("undoadded");
        let mut j = Journal::open(d.clone());
        let f = per100(1, n(10.0, None, None));
        let keep = log(&mut j, "2026-03-10", MealType::Lunch, &f, 100.0, "g");
        let a = log(&mut j, "2026-03-10", MealType::Lunch, &f, 100.0, "g");
        let b = log(&mut j, "2026-03-10", MealType::Dinner, &f, 100.0, "g");
        undo(&mut j, &DiaryUndo { day: "2026-03-10".into(), added: vec![a, b], ..Default::default() }, 9).unwrap();
        let left: Vec<&str> = j.month("2026-03").unwrap().entries.iter().map(|e| e.uid.as_str()).collect();
        assert_eq!(left, [keep.as_str()]);
        let _ = fs::remove_dir_all(&d);
    }

    /// An entry logged from a My Food of 100 g servings, saved and ready to edit.
    fn logged(j: &mut Journal, f: &MyFood, amount: f32, unit: &str) -> String {
        log(j, "2026-03-10", MealType::Dinner, f, amount, unit)
    }

    fn the(j: &mut Journal, uid: &str) -> DiaryEntry {
        j.month("2026-03").unwrap().entries.iter().find(|e| e.uid == uid).unwrap().clone()
    }

    #[test]
    fn edit_keeps_the_frozen_basis() {
        let d = tdir("edit");
        let mut j = Journal::open(d.clone());
        let mut f = per100(1, n(150.0, None, None));
        let uid = logged(&mut j, &f, 100.0, "g");
        f.per_serving.calories = Some(999.0); // the live food changes; the entry must not follow
        let before = the(&mut j, &uid);
        let (_, u) = edit(&mut j, "2026-03-10", &uid, 150.0, "g", MealType::Dinner, 7).unwrap();
        let e = the(&mut j, &uid);
        assert_eq!((e.basis, e.per_100g, e.grams, e.amount, e.source.clone()), (before.basis, true, Some(150.0), 150.0, before.source.clone()));
        assert_eq!(day_view(&mut j, "2026-03-10", "2026-03-10").unwrap().totals.calories, Some(225.0));
        edit(&mut j, "2026-03-10", &uid, 2.0, "oz", MealType::Dinner, 8).unwrap();
        let g = the(&mut j, &uid).grams.unwrap();
        assert!((g - 56.7).abs() < 0.05, "{g}");
        // the undo of an edit puts back the old amount, unit, grams and slot
        edit(&mut j, "2026-03-10", &uid, 3.0, "g", MealType::Snack, 9).unwrap();
        undo(&mut j, &u, 10).unwrap();
        let back = the(&mut j, &uid);
        assert_eq!((back.amount, back.unit.as_str(), back.grams, back.slot), (100.0, "g", Some(100.0), MealType::Dinner));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn entry_detail_offers_grams_oz_and_the_logged_unit() {
        let d = tdir("detail");
        let mut j = Journal::open(d.clone());
        let f = MyFood { id: 3, name: "Bread".into(), serving: "1 slice".into(), serving_g: 28.0, per_serving: n(70.0, None, None), piece: "slice".into(), piece_g: 28.0, ..Default::default() };
        let uid = logged(&mut j, &f, 2.0, "piece:slice");
        let det = entry_detail(&mut j, "2026-03-10", &uid, false).unwrap();
        assert_eq!(det.units.iter().map(|u| (u.id.as_str(), u.label.as_str())).collect::<Vec<_>>(), [("g", "g"), ("oz", "oz"), ("piece:slice", "slice (28 g)")]);
        assert_eq!(det.units[2].grams, 28.0);
        assert_eq!(det.key, format!("entry:2026-03-10:{uid}"));
        edit(&mut j, "2026-03-10", &uid, 3.0, "piece:slice", MealType::Dinner, 5).unwrap();
        assert_eq!(the(&mut j, &uid).grams, Some(84.0));
        // a weightless recipe entry offers only its serving, and grams are refused
        let soup = foods::RecipeFood { uid: "u1".into(), name: "Soup".into(), per_serving: Some(Nutrients { calories: Some(400.0), ..Default::default() }) };
        let w = add(&mut j, "2026-03-10", MealType::Dinner, from_food(&foods::recipe_detail(&soup, None, false), 1.0, "serving").unwrap(), 6).unwrap();
        assert_eq!(entry_detail(&mut j, "2026-03-10", &w, false).unwrap().units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), ["serving"]);
        edit(&mut j, "2026-03-10", &w, 2.0, "serving", MealType::Dinner, 7).unwrap();
        assert_eq!(the(&mut j, &w).amount, 2.0);
        assert_eq!(edit(&mut j, "2026-03-10", &w, 1.0, "g", MealType::Dinner, 8).unwrap_err(), "That unit doesn't work for this food.");
        assert_eq!(day_view(&mut j, "2026-03-10", "2026-03-10").unwrap().slots[2].entries[1].calories, Some(800.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn move_changes_only_the_meal() {
        let d = tdir("move");
        let mut j = Journal::open(d.clone());
        let uid = logged(&mut j, &per100(1, n(150.0, None, None)), 120.0, "g");
        let before = the(&mut j, &uid);
        edit(&mut j, "2026-03-10", &uid, 120.0, "g", MealType::Lunch, 4).unwrap();
        let after = the(&mut j, &uid);
        assert_eq!(after.slot, MealType::Lunch);
        assert_eq!((after.amount, after.unit.clone(), after.grams, after.basis, after.uid.clone()), (before.amount, before.unit.clone(), before.grams, before.basis, before.uid.clone()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn quick_add_entry_edits_its_portions_and_meal() {
        let d = tdir("quickedit");
        let mut j = Journal::open(d.clone());
        let q = quick("", Some(300.0), None, Some(30.0), None).unwrap();
        let uid = add(&mut j, "2026-03-10", MealType::Snack, q, 1).unwrap();
        assert_eq!(entry_detail(&mut j, "2026-03-10", &uid, false).unwrap().units.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(), ["serving"]);
        edit(&mut j, "2026-03-10", &uid, 2.0, "serving", MealType::Dinner, 2).unwrap();
        let v = day_view(&mut j, "2026-03-10", "2026-03-10").unwrap();
        let e = &v.slots[2].entries[0];
        assert_eq!((e.calories, e.carbs, e.amount_text.as_str(), the(&mut j, &uid).grams), (Some(600.0), Some(60.0), "Quick add × 2", None));
        assert!(v.slots[3].entries.is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn edit_refuses_bad_amounts_and_units() {
        let d = tdir("editbad");
        let mut j = Journal::open(d.clone());
        let uid = logged(&mut j, &per100(1, n(150.0, None, None)), 100.0, "g");
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(edit(&mut j, "2026-03-10", &uid, bad, "g", MealType::Dinner, 1).unwrap_err(), BAD_AMOUNT);
        }
        assert_eq!(edit(&mut j, "2026-03-10", &uid, 1.0, "cup", MealType::Dinner, 1).unwrap_err(), "That unit doesn't work for this food.");
        assert_eq!(edit(&mut j, "2026-03-10", "nope", 1.0, "g", MealType::Dinner, 1).unwrap_err(), NOT_THERE);
        assert!(edit(&mut j, "2026-02-30", &uid, 1.0, "g", MealType::Dinner, 1).unwrap_err().contains("isn't a valid date"));
        assert_eq!(the(&mut j, &uid).amount, 100.0, "a refusal changes nothing");
        let _ = fs::remove_dir_all(&d);
    }

    /// Two foods on the 14th (lunch and dinner), a deleted one, and one breakfast already on the 15th.
    fn two_days(j: &mut Journal) -> (String, String) {
        let f = per100(1, n(150.0, Some(10.0), None));
        let a = log(j, "2026-11-14", MealType::Lunch, &f, 100.0, "g");
        let b = log(j, "2026-11-14", MealType::Dinner, &f, 200.0, "g");
        let gone = log(j, "2026-11-14", MealType::Snack, &f, 50.0, "g");
        delete(j, "2026-11-14", &gone, 9).unwrap();
        log(j, "2026-11-15", MealType::Breakfast, &f, 30.0, "g");
        (a, b)
    }

    fn live(j: &mut Journal, day: &str) -> Vec<DiaryEntry> {
        j.month(&day[..7]).unwrap().entries.iter().filter(|e| e.day == day && e.deleted_at.is_none()).cloned().collect()
    }

    #[test]
    fn copy_adds_new_uids_and_keeps_values() {
        let d = tdir("copy");
        let mut j = Journal::open(d.clone());
        let (a, b) = two_days(&mut j);
        let (before14, before15) = (live(&mut j, "2026-11-14"), live(&mut j, "2026-11-15"));
        let (count, u) = copy(&mut j, "2026-11-14", "2026-11-15", None, 777).unwrap();
        assert_eq!((count, u.day.as_str(), u.before.len(), u.added.len()), (2, "2026-11-15", 0, 2));
        let (now14, now15) = (live(&mut j, "2026-11-14"), live(&mut j, "2026-11-15"));
        assert_eq!(now14, before14, "the source day is unchanged");
        assert_eq!(&now15[..1], &before15[..], "what was there is untouched");
        assert_eq!(now15.len(), 3, "the deleted entry is not copied");
        for (copy, orig) in now15[1..].iter().zip(&before14[..2]) {
            assert!(copy.uid != orig.uid && copy.uid != a && copy.uid != b && u.added.contains(&copy.uid));
            assert_eq!((copy.day.as_str(), copy.logged_at, copy.deleted_at), ("2026-11-15", 777, None));
            assert_eq!((&copy.name, &copy.source, copy.slot, copy.amount, &copy.unit, copy.grams, copy.basis, copy.per_100g), (&orig.name, &orig.source, orig.slot, orig.amount, &orig.unit, orig.grams, orig.basis, orig.per_100g));
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_never_removes_and_undo_removes_only_copies() {
        let d = tdir("copyundo");
        let mut j = Journal::open(d.clone());
        two_days(&mut j);
        let before = (live(&mut j, "2026-11-14"), live(&mut j, "2026-11-15"));
        let (_, u) = copy(&mut j, "2026-11-14", "2026-11-15", None, 5).unwrap();
        // into a day of another month too
        let (_, u2) = copy(&mut j, "2026-11-14", "2026-12-01", None, 6).unwrap();
        assert_eq!(live(&mut j, "2026-12-01").len(), 2);
        undo(&mut j, &u2, 7).unwrap();
        undo(&mut j, &u, 8).unwrap();
        assert_eq!((live(&mut j, "2026-11-14"), live(&mut j, "2026-11-15"), live(&mut j, "2026-12-01").len()), (before.0, before.1, 0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_of_one_meal_goes_to_the_same_meal() {
        let d = tdir("copymeal");
        let mut j = Journal::open(d.clone());
        two_days(&mut j);
        let (count, _) = copy(&mut j, "2026-11-14", "2026-11-16", Some(MealType::Lunch), 5).unwrap();
        let got = live(&mut j, "2026-11-16");
        assert_eq!((count, got.len(), got[0].slot, got[0].amount), (1, 1, MealType::Lunch, 100.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn copy_refuses_same_day_and_empty_source() {
        let d = tdir("copybad");
        let mut j = Journal::open(d.clone());
        two_days(&mut j);
        assert_eq!(copy(&mut j, "2026-11-14", "2026-11-14", None, 1).unwrap_err(), "Pick another day to copy from.");
        assert_eq!(copy(&mut j, "2026-11-20", "2026-11-14", None, 1).unwrap_err(), "There's nothing to copy from that day.");
        assert_eq!(copy(&mut j, "2026-11-14", "2026-11-16", Some(MealType::Breakfast), 1).unwrap_err(), "There's nothing to copy from that day.");
        assert!(copy(&mut j, "2026-02-30", "2026-11-14", None, 1).is_err());
        assert_eq!(live(&mut j, "2026-11-14").len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn complete_flag_shows_in_the_view() {
        let d = tdir("complete");
        let mut j = Journal::open(d.clone());
        let f = per100(1, n(10.0, None, None));
        assert!(!day_view(&mut j, "2026-11-15", "2026-11-15").unwrap().complete);
        set_complete(&mut j, "2026-11-15", true).unwrap();
        j.save_dirty().unwrap();
        assert!(fs::read_to_string(d.join("2026-11.json")).unwrap().contains("2026-11-15"));
        assert!(Journal::open(d.clone()).month("2026-11").unwrap().complete.contains("2026-11-15"));
        let v = day_view(&mut j, "2026-11-15", "2026-11-15").unwrap();
        assert!(v.complete && !day_view(&mut j, "2026-11-14", "2026-11-15").unwrap().complete);
        // the day stays editable
        let uid = log(&mut j, "2026-11-15", MealType::Lunch, &f, 100.0, "g");
        edit(&mut j, "2026-11-15", &uid, 200.0, "g", MealType::Lunch, 3).unwrap();
        assert!(day_view(&mut j, "2026-11-15", "2026-11-15").unwrap().complete);
        set_complete(&mut j, "2026-11-15", false).unwrap();
        j.save_dirty().unwrap();
        assert!(!day_view(&mut j, "2026-11-15", "2026-11-15").unwrap().complete);
        assert!(!fs::read_to_string(d.join("2026-11.json")).unwrap().contains("\"complete\""));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn previous_day_count_counts_live_entries() {
        let d = tdir("prevcount");
        let mut j = Journal::open(d.clone());
        two_days(&mut j); // the 14th has 2 live and 1 deleted
        assert_eq!(day_view(&mut j, "2026-11-15", "2026-11-15").unwrap().previous_day_count, 2);
        assert_eq!(day_view(&mut j, "2026-11-14", "2026-11-15").unwrap().previous_day_count, 0);
        // across a month end
        log(&mut j, "2026-10-31", MealType::Lunch, &per100(1, n(1.0, None, None)), 10.0, "g");
        assert_eq!(day_view(&mut j, "2026-11-01", "2026-11-15").unwrap().previous_day_count, 1);
        assert_eq!(day_view(&mut j, "2026-11-01", "2026-11-15").unwrap().planned_count, 0);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn planned_meals_list_in_meal_order_with_notes() {
        let today = day_number("2026-11-15").unwrap();
        let meal = |id: u64, day: i32, kind: MealType, recipe_id: Option<u64>, name: &str| Meal { id, day, kind, recipe_id, name: name.into(), ..Default::default() };
        let meals = [
            meal(3, today, MealType::Dinner, Some(1), ""),
            meal(1, today, MealType::Breakfast, None, "Toast"),
            meal(4, today, MealType::Lunch, Some(2), ""),
            meal(2, today, MealType::Lunch, Some(99), "Old salad"),
            meal(5, today + 1, MealType::Lunch, Some(1), ""),
        ];
        let lookup = |id: u64| match id {
            1 => Some(("Soup".to_string(), Some(n(400.0, None, None)))),
            2 => Some(("Rice".to_string(), None)),
            _ => None,
        };
        let got = planned(&meals, today, lookup);
        assert_eq!(got.iter().map(|p| (p.meal_id, p.name.as_str(), p.loggable, p.kcal, p.slot_label)).collect::<Vec<_>>(), [(1, "Toast", false, None, "Breakfast"), (2, "Old salad", false, None, "Lunch"), (4, "Rice", true, None, "Lunch"), (3, "Soup", true, Some(400.0), "Dinner")]);
        assert_eq!(got.iter().map(|p| p.note).collect::<Vec<_>>(), [Some("No recipe linked. Add it by hand."), Some("This recipe isn't in your library any more."), Some("No nutrition estimate yet. It will be logged with values unknown."), None]);
        assert_eq!(day_number("2026-11-16").unwrap() - today, 1);
    }

    #[test]
    fn renamed_my_food_keeps_its_history() {
        let d = tdir("renamed");
        let mut j = Journal::open(d.clone());
        let _turn = crate::nutrition::my_foods_test_lock();
        // odd names: save_my_food sets the estimator's global My Foods, which other tests share
        let mut lib = Default::default(); // inferred from save_my_food: the tracker never names the library type
        crate::nutrition::save_my_food(&mut lib, MyFood { name: "Zxq whey".into(), serving_g: 31.0, ..Default::default() });
        put(&mut j, "2026-11-10", "Zxq whey", 5);
        let id = lib.my_foods[0].id;
        crate::nutrition::save_my_food(&mut lib, MyFood { id, name: "Zxq vanilla whey".into(), serving_g: 31.0, ..Default::default() });
        crate::nutrition::set_my_foods(&[]);
        assert!(lib.my_foods[0].aliases.contains("Zxq whey"));
        let h = history(&mut j, "2026-11-15", &lib.my_foods, &[], &[]).unwrap();
        assert_eq!(h.recent.iter().map(|r| (r.key.clone(), r.name.clone())).collect::<Vec<_>>(), [(format!("my:{id}"), "Zxq vanilla whey".to_string())]);
        let _ = fs::remove_dir_all(&d);
    }

    // ---- the day's budget ----

    fn set_target(j: &mut Journal, from: &str, kcal: i32, f: impl FnOnce(&mut goals::TargetSet)) {
        let mut t = goals::TargetSet { effective_from: from.into(), kcal, floor_kcal: 1200, ..Default::default() };
        f(&mut t);
        goals::put_target_in(j, t);
    }

    fn workout(j: &mut Journal, day: &str, est: Option<f64>, over: Option<f64>) {
        let e = ExerciseEntry { uid: new_uid(), day: day.into(), name: "Walk".into(), est_kcal: est, kcal_override: over, ..Default::default() };
        j.month_mut(&day[..7]).unwrap().exercise.push(e);
    }

    fn lunch(j: &mut Journal, day: &str, id: u64, n: Nutrients) {
        log(j, day, MealType::Lunch, &per100(id, n), 100.0, "g");
    }

    fn view_of(j: &mut Journal, day: &str) -> DayView {
        day_view(j, day, "2026-11-30").unwrap()
    }

    #[test]
    fn budget_remaining_adds_up() {
        let d = tdir("budget-adds");
        let mut j = Journal::open(d.clone());
        set_target(&mut j, "2026-11-01", 2100, |_| {});
        lunch(&mut j, "2026-11-10", 1, n(1700.0, None, None));
        workout(&mut j, "2026-11-10", Some(240.0), None);
        let v = view_of(&mut j, "2026-11-10");
        let b = v.budget.unwrap();
        assert_eq!(v.budget_state, BudgetState::Set);
        assert_eq!((b.remaining, b.label, b.figure, b.over), (Some(640), "Remaining", Some(640), false));
        assert_eq!(b.figure_text.as_deref(), Some("640"));
        lunch(&mut j, "2026-11-11", 2, n(500.0, None, None));
        assert_eq!(view_of(&mut j, "2026-11-11").budget.unwrap().figure_text.as_deref(), Some("1,600"), "printed with the thousands separator");
        assert_eq!(b.formula, "Target 2,100 + Exercise 240 \u{2212} Food 1,700");
        assert_eq!((b.exercise_raw, b.exercise_counted, b.exercise_note), (Some(240), Some(240), None));
        assert_eq!(b.said, "Remaining 640 kcal");
        // half of the workout counts: the term says so and the sum uses the printed 120
        set_target(&mut j, "2026-11-05", 2100, |t| t.addback_pct = 50);
        let b = view_of(&mut j, "2026-11-10").budget.unwrap();
        assert_eq!((b.exercise_counted, b.remaining), (Some(120), Some(520)));
        assert_eq!(b.formula, "Target 2,100 + Exercise 120 (50% of 240) \u{2212} Food 1,700");
        // an override replaces the estimate
        workout(&mut j, "2026-11-11", Some(300.0), Some(100.0));
        assert_eq!(view_of(&mut j, "2026-11-11").budget.unwrap().exercise_raw, Some(100));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn budget_edges() {
        let d = tdir("budget-edges");
        let mut j = Journal::open(d.clone());
        set_target(&mut j, "2026-11-01", 2100, |_| {});
        // over: positive figure, no minus sign
        lunch(&mut j, "2026-11-02", 1, n(2220.0, None, None));
        let b = view_of(&mut j, "2026-11-02").budget.unwrap();
        assert_eq!((b.label, b.figure, b.over, b.remaining), ("Over by", Some(120), true, Some(-120)));
        assert_eq!(b.figure_text.as_deref(), Some("120"), "never a minus sign");
        assert_eq!(b.said, "Over by 120 kcal");
        // food exactly at the target is Remaining 0, not over
        lunch(&mut j, "2026-11-03", 2, n(2100.0, None, None));
        let b = view_of(&mut j, "2026-11-03").budget.unwrap();
        assert_eq!((b.remaining, b.over, b.label), (Some(0), false, "Remaining"));
        // food plus credited exercise exactly the target
        lunch(&mut j, "2026-11-04", 3, n(2340.0, None, None));
        workout(&mut j, "2026-11-04", Some(240.0), None);
        let b = view_of(&mut j, "2026-11-04").budget.unwrap();
        assert_eq!((b.remaining, b.over), (Some(0), false));
        // a day with no food shows the target plus credit; no exercise reads Exercise 0
        workout(&mut j, "2026-11-05", Some(240.0), None);
        assert_eq!(view_of(&mut j, "2026-11-05").budget.unwrap().remaining, Some(2340));
        let b = view_of(&mut j, "2026-11-06").budget.unwrap();
        assert_eq!((b.remaining, b.formula.as_str()), (Some(2100), "Target 2,100 + Exercise 0 \u{2212} Food 0"));
        // the order the workouts were logged in makes no difference to the sum
        workout(&mut j, "2026-11-07", Some(100.0), None);
        workout(&mut j, "2026-11-07", Some(50.0), None);
        workout(&mut j, "2026-11-08", Some(50.0), None);
        workout(&mut j, "2026-11-08", Some(100.0), None);
        assert_eq!(view_of(&mut j, "2026-11-07").budget.unwrap().exercise_counted, Some(150));
        assert_eq!(view_of(&mut j, "2026-11-08").budget.unwrap().exercise_counted, Some(150));
        // a food with no calories: Remaining is unknown, never 0
        lunch(&mut j, "2026-11-09", 4, Nutrients::default());
        let b = view_of(&mut j, "2026-11-09").budget.unwrap();
        assert_eq!((b.remaining, b.figure, b.over), (None, None, false));
        assert_eq!(b.said, "not known: a food has no calories");
        assert_eq!(b.formula, "Target 2,100 + Exercise 0 \u{2212} Food \u{2013}");
        // an exercise with no calories is left out, with a note; alone it makes the term unknown
        workout(&mut j, "2026-11-12", None, None);
        let b = view_of(&mut j, "2026-11-12").budget.unwrap();
        assert_eq!((b.exercise_raw, b.exercise_note), (None, Some("Add a weight to estimate exercise calories.")));
        assert!(b.formula.contains("Exercise \u{2013}"), "{}", b.formula);
        assert_eq!(b.remaining, Some(2100), "an unknown credit adds nothing");
        workout(&mut j, "2026-11-13", None, None);
        workout(&mut j, "2026-11-13", Some(200.0), None);
        let b = view_of(&mut j, "2026-11-13").budget.unwrap();
        assert_eq!((b.exercise_raw, b.exercise_note.is_some()), (Some(200), true));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn budget_states() {
        let d = tdir("budget-states");
        let mut j = Journal::open(d.clone());
        lunch(&mut j, "2026-11-10", 1, n(500.0, None, None));
        let v = view_of(&mut j, "2026-11-10");
        assert_eq!((v.budget_state, v.budget.is_none(), v.budget_note), (BudgetState::Never, true, None));
        set_target(&mut j, "2026-11-05", 2000, |_| {});
        let v = view_of(&mut j, "2026-11-04");
        assert_eq!((v.budget_state, v.budget.is_none()), (BudgetState::Before, true));
        assert_eq!(v.budget_note.as_deref(), Some("No target was set for this day."));
        assert_eq!(view_of(&mut j, "2026-11-05").budget_state, BudgetState::Set);
        j.save_dirty().unwrap();
        // a profile from a newer build: no budget, the day still works
        fs::write(d.join("profile.json"), "{\"format\": 3}").unwrap();
        let mut fresh = Journal::open(d.clone());
        let v = view_of(&mut fresh, "2026-11-10");
        assert_eq!((v.budget_state, v.budget.is_none()), (BudgetState::Unavailable, true));
        assert_eq!(v.slots[1].entries.len(), 1, "the day's foods still list");
        assert_eq!(v.totals.calories, Some(500.0));
        // a damaged one the same
        fs::write(d.join("profile.json"), "{not json").unwrap();
        let mut broken = Journal::open(d.clone());
        assert_eq!(view_of(&mut broken, "2026-11-10").budget_state, BudgetState::Unavailable);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn budget_macros_use_net_carbs() {
        let d = tdir("budget-macros");
        let mut j = Journal::open(d.clone());
        // goals set in grams so the numbers are plain: 150 g protein, 85 g net carbs, 67 g fat
        set_target(&mut j, "2026-11-01", 2100, |t| t.macros = goals::MacroSpec { protein: goals::MacroMode::Grams(150.0), carbs: goals::MacroMode::Grams(85.0), fat: goals::MacroMode::Grams(67.0) });
        let food = Nutrients { calories: Some(100.0), protein_g: Some(96.0), carbs_g: Some(100.0), fiber_g: Some(20.0), fat_g: Some(79.0), ..Default::default() };
        lunch(&mut j, "2026-11-10", 1, food);
        let b = view_of(&mut j, "2026-11-10").budget.unwrap();
        assert_eq!((b.protein.goal, b.protein.unit_text.as_str(), b.protein.said.as_str(), b.protein.over), (150, "/ 150 g", "96 of 150 g protein", false));
        // 100 g carbs less 20 g fibre is 80 net, under the 85 goal; gross would have been over
        assert_eq!((b.carbs.eaten, b.carbs.over, b.carbs.unit_text.as_str()), (Some(80.0), false, "/ 85 g"));
        assert_eq!((b.fat.over, b.fat.unit_text.as_str(), b.fat.said.as_str()), (true, "over by 12 g", "79 of 67 g fat, over by 12 g"));
        // exactly at a goal is not over
        lunch(&mut j, "2026-11-11", 2, Nutrients { calories: Some(10.0), protein_g: Some(150.0), carbs_g: Some(85.0), fat_g: Some(67.0), ..Default::default() });
        let b = view_of(&mut j, "2026-11-11").budget.unwrap();
        assert_eq!((b.protein.over, b.carbs.over, b.fat.over), (false, false, false));
        // a macro some food doesn't report: not over, and it says so
        lunch(&mut j, "2026-11-12", 3, n(10.0, None, None));
        let b = view_of(&mut j, "2026-11-12").budget.unwrap();
        assert_eq!((b.protein.eaten, b.protein.over), (None, false));
        assert!(b.protein.said.contains("not known"), "{}", b.protein.said);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn budget_names_the_template() {
        let d = tdir("budget-template");
        let mut j = Journal::open(d.clone());
        set_target(&mut j, "2026-11-01", 2100, |t| {
            t.templates = vec![goals::Template { id: 1, name: "Training".into(), kcal_offset: 300, ..Default::default() }, goals::Template { id: 2, name: "Fast".into(), kcal_offset: -1000, ..Default::default() }];
            t.floor_kcal = 1500;
            t.week[1] = Some(1); // Monday, Sunday first
            t.week[2] = Some(2); // Tuesday
        });
        let b = view_of(&mut j, "2026-11-16").budget.unwrap(); // a Monday
        assert_eq!((b.target, b.template.as_deref(), b.template_title.as_deref()), (2400, Some("Training"), Some("Training: +300 kcal from your base target")));
        assert_eq!(b.raised_note, None);
        // Tuesday would be 1,100: raised to the floor, and the note says so
        let b = view_of(&mut j, "2026-11-17").budget.unwrap();
        assert_eq!((b.target, b.template.as_deref()), (1500, Some("Fast")));
        assert_eq!(b.raised_note.as_deref(), Some("Raised to 1,500, the lowest Tiffin sets."));
        // a day with no template
        let b = view_of(&mut j, "2026-11-18").budget.unwrap();
        assert_eq!((b.target, b.template, b.template_title), (2100, None, None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn budget_per_kg_protein_follows_the_weight() {
        let d = tdir("budget-perkg");
        let mut j = Journal::open(d.clone());
        let weigh = |j: &mut Journal, day: &str, kg: f64| {
            body::set_value(j, day, "2026-11-30", "weight", kg, 1).unwrap();
        };
        weigh(&mut j, "2026-11-01", 80.0);
        set_target(&mut j, "2026-11-01", 2000, |t| {
            t.basis_weight_kg = Some(80.0);
            t.macros = goals::MacroSpec { protein: goals::MacroMode::PerKg(2.0), carbs: goals::MacroMode::Rest, fat: goals::MacroMode::Percent(30.0) };
        });
        j.save_dirty().unwrap();
        let profile_bytes = fs::read(d.join("profile.json")).unwrap();
        let protein = |j: &mut Journal, day: &str| view_of(j, day).budget.unwrap().protein.goal;
        assert_eq!(protein(&mut j, "2026-11-05"), 160);
        weigh(&mut j, "2026-11-10", 81.0);
        assert_eq!((protein(&mut j, "2026-11-05"), protein(&mut j, "2026-11-10"), protein(&mut j, "2026-11-30")), (160, 162, 162), "the goal follows the weigh-in, no save in between");
        weigh(&mut j, "2026-11-20", 82.0);
        assert_eq!((protein(&mut j, "2026-11-05"), protein(&mut j, "2026-11-10"), protein(&mut j, "2026-11-20")), (160, 162, 164), "a later weigh-in never reaches an earlier day");
        j.save_dirty().unwrap();
        assert_eq!(fs::read(d.join("profile.json")).unwrap(), profile_bytes, "reading the budget never rewrites the profile");
        let _ = fs::remove_dir_all(&d);
    }

    // ---- the Exercise section ----

    fn custom(j: &mut Journal, name: &str, basis: ExerciseBasis) -> exercise::ActivityRef {
        exercise::ActivityRef::Custom(exercise::save_activity(j, crate::tracker::journal::CustomActivity { uid: String::new(), name: name.into(), basis }).unwrap())
    }

    #[test]
    fn day_view_lists_exercise() {
        let d = tdir("exercise-view");
        let mut j = Journal::open(d.clone());
        set_target(&mut j, "2026-11-01", 2100, |_| {});
        body::set_value(&mut j, "2026-11-17", "2026-11-30", "weight", 80.0, 1).unwrap();
        let boulder = custom(&mut j, "Bouldering", ExerciseBasis::Met(5.8));
        lunch(&mut j, "2026-11-17", 1, n(1700.0, None, None));
        assert_eq!(view_of(&mut j, "2026-11-17").exercise, ExerciseSection::default());
        let before = view_of(&mut j, "2026-11-17").budget.unwrap();
        assert_eq!(before.remaining, Some(400));

        exercise::log(&mut j, "2026-11-17", &boulder, 45.0, None, 10).unwrap();
        let v = view_of(&mut j, "2026-11-17");
        let x = &v.exercise.entries[0];
        assert_eq!((x.name.as_str(), x.tag, x.edited, x.minutes_text.as_str(), x.basis_text.as_str()), ("Bouldering", "Custom", false, "45 min", "MET 5.8"));
        assert_eq!((x.weight_text.as_deref(), x.kcal, x.kcal_text.as_deref()), (Some("at 80.0 kg"), Some(288), Some("\u{2212}288")));
        assert_eq!((v.exercise.total_text.as_deref(), v.exercise.counted_text.as_deref()), (Some("\u{2212}288 kcal"), None));
        let b = v.budget.unwrap();
        assert_eq!((b.target, b.remaining), (2100, Some(688)), "Remaining moves, the target stays");

        // half counts: the section says so and Remaining follows the add-back
        set_target(&mut j, "2026-11-05", 2100, |t| t.addback_pct = 50);
        let v = view_of(&mut j, "2026-11-17");
        assert_eq!(v.exercise.counted_text.as_deref(), Some("counts as \u{2212}144"));
        assert_eq!((v.budget.as_ref().unwrap().target, v.budget.unwrap().remaining), (2100, Some(544)));

        // the person's weight unit, the person's own number, an unknown row, and log order
        j.profile_mut().unwrap().units.weight = goals::MassUnit::Lb;
        let rate = custom(&mut j, "Rowing", ExerciseBasis::KcalPerHour(600.0));
        exercise::log(&mut j, "2026-11-17", &rate, 30.0, Some(250.0), 20).unwrap();
        let v = view_of(&mut j, "2026-11-17");
        assert_eq!(v.exercise.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Bouldering", "Rowing"]);
        assert_eq!((v.exercise.entries[0].weight_text.as_deref(), v.exercise.entries[1].weight_text.clone()), (Some("at 176.4 lb"), None));
        assert_eq!((v.exercise.entries[1].edited, v.exercise.entries[1].basis_text.as_str(), v.exercise.entries[1].kcal), (true, "your estimate", Some(250)));
        // no weight on 2026-11-10 (before the weigh-in): unknown, never 0
        exercise::log(&mut j, "2026-11-10", &boulder, 30.0, None, 30).unwrap();
        let v = view_of(&mut j, "2026-11-10");
        assert_eq!((v.exercise.entries[0].kcal, v.exercise.entries[0].kcal_text.clone(), v.exercise.total_text.as_deref(), v.exercise.counted_text.clone()), (None, None, Some("–"), None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn exercise_row_carries_minutes_and_the_frozen_line() {
        let d = tdir("exercise-frozen");
        let mut j = Journal::open(d.clone());
        body::set_value(&mut j, "2026-11-17", "2026-11-30", "weight", 80.0, 1).unwrap();
        let boulder = custom(&mut j, "Bouldering", ExerciseBasis::Met(5.8));
        let rate = custom(&mut j, "Rowing", ExerciseBasis::KcalPerHour(600.0));
        exercise::log(&mut j, "2026-11-17", &boulder, 37.5, None, 10).unwrap();
        exercise::log(&mut j, "2026-11-17", &rate, 30.0, None, 20).unwrap();
        exercise::log(&mut j, "2026-11-10", &boulder, 30.0, None, 30).unwrap();
        let v = view_of(&mut j, "2026-11-17");
        let (a, b) = (&v.exercise.entries[0], &v.exercise.entries[1]);
        assert_eq!((a.minutes, b.minutes), (37.5, 30.0));
        assert_eq!(a.frozen_text, "MET 5.8 and 80.0 kg are kept as they were when you logged this, even if your weight has changed since.");
        assert_eq!(b.frozen_text, "Your own rate, 600 kcal an hour, is kept as you saved it.");
        let none = &view_of(&mut j, "2026-11-10").exercise.entries[0];
        assert_eq!(none.frozen_text, "MET 5.8 is kept as it was when you logged this. No weight was on record, so the calories stay unknown unless you enter your own.");
        let _ = fs::remove_dir_all(&d);
    }

    // ---- exercise and body in copy, undo and the previous-day count ----

    fn seed_exercise(j: &mut Journal, uid: &str, day: &str, deleted: bool) {
        let e = ExerciseEntry {
            uid: uid.into(),
            day: day.into(),
            logged_at: 5,
            modified: 5,
            deleted_at: deleted.then_some(6),
            name: "Hill walk".into(),
            source: ExerciseSource::Custom("a1".into()),
            basis: ExerciseBasis::Met(7.5),
            minutes: 45.0,
            weight_kg: Some(80.0),
            est_kcal: Some(390.0),
            kcal_override: Some(400.0),
        };
        j.month_mut(&day[..7]).unwrap().exercise.push(e);
    }

    fn live_exercise(j: &mut Journal, day: &str) -> Vec<ExerciseEntry> {
        j.month(&day[..7]).unwrap().exercise.iter().filter(|e| e.day == day && e.deleted_at.is_none()).cloned().collect()
    }

    #[test]
    fn copy_includes_exercise() {
        let d = tdir("copy-exercise");
        let mut j = Journal::open(d.clone());
        two_days(&mut j); // the 14th: 2 live foods and a deleted one; the 15th: 1 food
        seed_exercise(&mut j, "x1", "2026-11-14", false);
        seed_exercise(&mut j, "x2", "2026-11-14", true);
        let (count, u) = copy(&mut j, "2026-11-14", "2026-11-16", None, 77).unwrap();
        assert_eq!(count, 3, "two foods and the one live workout");
        assert_eq!((u.added.len(), u.added_exercise.len()), (2, 1));
        let copy_of = live_exercise(&mut j, "2026-11-16").remove(0);
        let first = live_exercise(&mut j, "2026-11-14").remove(0);
        assert_ne!(copy_of.uid, first.uid);
        assert_eq!(u.added_exercise, [copy_of.uid.clone()]);
        assert_eq!(ExerciseEntry { uid: first.uid.clone(), day: "2026-11-14".into(), logged_at: 5, modified: 5, ..copy_of.clone() }, first, "the frozen values travel with it");
        assert_eq!((copy_of.logged_at, copy_of.modified, copy_of.kcal_override, copy_of.weight_kg), (77, 77, Some(400.0), Some(80.0)));
        assert_eq!(day_view(&mut j, "2026-11-16", "2026-11-16").unwrap().exercise.entries.len(), 1, "the copy counts in the day");
        // one undo takes the foods and the workout away together, and nothing else
        undo(&mut j, &u, 88).unwrap();
        assert!(live_exercise(&mut j, "2026-11-16").is_empty());
        assert!(live(&mut j, "2026-11-16").is_empty());
        assert_eq!((live(&mut j, "2026-11-14").len(), live_exercise(&mut j, "2026-11-14").len()), (2, 1));

        // one meal copies no workout
        let (count, _) = copy(&mut j, "2026-11-14", "2026-11-16", Some(MealType::Lunch), 90).unwrap();
        assert_eq!((count, live_exercise(&mut j, "2026-11-16").len()), (1, 0));
        // a day that only had a workout can be copied, and an empty day still can't
        seed_exercise(&mut j, "x3", "2026-11-20", false);
        assert_eq!(copy(&mut j, "2026-11-20", "2026-11-21", None, 91).unwrap().0, 1);
        assert_eq!(copy(&mut j, "2026-11-19", "2026-11-22", None, 92).unwrap_err(), "There's nothing to copy from that day.");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn previous_day_count_counts_exercise() {
        let d = tdir("prevcount-exercise");
        let mut j = Journal::open(d.clone());
        seed_exercise(&mut j, "x1", "2026-11-20", false);
        seed_exercise(&mut j, "x2", "2026-11-20", true);
        assert_eq!(day_view(&mut j, "2026-11-21", "2026-11-21").unwrap().previous_day_count, 1, "Copy Yesterday is offered for a day that only had a workout");
        log(&mut j, "2026-11-20", MealType::Lunch, &per100(1, n(1.0, None, None)), 10.0, "g");
        assert_eq!(day_view(&mut j, "2026-11-21", "2026-11-21").unwrap().previous_day_count, 2);
        // across a month end
        seed_exercise(&mut j, "x4", "2026-10-31", false);
        assert_eq!(day_view(&mut j, "2026-11-01", "2026-11-21").unwrap().previous_day_count, 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn undo_restores_body_lists() {
        let d = tdir("undo-body");
        let mut j = Journal::open(d.clone());
        let body_entry = |uid: &str, value: f64| BodyEntry { uid: uid.into(), day: "2026-11-15".into(), logged_at: 1, modified: 1, measure: "weight".into(), value, ..Default::default() };
        // the change: b1 moved from 79 to 80, b2 was added, a workout was edited and one added
        j.month_mut("2026-11").unwrap().body.extend([body_entry("b1", 80.0), body_entry("b2", 81.0)]);
        seed_exercise(&mut j, "x1", "2026-11-15", false);
        seed_exercise(&mut j, "x2", "2026-11-15", false);
        let old_x1 = ExerciseEntry { minutes: 20.0, ..live_exercise(&mut j, "2026-11-15")[0].clone() };
        let u = DiaryUndo {
            day: "2026-11-15".into(),
            before_body: vec![body_entry("b1", 79.0), body_entry("b3", 82.0)],
            added_body: vec!["b2".into()],
            before_exercise: vec![old_x1.clone()],
            added_exercise: vec!["x2".into()],
            ..Default::default()
        };
        undo(&mut j, &u, 55).unwrap();
        let body: Vec<(String, f64, u64)> = j.month("2026-11").unwrap().body.iter().map(|b| (b.uid.clone(), b.value, b.modified)).collect();
        assert_eq!(body, [("b1".to_string(), 79.0, 55), ("b3".to_string(), 82.0, 55)], "b1 put back, b3 put back (it had been deleted away), b2 removed");
        let ex = live_exercise(&mut j, "2026-11-15");
        assert_eq!((ex.len(), ex[0].uid.as_str(), ex[0].minutes, ex[0].modified), (1, "x1", 20.0, 55));
        let _ = fs::remove_dir_all(&d);
    }
}
