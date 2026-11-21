//! The food search the Foods page and, later, the diary's add-food dialog use; rules only, no I/O.
//!
//! One ranked, typo-tolerant list over the bundled foods, My Foods and cached Open Food Facts
//! products. My Foods and online products are passed in, never read from a global (the recipe
//! estimator's `set_my_foods` is for the estimator only).

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::ingredients;
use crate::model::{MyFood, Nutrients};
use crate::nutrition::{self, DbFood};

pub mod off;
pub mod portions;

use portions::Unit;

/// Most rows one search returns.
pub const MAX_RESULTS: usize = 50;

/// An Open Food Facts product, from a search or the local cache.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct OnlineFood {
    pub code: String,
    pub name: String,
    pub brands: String,
    pub per100: Nutrients,
    pub serving: String,
    pub serving_g: f32,
    pub suspect: bool,
    pub fetched: u64,
    pub last_used: u64,
}

/// One line of the result list. Values are already rounded by [`display`]; `None` is "not reported" (shown as "–").
#[derive(Serialize, Clone, Debug)]
pub struct FoodRow {
    /// `db:<id>`, `my:<id>` or `off:<code>`
    pub key: String,
    pub name: String,
    pub brand: String,
    /// `My Food`, `USDA`, `Canada` or `Open Food Facts`
    pub tag: &'static str,
    pub kcal: Option<f32>,
    pub protein: Option<f32>,
    pub carbs: Option<f32>,
    pub fat: Option<f32>,
    /// What the numbers are for: "per 100 g", or "per 1 scoop (31 g)" for a My Food.
    pub basis: String,
}

/// The one rounding for the window: calories and sodium to whole numbers, everything else to 1 decimal; unknown stays unknown.
pub fn display(n: &Nutrients) -> Nutrients {
    let r = |v: Option<f32>, k: f32| v.map(|x| (x * k).round() / k);
    Nutrients {
        calories: r(n.calories, 1.0),
        protein_g: r(n.protein_g, 10.0),
        fat_g: r(n.fat_g, 10.0),
        carbs_g: r(n.carbs_g, 10.0),
        fiber_g: r(n.fiber_g, 10.0),
        sugar_g: r(n.sugar_g, 10.0),
        sodium_mg: r(n.sodium_mg, 1.0),
    }
}

/// A number with one decimal and no trailing ".0" ("31", "28.4").
pub(crate) fn num1(x: f32) -> String {
    let s = format!("{:.1}", x);
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

// ---------------------------------------------------------------------------------------------
// Scoring

// Ranking weights, tuned against the golden-query test (see 02-04-SUMMARY).
const ALL_MATCHED: f32 = 1.0;
const HEAD_HIT: f32 = 0.3;
const FIRST_TOKEN: f32 = 0.4;
const PRECISION: f32 = 1.0;
const PROCESSED_PENALTY: f32 = 0.5;
const MY_FOOD_BONUS: f32 = 0.05;
/// A food the person logged before wins a tie against an equal-scoring row (D-05).
const LOGGED_BONUS: f32 = 0.05;
/// The food the recipe estimator picks for the query words (its hand-checked matches) is what people usually mean.
const ESTIMATOR_PICK: f32 = 1.5;
/// Words that mark a prepared or processed food; each one the query did not ask for costs `PROCESSED_PENALTY`.
const PROCESSED: &[&str] = &["breaded", "tender", "roll", "nugget", "patty", "fried", "sandwich", "deli"];

fn words(s: &str) -> Vec<String> {
    ingredients::match_key(s).split(' ').filter(|w| !w.is_empty()).map(String::from).collect()
}

/// How well food word `v` stands for query word `q`: exact 1.0, prefix (last query word only) 0.85,
/// a small typo 0.75 - 0.2 per edit; words of 3 letters or fewer are never fuzzy.
fn weight(q: &str, last: bool, v: &str) -> Option<f32> {
    if v == q {
        return Some(1.0);
    }
    let n = q.chars().count();
    if last && n >= 2 && v.starts_with(q) {
        return Some(0.85);
    }
    let max_d = if n <= 3 { 0 } else if n <= 7 { 1 } else { 2 };
    if max_d == 0 || v.chars().count().abs_diff(n) > max_d {
        return None;
    }
    let d = strsim::damerau_levenshtein(v, q);
    (d <= max_d).then(|| 0.75 - 0.2 * d as f32)
}

/// Score of one food (its words, and the words before its first comma) for the query, or `None` when no query word matches.
/// `w(i, word)` is the weight of a food word for query word `i`.
fn score(q: &[String], toks: &[String], head: &[String], w: &dyn Fn(usize, &str) -> Option<f32>) -> Option<f32> {
    let (mut sum, mut matched, mut head_hits) = (0.0, 0, 0);
    for i in 0..q.len() {
        let best = toks.iter().filter_map(|t| w(i, t).map(|x| (x, t))).max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((x, t)) = best {
            sum += x;
            matched += 1;
            if head.iter().any(|h| h == t) {
                head_hits += 1;
            }
        }
    }
    if matched == 0 {
        return None;
    }
    let mut s = sum + PRECISION * matched as f32 / toks.len().max(1) as f32 + HEAD_HIT * head_hits as f32;
    if matched == q.len() {
        s += ALL_MATCHED;
    }
    if toks.first().is_some_and(|t| (0..q.len()).any(|i| w(i, t).is_some())) {
        s += FIRST_TOKEN;
    }
    let extra = toks.iter().filter(|t| PROCESSED.contains(&t.as_str()) && !q.contains(t)).count();
    Some(s - PROCESSED_PENALTY * extra as f32)
}

/// Vocabulary of the bundled foods' words with the foods that hold each, built once.
struct Index {
    vocab: Vec<(String, Vec<u32>)>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut m: HashMap<&str, Vec<u32>> = HashMap::new();
        for (i, f) in nutrition::db_foods().iter().enumerate() {
            let mut seen: HashSet<&str> = HashSet::new();
            for t in &f.tokens {
                if seen.insert(t) {
                    m.entry(t).or_default().push(i as u32);
                }
            }
        }
        Index { vocab: m.into_iter().map(|(k, v)| (k.to_string(), v)).collect() }
    })
}

/// Build the index now (the shell calls this on a background thread at start, so the first keystroke does not pay).
pub fn warm() {
    index();
}

#[derive(Clone, Copy)]
enum Item {
    Db(usize),
    Mine(usize),
    Online(usize),
    Recipe(usize),
}

/// Score of a name and its comma separated aliases, with the My Food bonus (My Foods and recipes both use it).
fn mine_score(q: &[String], name: &str, aliases: &str, pick: bool) -> Option<f32> {
    let w = |i: usize, t: &str| weight(&q[i], i + 1 == q.len(), t);
    let head = words(name.split(',').next().unwrap_or(""));
    let name = words(name);
    let aliases = aliases.split(',').map(words).filter(|a| !a.is_empty());
    let best = std::iter::once(score(q, &name, &head, &w)).chain(aliases.map(|a| score(q, &a, &a, &w))).flatten().max_by(f32::total_cmp);
    // the estimator tries My Foods before its own picks, so a My Food keeps its place above the pick
    best.map(|s| s + MY_FOOD_BONUS + if pick { ESTIMATOR_PICK } else { 0.0 })
}

fn online_score(q: &[String], f: &OnlineFood) -> Option<f32> {
    let w = |i: usize, t: &str| weight(&q[i], i + 1 == q.len(), t);
    let mut toks = words(&f.name);
    let head = words(f.name.split(',').next().unwrap_or(""));
    toks.extend(words(&f.brands));
    score(q, &toks, &head, &w)
}

/// A recipe in the search: its uid, name and what one serving holds (None when it has no estimate yet).
#[derive(Clone, Debug)]
pub struct RecipeFood {
    pub uid: String,
    pub name: String,
    pub per_serving: Option<Nutrients>,
}

/// Find foods for `q` across the bundled foods, `mine` and the cached `online` products: one list, best first,
/// at most [`MAX_RESULTS`] rows. A blank query lists the My Foods by name and nothing else.
pub fn search(q: &str, mine: &[MyFood], online: &[OnlineFood]) -> Vec<FoodRow> {
    search_with(q, mine, online, &[], &[])
}

/// [`search`] with the user's recipes in the list too, and the row keys already `logged` winning ties.
pub fn search_with(q: &str, mine: &[MyFood], online: &[OnlineFood], recipes: &[RecipeFood], logged: &[String]) -> Vec<FoodRow> {
    let qt = words(q);
    if qt.is_empty() {
        let mut m: Vec<&MyFood> = mine.iter().collect();
        m.sort_by_key(|f| f.name.to_lowercase());
        return m.into_iter().take(MAX_RESULTS).map(my_row).collect();
    }
    let foods = nutrition::db_foods();
    let idx = index();
    // vocabulary words each query word could mean, with their weights
    let maps: Vec<HashMap<&str, f32>> = qt
        .iter()
        .enumerate()
        .map(|(i, t)| idx.vocab.iter().filter_map(|(v, _)| weight(t, i + 1 == qt.len(), v).map(|w| (v.as_str(), w))).collect())
        .collect();
    let mut cands: HashSet<u32> = HashSet::new();
    for (v, post) in &idx.vocab {
        if maps.iter().any(|m| m.contains_key(v.as_str())) {
            cands.extend(post);
        }
    }
    let w = |i: usize, t: &str| maps[i].get(t).copied();
    let pick = nutrition::lookup(q);
    // (score, comma segments, source rank, position, item); sorted best first
    let bonus = |key: String| if logged.contains(&key) { LOGGED_BONUS } else { 0.0 };
    let mut hits: Vec<(f32, usize, u8, usize, Item)> = vec![];
    for &i in &cands {
        let f = &foods[i as usize];
        if let Some(s) = score(&qt, &f.tokens, &f.head, &w) {
            let s = if pick == Some(f.name) { s + ESTIMATOR_PICK } else { s } + bonus(format!("db:{}", f.id));
            let rank = match f.source {
                'F' => 1,
                'S' => 2,
                'C' => 3,
                _ => 4,
            };
            hits.push((s, f.name.split(',').count(), rank, i as usize, Item::Db(i as usize)));
        }
    }
    for (i, f) in mine.iter().enumerate() {
        if let Some(s) = mine_score(&qt, &f.name, &f.aliases, pick.is_some()) {
            hits.push((s + bonus(format!("my:{}", f.id)), f.name.split(',').count(), 0, i, Item::Mine(i)));
        }
    }
    for (i, r) in recipes.iter().enumerate() {
        if let Some(s) = mine_score(&qt, &r.name, "", pick.is_some()) {
            hits.push((s + bonus(format!("recipe:{}", r.uid)), r.name.split(',').count(), 0, i, Item::Recipe(i)));
        }
    }
    for (i, f) in online.iter().enumerate() {
        if let Some(s) = online_score(&qt, f) {
            hits.push((s + bonus(format!("off:{}", f.code)), f.name.split(',').count(), 5, i, Item::Online(i)));
        }
    }
    hits.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
    hits.truncate(MAX_RESULTS);
    hits.into_iter()
        .map(|h| match h.4 {
            Item::Db(i) => db_row(&foods[i]),
            Item::Mine(i) => my_row(&mine[i]),
            Item::Online(i) => online_row(&online[i]),
            Item::Recipe(i) => recipe_row(&recipes[i]),
        })
        .collect()
}

fn row(key: String, name: &str, brand: &str, tag: &'static str, n: &Nutrients, basis: String) -> FoodRow {
    let n = display(n);
    FoodRow { key, name: name.to_string(), brand: brand.to_string(), tag, kcal: n.calories, protein: n.protein_g, carbs: n.carbs_g, fat: n.fat_g, basis }
}

pub(crate) fn db_row(f: &DbFood) -> FoodRow {
    let tag = if f.source == 'C' { "Canada" } else { "USDA" };
    row(format!("db:{}", f.id), f.name, "", tag, &f.nutrients(), "per 100 g".into())
}

pub(crate) fn my_row(f: &MyFood) -> FoodRow {
    let serving = if f.serving.trim().is_empty() { "serving" } else { f.serving.trim() };
    let basis = if f.serving_g > 0.0 { format!("per {} ({} g)", serving, num1(f.serving_g)) } else { format!("per {}", serving) };
    row(format!("my:{}", f.id), &f.name, "", "My Food", &f.per_serving, basis)
}

pub(crate) fn online_row(f: &OnlineFood) -> FoodRow {
    row(format!("off:{}", f.code), &f.name, &f.brands, "Open Food Facts", &f.per100, "per 100 g".into())
}

pub(crate) fn recipe_row(r: &RecipeFood) -> FoodRow {
    row(format!("recipe:{}", r.uid), &r.name, "", "Recipe", &r.per_serving.unwrap_or_default(), "per serving".into())
}

// ---------------------------------------------------------------------------------------------
// One food: units, amount, nutrients

pub const GONE: &str = "That food isn't in the list any more.";
const BAD_UNIT: &str = "That unit doesn't work for this food.";
const BAD_AMOUNT: &str = "Enter an amount of 0 or more.";

/// A food opened in the detail panel: the units it supports and where it starts (D-13, D-14).
#[derive(Serialize, Clone, Debug)]
pub struct FoodDetail {
    pub key: String,
    pub name: String,
    pub brand: String,
    pub tag: &'static str,
    pub online: bool,
    pub suspect: bool,
    pub units: Vec<Unit>,
    /// The unit it opens in: its own serving when it has one, otherwise grams.
    pub unit: String,
    /// The amount it opens at: 1 serving or 100 g.
    pub amount: f32,
    #[serde(skip)]
    pub per100: Nutrients,
    /// What one serving holds, for a recipe whose weight is not known (its entries are counted in servings, not grams).
    #[serde(skip)]
    pub per_serving: Option<Nutrients>,
    /// Shown above the amount: what logging this food will do to the day's totals.
    pub warning: Option<&'static str>,
    /// A softer line: the food is fine to log, some figures just are not known.
    pub note: Option<&'static str>,
}

const NO_CALORIES: &str = "No calories on record for this food. Your day's calorie total will show – while it is logged.";
const NO_ESTIMATE: &str = "No nutrition estimate for this recipe yet. You can still log it; every value will show –. Calculate its nutrition on the recipe page to fill it in.";
const SOME_UNKNOWN: &str = "Some values aren't known for this recipe, so some totals will show –.";

/// What an amount of a food weighs and holds. `None` is "not reported" (shown as "–"), never 0.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Amount {
    /// The amount in the asked unit after the crate's rounding; the window writes it into the box only after a unit change.
    pub amount: Option<f32>,
    pub grams: Option<f32>,
    pub nutrients: Nutrients,
}

/// The field that blocks saving a My Food and why (the sentences are from UI-SPEC section 6).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FoodProblem {
    /// A `MyFood` / `Nutrients` key name (`name`, `serving_g`, `calories`, ...) so the form can find its box.
    pub field: &'static str,
    pub message: &'static str,
}

/// The one sentence for a My Food name that another My Food already has (the preview under Name and the save-time refusal both use it).
pub const NAME_TAKEN: &str = "A food with this name already exists. Pick another name, or edit that food.";

/// The My Food form's live line: per 100 g, two warnings that do not block a save, and what does.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Preview {
    pub per100: Option<Nutrients>,
    /// Protein, carbs and fat add up to more than the serving weighs.
    pub heavy: bool,
    /// More than 900 kcal per 100 g.
    pub dense: bool,
    pub problem: Option<FoodProblem>,
}

fn round1(x: f32) -> f32 {
    (x * 10.0).round() / 10.0
}

fn valid_g(g: f32) -> bool {
    g.is_finite() && g > 0.0
}

/// A My Food's values per 100 g, worked out from its label serving; all unknown until the serving has a weight.
fn my_per100(f: &MyFood) -> Nutrients {
    if valid_g(f.serving_g) {
        f.per_serving.scaled(100.0 / f.serving_g)
    } else {
        Nutrients::default()
    }
}

/// The database food's piece units: `each` and `medium` first, at most 4.
fn db_pieces(f: &DbFood) -> Vec<(String, f32)> {
    let mut p: Vec<(String, f32)> = f.counts.iter().map(|(k, g)| (k.to_string(), *g)).collect();
    p.sort_by_key(|(k, _)| match k.as_str() {
        "each" => 0,
        "medium" => 1,
        _ => 2,
    });
    p.truncate(4);
    p
}

/// Open the food `key` (a row's key): its units in the Metric or US order, and where it starts (D-13).
pub fn detail(key: &str, mine: &[MyFood], online: &[OnlineFood], us: bool) -> Result<FoodDetail, String> {
    let make = |key: &str, name: &str, brand: &str, tag, online, suspect, per100: Nutrients, serving: Option<(&str, f32)>, density, pieces: &[(String, f32)]| {
        let units = portions::units(serving, density, pieces, us);
        let (unit, amount) = if units.iter().any(|u| u.id == "serving") { ("serving", 1.0) } else { ("g", 100.0) };
        let warning = per100.calories.is_none().then_some(NO_CALORIES);
        FoodDetail { key: key.into(), name: name.into(), brand: brand.into(), tag, online, suspect, units, unit: unit.into(), amount, per100, per_serving: None, warning, note: None }
    };
    let (kind, id) = key.split_once(':').ok_or(GONE)?;
    match kind {
        "db" => {
            let f = nutrition::db_foods().iter().find(|f| f.id == id).ok_or(GONE)?;
            let tag = if f.source == 'C' { "Canada" } else { "USDA" };
            Ok(make(key, f.name, "", tag, false, false, f.nutrients(), None, f.density, &db_pieces(f)))
        }
        "my" => {
            let f = mine.iter().find(|f| id.parse::<u64>().ok() == Some(f.id)).ok_or(GONE)?;
            let serving = if f.serving.trim().is_empty() { "serving" } else { f.serving.trim() };
            let density = (valid_g(f.serving_g) && valid_g(f.serving_ml)).then(|| f.serving_g / f.serving_ml);
            let pieces = if !f.piece.trim().is_empty() && valid_g(f.piece_g) { vec![(f.piece.trim().to_string(), f.piece_g)] } else { vec![] };
            Ok(make(key, &f.name, "", "My Food", false, false, my_per100(f), Some((serving, f.serving_g)), density, &pieces))
        }
        "off" => {
            let f = online.iter().find(|f| f.code == id).ok_or(GONE)?;
            let serving = if f.serving.trim().is_empty() { "serving" } else { f.serving.trim() };
            Ok(make(key, &f.name, &f.brands, "Open Food Facts", true, f.suspect, f.per100, Some((serving, f.serving_g)), None, &[]))
        }
        _ => Err(GONE.into()),
    }
}

/// A recipe opened as a food, logged by the serving. With its serving weight (`serving_g`, known only when every ingredient weighs)
/// it also offers grams; without one the only unit is the serving and the figures are per serving (D-08, D-09).
pub fn recipe_detail(r: &RecipeFood, serving_g: Option<f32>, us: bool) -> FoodDetail {
    let known = r.per_serving.unwrap_or_default();
    let weight = serving_g.filter(|g| valid_g(*g));
    let (units, per100, per_serving) = match weight {
        Some(g) => (portions::units(Some(("serving", g)), None, &[], us), known.scaled(100.0 / g), None),
        None => (vec![Unit { id: "serving".into(), label: "serving".into(), grams: 0.0 }], Nutrients::default(), Some(known)),
    };
    let some_unknown = r.per_serving.is_some_and(|n| [n.calories, n.protein_g, n.carbs_g, n.fat_g, n.fiber_g, n.sugar_g, n.sodium_mg].contains(&None));
    FoodDetail {
        key: format!("recipe:{}", r.uid),
        name: r.name.clone(),
        brand: String::new(),
        tag: "Recipe",
        online: false,
        suspect: false,
        units,
        unit: "serving".into(),
        amount: 1.0,
        per100,
        per_serving,
        warning: r.per_serving.is_none().then_some(NO_ESTIMATE),
        note: some_unknown.then_some(SOME_UNKNOWN),
    }
}

/// `amount` of `unit`, or, when `from` names another unit, the amount typed in `from` converted to `unit` keeping the weight.
/// What was typed is used as typed (0.25 cup is 0.25 cup); only an amount converted from another unit is rounded to 1 decimal,
/// and that rounded amount is what goes back in the box, so the box, the weight line and the figures agree. An empty or 0 amount gives no weight and every nutrient unknown (D-15).
pub fn amount(d: &FoodDetail, amount: Option<f32>, unit: &str, from: Option<&str>) -> Result<Amount, String> {
    let find = |id: &str| d.units.iter().find(|u| u.id == id).ok_or_else(|| BAD_UNIT.to_string());
    let to = find(unit)?;
    let src = match from {
        Some(f) if f != unit => Some(find(f)?),
        _ => None,
    };
    let none = |amount| Ok(Amount { amount, grams: None, nutrients: Nutrients::default() });
    let Some(a) = amount else { return none(None) };
    if !a.is_finite() || a < 0.0 {
        return Err(BAD_AMOUNT.into());
    }
    // a unit with no weight (a recipe serving of unknown size) can't be converted, and measures no grams
    if src.is_some_and(|s| s.grams <= 0.0) || to.grams <= 0.0 && src.is_some() {
        return Err(BAD_UNIT.into());
    }
    if to.grams <= 0.0 {
        let per = d.per_serving.unwrap_or_default();
        return Ok(Amount { amount: Some(a), grams: None, nutrients: display(&per.scaled(a)) });
    }
    let a = src.map_or(a, |s| round1(a * s.grams / to.grams));
    if a == 0.0 {
        return none(Some(0.0));
    }
    let grams = a * to.grams; // unrounded: only what is shown is rounded
    Ok(Amount { amount: Some(a), grams: Some(round1(grams)), nutrients: display(&d.per100.scaled(grams / 100.0)) })
}

/// What blocks saving a My Food, the first problem in form order (D-06). The form shows it through [`preview`]; the save command runs it too.
pub fn check_food(f: &MyFood) -> Result<(), FoodProblem> {
    let problem = |field, message| Err(FoodProblem { field, message });
    if f.name.trim().is_empty() {
        return problem("name", "Give this food a name.");
    }
    if !valid_g(f.serving_g) {
        return problem("serving_g", "Enter the serving weight in grams, from the label.");
    }
    let n = &f.per_serving;
    if n.calories.is_none() {
        return problem("calories", "Enter the calories for one serving.");
    }
    let both = "Add both the piece name and its weight, or leave both empty.";
    match (f.piece.trim().is_empty(), f.piece_g > 0.0) {
        (false, false) => return problem("piece_g", both),
        (true, true) => return problem("piece", both),
        _ => {}
    }
    let bad = |v: f32| !v.is_finite() || v < 0.0;
    let opt = |v: Option<f32>| v.is_some_and(bad);
    let checks = [
        ("serving_ml", bad(f.serving_ml)),
        ("calories", opt(n.calories)),
        ("protein_g", opt(n.protein_g)),
        ("carbs_g", opt(n.carbs_g)),
        ("fat_g", opt(n.fat_g)),
        ("fiber_g", opt(n.fiber_g)),
        ("sugar_g", opt(n.sugar_g)),
        ("sodium_mg", opt(n.sodium_mg)),
        ("piece_g", bad(f.piece_g)),
    ];
    match checks.into_iter().find(|c| c.1) {
        Some((field, _)) => problem(field, "Nutrition numbers can't be negative."),
        None => Ok(()),
    }
}

/// The My Food form's live line (D-05, UI-SPEC section 6): per 100 g once the serving weight and calories are known,
/// the two warnings, and what blocks a save. The form keeps no copy of these rules.
pub fn preview(f: &MyFood) -> Preview {
    let g = f.serving_g;
    let n = &f.per_serving;
    let known = valid_g(g) && n.calories.is_some();
    let macros: f32 = [n.protein_g, n.carbs_g, n.fat_g].into_iter().flatten().sum();
    Preview {
        per100: known.then(|| display(&my_per100(f))),
        heavy: valid_g(g) && macros > g,
        dense: valid_g(g) && n.calories.is_some_and(|c| c / g * 100.0 > 900.0),
        problem: check_food(f).err(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn plain_chicken_breast(n: &str) -> bool {
        let n = n.to_lowercase();
        n.contains("chicken") && n.contains("breast") && !["breaded", "tenders", "roll", "nugget", "patty", "fried", "sandwich"].iter().any(|w| n.contains(w))
    }

    /// (query, does this food name satisfy the query?). Names are matched by rule so the test survives table updates.
    fn golden() -> Vec<(&'static str, fn(&str) -> bool)> {
        let oats: fn(&str) -> bool = |n| n.starts_with("Oats") || n.starts_with("Cereals, oats") || n.starts_with("Oatmeal");
        let yogurt: fn(&str) -> bool = |n| n.contains("Yogurt, Greek");
        vec![
            ("egg", |n| n.starts_with("Egg, whole")),
            ("banana", |n| n.starts_with("Bananas, raw")),
            ("chicken breast", plain_chicken_breast),
            ("chiken brest", plain_chicken_breast),
            ("rice", |n| n.starts_with("Rice, white")),
            ("oats", oats),
            ("oatmeel", oats),
            ("whole milk", |n| n.contains("Milk, whole")),
            ("peanut buter", |n| n.starts_with("Peanut butter")),
            ("tofu", |n| n.starts_with("Tofu")),
            ("greek yogurt", yogurt),
            ("grek yoghurt", yogurt),
            ("olive oil", |n| n.to_lowercase().contains("olive") && n.starts_with("Oil")),
            ("lentils", |n| n.starts_with("Lentils")),
            ("cheddar", |n| n.to_lowercase().contains("cheddar")),
            ("brocoli", |n| n.starts_with("Broccoli")),
        ]
    }

    fn my(id: u64, name: &str) -> MyFood {
        MyFood { id, name: name.into(), serving: "1 scoop".into(), serving_g: 31.0, per_serving: Nutrients { calories: Some(120.0), ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn search_golden_queries() {
        for (q, ok) in golden() {
            let rows = search(q, &[], &[]);
            assert!(rows.iter().take(5).any(|r| ok(&r.name)), "{q:?}: no expected food in the top 5: {:?}", rows.iter().take(8).map(|r| &r.name).collect::<Vec<_>>());
        }
    }

    #[test]
    fn typo_reaches_clean_query_food() {
        let g = golden();
        let pred = |q: &str| g.iter().find(|x| x.0 == q).unwrap().1;
        for (typo, clean) in [("chiken brest", "chicken breast"), ("grek yoghurt", "greek yogurt"), ("peanut buter", "peanut butter"), ("brocoli", "broccoli"), ("oatmeel", "oatmeal")] {
            let ok = pred(typo);
            let hit = search(typo, &[], &[]).into_iter().take(5).find(|r| ok(&r.name)).unwrap_or_else(|| panic!("{typo:?}: nothing expected in the top 5"));
            let clean_keys: Vec<String> = search(clean, &[], &[]).into_iter().take(10).map(|r| r.key).collect();
            assert!(clean_keys.contains(&hit.key), "{typo:?} found {:?}, which {clean:?} does not list in its top 10", hit.name);
        }
    }

    #[test]
    fn my_foods_win_ties_and_empty_query_lists_them() {
        let mine = vec![my(2, "chicken breast"), my(1, "Apple pie")];
        let rows = search("chicken breast", &mine, &[]);
        assert_eq!(rows[0].key, "my:2");
        assert_eq!(rows[0].tag, "My Food");
        let all = search("  ", &mine, &[]);
        assert_eq!(all.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["my:1", "my:2"]);
        assert!(search("", &[], &[]).is_empty());
    }

    #[test]
    fn results_are_capped_at_50() {
        assert_eq!(search("chicken", &[], &[]).len(), MAX_RESULTS);
    }

    #[test]
    fn cached_online_foods_are_found() {
        let o = OnlineFood { code: "0123".into(), name: "Greek Yogurt".into(), brands: "Fage".into(), per100: Nutrients { calories: Some(59.0), ..Default::default() }, ..Default::default() };
        let rows = search("greek yogurt", &[], &[o]);
        let r = rows.iter().find(|r| r.key == "off:0123").expect("cached product not found");
        assert_eq!((r.tag, r.brand.as_str(), r.kcal), ("Open Food Facts", "Fage", Some(59.0)));
    }

    #[test]
    fn rows_carry_tags_basis_and_unknowns() {
        let foods = nutrition::db_foods();
        let first = |c: char| db_row(foods.iter().find(|f| f.source == c).unwrap());
        assert_eq!((first('F').tag, first('S').tag, first('N').tag, first('C').tag), ("USDA", "USDA", "USDA", "Canada"));
        assert_eq!(first('S').basis, "per 100 g");
        // a food that does not report one of the four row values shows None there, not 0
        let gap = foods.iter().find(|f| f.known & 0b1111 != 0b1111).expect("a food missing a macro");
        let r = db_row(gap);
        let shown = [r.kcal, r.protein, r.fat, r.carbs];
        let missing = (0..4).find(|i| gap.known >> i & 1 == 0).unwrap();
        assert_eq!(shown[missing], None);
        assert_eq!(my_row(&my(1, "Whey")).basis, "per 1 scoop (31 g)");
        let mut m = my(1, "Whey");
        m.serving_g = 0.0;
        assert_eq!(my_row(&m).basis, "per 1 scoop");
        assert_eq!(search("whey", &[my(1, "Whey")], &[])[0].protein, None);
    }

    #[test]
    fn display_rounds_for_the_window() {
        let n = Nutrients { calories: Some(164.6), protein_g: Some(31.04), fat_g: Some(3.57), sodium_mg: Some(437.6), ..Default::default() };
        let d = display(&n);
        assert_eq!((d.calories, d.protein_g, d.fat_g, d.sodium_mg, d.sugar_g), (Some(165.0), Some(31.0), Some(3.6), Some(438.0), None));
    }

    fn ids(d: &FoodDetail) -> Vec<&str> {
        d.units.iter().map(|u| u.id.as_str()).collect()
    }

    fn db_detail(name: &str, us: bool) -> FoodDetail {
        let f = nutrition::db_foods().iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no food {name}"));
        detail(&format!("db:{}", f.id), &[], &[], us).unwrap()
    }

    fn whey() -> MyFood {
        MyFood { per_serving: Nutrients { calories: Some(120.0), protein_g: Some(24.0), ..Default::default() }, ..my(1, "Whey") }
    }

    fn my_detail(f: &MyFood) -> FoodDetail {
        detail(&format!("my:{}", f.id), &[f.clone()], &[], false).unwrap()
    }

    #[test]
    fn portions_hide_unsupported_units() {
        let foods = nutrition::db_foods();
        let one = |f: &DbFood| detail(&format!("db:{}", f.id), &[], &[], false).unwrap();
        let bare = foods.iter().find(|f| f.density.is_none() && f.counts.is_empty()).unwrap();
        assert_eq!(ids(&one(bare)), ["g", "oz"]);
        let dense = foods.iter().find(|f| f.density.is_some() && f.counts.is_empty()).unwrap();
        assert_eq!(ids(&one(dense)), ["g", "ml", "tbsp", "cup", "oz"]);
        assert!(ids(&db_detail("Egg, whole, raw, fresh", false)).contains(&"piece:each"));
        let d = my_detail(&whey());
        assert_eq!(ids(&d), ["g", "serving", "oz"]);
        assert_eq!(d.units[1].label, "1 scoop (31 g)");
        let d = my_detail(&MyFood { piece: "slice".into(), piece_g: 28.0, ..whey() });
        assert_eq!(ids(&d), ["g", "serving", "piece:slice", "oz"]);
        assert_eq!(d.units[2].label, "slice (28 g)");
        let d = my_detail(&MyFood { serving_ml: 30.0, ..whey() });
        assert_eq!(ids(&d), ["g", "ml", "serving", "tbsp", "cup", "oz"]);
        assert!((d.units[1].grams - 31.0 / 30.0).abs() < 1e-4);
        // a serving without a weight has no serving unit and no per-100 g values
        let d = my_detail(&MyFood { serving_g: 0.0, ..whey() });
        assert_eq!(ids(&d), ["g", "oz"]);
        assert_eq!(d.per100, Nutrients::default());
    }

    #[test]
    fn unit_order_follows_metric_or_us() {
        let pieces = vec![("slice".to_string(), 28.0)];
        let order = |us| portions::units(Some(("1 scoop", 31.0)), Some(1.0), &pieces, us).into_iter().map(|u| u.id).collect::<Vec<_>>();
        assert_eq!(order(false), ["g", "ml", "serving", "piece:slice", "tbsp", "cup", "oz"]);
        assert_eq!(order(true), ["oz", "cup", "tbsp", "g", "ml", "serving", "piece:slice"]);
    }

    #[test]
    fn default_amount_is_the_serving_or_100_g() {
        let d = db_detail("Egg, whole, raw, fresh", false);
        assert_eq!((d.unit.as_str(), d.amount), ("g", 100.0));
        let d = my_detail(&whey());
        assert_eq!((d.unit.as_str(), d.amount), ("serving", 1.0));
        let o = OnlineFood { code: "9".into(), name: "Bar".into(), serving: "1 bar".into(), serving_g: 40.0, ..Default::default() };
        let d = detail("off:9", &[], &[o.clone()], false).unwrap();
        assert_eq!((d.unit.as_str(), d.amount, d.online), ("serving", 1.0, true));
        let d = detail("off:9", &[], &[OnlineFood { serving_g: 0.0, ..o }], false).unwrap();
        assert_eq!((d.unit.as_str(), d.amount), ("g", 100.0));
        for key in ["db:nope", "my:7", "zzz"] {
            assert_eq!(detail(key, &[], &[], false).unwrap_err(), "That food isn't in the list any more.");
        }
    }

    #[test]
    fn amount_counts_servings_and_keeps_unknown() {
        let m = whey();
        let d = my_detail(&m);
        let a = amount(&d, Some(2.0), "serving", None).unwrap();
        assert_eq!((a.amount, a.grams), (Some(2.0), Some(62.0)));
        assert_eq!(a.nutrients, display(&m.per_serving.scaled(2.0)));
        assert_eq!((a.nutrients.calories, a.nutrients.fat_g), (Some(240.0), None));
        assert_eq!(amount(&d, Some(1.0), "oz", None).unwrap().grams, Some(28.3));
    }

    #[test]
    fn zero_or_empty_amount_is_all_unknown_and_negative_is_refused() {
        let d = my_detail(&whey());
        for a in [None, Some(0.0)] {
            let r = amount(&d, a, "g", None).unwrap();
            assert_eq!((r.grams, r.nutrients), (None, Nutrients::default()));
        }
        for bad in [-1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(amount(&d, Some(bad), "g", None).unwrap_err(), "Enter an amount of 0 or more.");
        }
        let no = "That unit doesn't work for this food.";
        assert_eq!(amount(&d, Some(1.0), "cup", None).unwrap_err(), no);
        assert_eq!(amount(&d, Some(1.0), "g", Some("cup")).unwrap_err(), no);
    }

    #[test]
    fn changing_unit_keeps_the_weight() {
        let d = db_detail("Egg, whole, raw, fresh", false);
        let a = amount(&d, Some(100.0), "oz", Some("g")).unwrap();
        assert_eq!((a.amount, a.grams), (Some(3.5), Some(99.2)));
        let d = my_detail(&whey());
        let a = amount(&d, Some(62.0), "serving", Some("g")).unwrap();
        assert_eq!((a.amount, a.grams), (Some(2.0), Some(62.0)));
        for from in [None, Some("g")] {
            assert_eq!(amount(&d, Some(3.14), "g", from).unwrap().amount, Some(3.14), "a typed amount is used as typed");
        }
        // 0.25 serving is a quarter of the figures, not 0.3 of them
        let a = amount(&d, Some(0.25), "serving", None).unwrap();
        assert_eq!((a.amount, a.nutrients.calories), (Some(0.25), Some(30.0)));
        assert!(amount(&d, Some(0.04), "g", None).unwrap().nutrients.calories.is_some(), "a tiny amount is still a known value");
    }

    #[test]
    fn one_cup_agrees_with_the_recipe_estimate() {
        let r = nutrition::calculate(&crate::model::Recipe { ingredients: "1 cup whole milk".into(), ..Default::default() });
        let name = r.lines[0].matched.clone().expect("whole milk matched");
        let a = amount(&db_detail(&name, false), Some(1.0), "cup", None).unwrap();
        let (got, est) = (a.nutrients.calories.unwrap(), r.total.calories.expect("whole milk calories are known"));
        assert!((got - est).abs() / est < 0.02, "1 cup is {got} kcal, the recipe estimate {est}");
    }

    #[test]
    fn my_food_preview_per_100_g_and_warnings() {
        let p = preview(&whey());
        assert_eq!(p.per100, Some(Nutrients { calories: Some(387.0), protein_g: Some(77.4), ..Default::default() }));
        assert_eq!((p.heavy, p.dense, p.problem), (false, false, None));
        assert_eq!(preview(&MyFood { serving_g: 0.0, ..whey() }).per100, None);
        assert_eq!(preview(&MyFood { per_serving: Nutrients::default(), ..whey() }).per100, None);
        let heavy = MyFood { per_serving: Nutrients { calories: Some(100.0), protein_g: Some(20.0), carbs_g: Some(15.0), fat_g: Some(6.0), ..Default::default() }, ..whey() };
        assert!(preview(&heavy).heavy);
        let dense = MyFood { per_serving: Nutrients { calories: Some(400.0), ..Default::default() }, ..whey() };
        let p = preview(&dense);
        assert!(p.dense && !p.heavy);
        assert!(p.problem.is_none(), "warnings do not block a save");
    }

    #[test]
    fn my_food_check_names_the_field() {
        let field = |f: MyFood| {
            let e = check_food(&f).unwrap_err();
            assert_eq!(preview(&f).problem, Some(e.clone()), "preview carries the same problem");
            (e.field, e.message)
        };
        let both = "Add both the piece name and its weight, or leave both empty.";
        let neg = "Nutrition numbers can't be negative.";
        assert_eq!(field(MyFood { name: "  ".into(), ..whey() }), ("name", "Give this food a name."));
        for g in [0.0, f32::NAN] {
            assert_eq!(field(MyFood { serving_g: g, ..whey() }), ("serving_g", "Enter the serving weight in grams, from the label."));
        }
        assert_eq!(field(MyFood { per_serving: Nutrients::default(), ..whey() }), ("calories", "Enter the calories for one serving."));
        assert_eq!(field(MyFood { piece: "slice".into(), ..whey() }), ("piece_g", both));
        assert_eq!(field(MyFood { piece_g: 28.0, ..whey() }), ("piece", both));
        let mut f = whey();
        f.per_serving.protein_g = Some(-1.0);
        assert_eq!(field(f), ("protein_g", neg));
        assert_eq!(field(MyFood { serving_ml: f32::INFINITY, ..whey() }), ("serving_ml", neg));
        // several problems: the first in form order
        assert_eq!(field(MyFood { name: "".into(), serving_g: 0.0, ..whey() }).0, "name");
        assert_eq!(field(MyFood { serving_g: 0.0, per_serving: Nutrients::default(), ..whey() }).0, "serving_g");
        let ok = MyFood { per_serving: Nutrients { calories: Some(120.0), ..Default::default() }, ..whey() };
        assert_eq!(check_food(&ok), Ok(()));
        assert_eq!(preview(&ok).problem, None);
    }

    #[test]
    fn search_is_fast() {
        warm();
        let mut times: Vec<u128> = vec![];
        for (q, _) in golden() {
            let t = Instant::now();
            let n = search(q, &[], &[]).len();
            times.push(t.elapsed().as_micros());
            println!("{q:>16}: {:>6} us, {n} rows", times.last().unwrap());
        }
        times.sort();
        let median_ms = times[times.len() / 2] as f64 / 1000.0;
        println!("median {median_ms:.2} ms");
        let limit = if cfg!(debug_assertions) { 150.0 } else { 50.0 };
        assert!(median_ms < limit, "median {median_ms} ms is over {limit} ms");
    }

    fn tomato_soup(per_serving: Option<Nutrients>) -> RecipeFood {
        RecipeFood { uid: "u1".into(), name: "Tomato soup".into(), per_serving }
    }

    fn soup_nutrients() -> Nutrients {
        Nutrients { calories: Some(400.0), protein_g: Some(20.0), fat_g: Some(10.0), carbs_g: Some(50.0), fiber_g: None, sugar_g: Some(5.0), sodium_mg: Some(300.0) }
    }

    #[test]
    fn recipe_detail_offers_grams_only_with_a_weight() {
        let weighed = recipe_detail(&tomato_soup(Some(soup_nutrients())), Some(250.0), false);
        assert_eq!((weighed.key.as_str(), weighed.tag, weighed.unit.as_str(), weighed.amount), ("recipe:u1", "Recipe", "serving", 1.0));
        assert_eq!(ids(&weighed), ["g", "serving", "oz"]);
        assert_eq!(weighed.units[1].label, "serving (250 g)");
        let a = amount(&weighed, Some(1.5), "serving", None).unwrap();
        assert_eq!((a.grams, a.nutrients.calories, a.nutrients.fiber_g), (Some(375.0), Some(600.0), None));
        assert_eq!(weighed.note, Some("Some values aren't known for this recipe, so some totals will show –."));
        assert_eq!(weighed.warning, None);

        let light = recipe_detail(&tomato_soup(Some(soup_nutrients())), None, false);
        assert_eq!(ids(&light), ["serving"]);
        let a = amount(&light, Some(2.0), "serving", None).unwrap();
        assert_eq!((a.grams, a.nutrients.calories), (None, Some(800.0)));
        assert_eq!(amount(&light, Some(100.0), "g", None).unwrap_err(), "That unit doesn't work for this food.");
        assert_eq!(amount(&light, Some(1.0), "serving", Some("g")).unwrap_err(), "That unit doesn't work for this food.");
        assert_eq!(amount(&light, None, "serving", None).unwrap().amount, None);

        let none = recipe_detail(&tomato_soup(None), None, false);
        assert_eq!(none.warning, Some("No nutrition estimate for this recipe yet. You can still log it; every value will show –. Calculate its nutrition on the recipe page to fill it in."));
        assert_eq!(amount(&none, Some(1.0), "serving", None).unwrap().nutrients, Nutrients::default());
    }

    #[test]
    fn recipes_join_the_search() {
        let soup = tomato_soup(Some(soup_nutrients()));
        let rows = search_with("tomato soup", &[], &[], std::slice::from_ref(&soup), &[]);
        let r = rows.iter().find(|r| r.key == "recipe:u1").expect("the recipe is listed");
        assert_eq!((r.tag, r.basis.as_str(), r.kcal, r.name.as_str()), ("Recipe", "per serving", Some(400.0), "Tomato soup"));
        let blank = search_with("", &[my(1, "Whey")], &[], std::slice::from_ref(&soup), &[]);
        assert_eq!(blank.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(), ["my:1"]);
        assert!(!search("tomato soup", &[], &[]).iter().any(|r| r.tag == "Recipe"), "plain search never lists recipes");
        // foods logged before win a tie against an equal-scoring row
        let twins = [my(1, "Bowl"), my(2, "Bowl")];
        assert_eq!(search_with("bowl", &twins, &[], &[], &[])[0].key, "my:1");
        assert_eq!(search_with("bowl", &twins, &[], &[], &["my:2".to_string()])[0].key, "my:2");
        // a recipe row with no estimate shows unknowns
        let bare = search_with("tomato soup", &[], &[], &[tomato_soup(None)], &[]);
        assert_eq!(bare.iter().find(|r| r.key == "recipe:u1").unwrap().kcal, None);
    }

    #[test]
    fn unknown_calories_warn() {
        let o = OnlineFood { code: "1".into(), name: "Mystery bar".into(), ..Default::default() };
        let d = detail("off:1", &[], &[o.clone()], false).unwrap();
        assert_eq!(d.warning, Some("No calories on record for this food. Your day's calorie total will show – while it is logged."));
        let known = OnlineFood { per100: Nutrients { calories: Some(59.0), ..Default::default() }, ..o };
        assert_eq!(detail("off:1", &[], &[known], false).unwrap().warning, None);
        assert_eq!(my_detail(&whey()).warning, None);
    }
}
