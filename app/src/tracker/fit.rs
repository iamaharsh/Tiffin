//! Foods and recipes that fit the protein, net carbs and fat left today: everyday bundled foods, My Foods and the
//! user's recipes, each at the largest sensible amount that stays within what is left, best fill first. Rules only.

use crate::foods::{self, RecipeFood};
use crate::model::{MyFood, Nutrients};
use crate::nutrition;
use crate::tracker::diary::{self, Budget};
use serde::Serialize;

/// Everyday bundled foods offered as suggestions (eaten as they are, not cooking ingredients), each with its largest ordinary
/// portion in grams (a scoop of whey, not 110 g). Exact database names; a test checks.
const EVERYDAY: &[(&str, f32)] = &[
    ("Yogurt, Greek, plain, nonfat", 200.0),
    ("Yogurt, Greek, plain, whole milk", 200.0),
    ("Cheese, cottage, creamed, large or small curd", 200.0),
    ("Egg, whole, raw, fresh", 200.0),
    ("Egg, white, raw, fresh", 200.0),
    ("Chicken, breast, boneless, skinless, raw", 200.0),
    ("Turkey, ground, 93% lean, 7% fat, raw", 200.0),
    ("Fish, salmon, Atlantic, farmed, raw", 200.0),
    ("Fish, tuna, light, canned in water, drained solids", 200.0),
    ("Crustaceans, shrimp, raw", 200.0),
    ("Tofu, raw, firm, prepared with calcium sulfate", 200.0),
    ("Tempeh", 200.0),
    ("Edamame, frozen, prepared", 200.0),
    ("Lentils, mature seeds, cooked, boiled, without salt", 200.0),
    ("Beans, black, mature seeds, cooked, boiled, without salt", 200.0),
    ("Chickpeas (garbanzo beans, bengal gram), mature seeds, cooked, boiled, without salt", 200.0),
    ("Hummus, commercial", 100.0),
    ("Quinoa, cooked", 200.0),
    ("Rice, white, long-grain, regular, enriched, cooked", 200.0),
    ("Rice, brown, long-grain, cooked (Includes foods for USDA's Food Distribution Program)", 200.0),
    ("Pasta, cooked, enriched, without added salt", 200.0),
    ("Cereals, oats, regular and quick, not fortified, dry", 80.0),
    ("Bread, whole-wheat, commercially prepared", 100.0),
    ("Potatoes, flesh and skin, raw", 200.0),
    ("Sweet potato, raw, unprepared (Includes foods for USDA's Food Distribution Program)", 200.0),
    ("Bananas, raw", 200.0),
    ("Apples, raw, with skin (Includes foods for USDA's Food Distribution Program)", 200.0),
    ("Blueberries, raw", 200.0),
    ("Strawberries, raw", 200.0),
    ("Avocados, raw, all commercial varieties", 150.0),
    ("Nuts, almonds", 40.0),
    ("Nuts, walnuts, english", 40.0),
    ("Peanut butter, smooth style, without salt", 32.0),
    ("Milk, reduced fat, fluid, 2% milkfat, with added vitamin A and vitamin D", 250.0),
    ("Soymilk, original and vanilla, unfortified", 250.0),
    ("Cheese, cheddar", 60.0),
    ("Cheese, mozzarella, whole milk", 60.0),
    ("Beverages, Protein powder whey based", 40.0),
];

/// A macro with less than this left is nearly met: it no longer counts toward the fill.
const NEARLY_MET_G: f32 = 5.0;
/// A suggestion may pass what is left of any one macro by at most this much (lean chicken still has a little fat).
const SLACK_G: f32 = 3.0;
// one ordinary portion at most: early in the day "the most that fits" would be 400 g of tempeh
const MIN_GRAMS: f32 = 30.0;
const MAX_SERVINGS: f32 = 1.0;
/// A suggestion must cover at least this mean share of what is left; less adds next to nothing.
const MIN_FILL: f32 = 0.1;
pub const MAX_FOODS: usize = 4;
pub const MAX_RECIPES: usize = 3;

pub const NEARLY_MET: &str = "Today's protein, carbs and fat are nearly met.";
pub const LEFT_UNKNOWN: &str = "What's left can't be worked out: a food today has no protein, carbs or fat listed.";
pub const NOTHING_FITS: &str = "Nothing in your foods or recipes fits what's left today.";

/// One suggestion at the amount that fits.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Fit {
    /// The food search key (`db:`, `my:` or `recipe:`); unique, so the page keys its list by it.
    pub key: String,
    pub name: String,
    pub tag: &'static str,
    /// "150 g", "1 serving", "1.5 servings"
    pub amount_text: String,
    /// "31 g protein · 4 g net carbs · 3 g fat · 165 kcal"
    pub macros_text: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Fits {
    pub foods: Vec<Fit>,
    pub recipes: Vec<Fit>,
    /// Said in place of the lists when there is nothing to suggest.
    pub note: Option<&'static str>,
}

/// What a candidate holds for one unit: 1 g for a bundled food, one serving otherwise.
struct Cand {
    key: String,
    name: String,
    tag: &'static str,
    per: Nutrients,
    /// The largest portion in grams for a bundled food; None for a food counted in servings.
    max_g: Option<f32>,
    recipe: bool,
}

/// Suggestions for what is left in `b` (None, no target today, suggests nothing and says nothing: Home already says why).
pub fn fits(b: Option<&Budget>, mine: &[MyFood], recipes: &[RecipeFood]) -> Fits {
    let Some(b) = b else { return Fits::default() };
    let left = [&b.protein, &b.carbs, &b.fat].map(|g| g.eaten.map(|e| (g.goal as f32 - e).max(0.0)));
    let [Some(lp), Some(lc), Some(lf)] = left else { return Fits { note: Some(LEFT_UNKNOWN), ..Default::default() } };
    let left = [lp, lc, lf];
    if left.iter().all(|&l| l < NEARLY_MET_G) {
        return Fits { note: Some(NEARLY_MET), ..Default::default() };
    }
    let db = nutrition::db_foods();
    let everyday = EVERYDAY.iter().filter_map(|&(n, max)| db.iter().find(|f| f.name == n).map(|f| (f, max))).map(|(f, max)| {
        let r = foods::db_row(f);
        Cand { key: r.key, name: r.name, tag: r.tag, per: f.nutrients().scaled(0.01), max_g: Some(max), recipe: false }
    });
    let my = mine.iter().map(|f| Cand { key: format!("my:{}", f.id), name: f.name.clone(), tag: "My Food", per: f.per_serving, max_g: None, recipe: false });
    let rec = recipes.iter().filter_map(|r| Some(Cand { key: format!("recipe:{}", r.uid), name: r.name.clone(), tag: "Recipe", per: r.per_serving?, max_g: None, recipe: true }));
    let mut scored: Vec<(f32, bool, Fit)> = everyday.chain(my).chain(rec).filter_map(|c| fit_one(&c, left).map(|(s, f)| (s, c.recipe, f))).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.2.name.cmp(&b.2.name)));
    let pick = |recipe: bool, n: usize| scored.iter().filter(|s| s.1 == recipe).take(n).map(|s| s.2.clone()).collect::<Vec<_>>();
    let (foods, recipes) = (pick(false, MAX_FOODS), pick(true, MAX_RECIPES));
    let note = (foods.is_empty() && recipes.is_empty()).then_some(NOTHING_FITS);
    Fits { foods, recipes, note }
}

/// The match score and the suggestion, or None when it doesn't fit or adds next to nothing. The score is how closely the food's
/// split of calories between protein, net carbs and fat matches what is left (cosine, 0 to 1), so a lean protein wins when
/// protein is what's left and a dense food doesn't win just for filling more.
fn fit_one(c: &Cand, left: [f32; 3]) -> Option<(f32, Fit)> {
    let amt = [c.per.protein_g?, diary::net_carbs(&c.per)?.max(0.0), c.per.fat_g?];
    if amt.iter().all(|&a| a <= 0.0) {
        return None;
    }
    // the most of this food that stays within what is left (plus the slack) on every macro
    let most = (0..3).filter(|&i| amt[i] > 0.0).map(|i| (left[i] + SLACK_G) / amt[i]).fold(f32::INFINITY, f32::min);
    let (n, amount_text) = if let Some(max) = c.max_g {
        let g = ((most / 10.0).floor() * 10.0).min(max);
        if g < MIN_GRAMS {
            return None;
        }
        (g, format!("{} g", g as i32))
    } else {
        let s = ((most * 2.0).floor() / 2.0).min(MAX_SERVINGS);
        if s < 0.5 {
            return None;
        }
        (s, if s == 1.0 { "1 serving".into() } else { format!("{} servings", foods::num1(s)) })
    };
    let open: Vec<usize> = (0..3).filter(|&i| left[i] >= NEARLY_MET_G).collect();
    let fill = open.iter().map(|&i| (n * amt[i] / left[i]).min(1.0)).sum::<f32>() / open.len() as f32;
    if fill < MIN_FILL {
        return None;
    }
    const KCAL_PER_G: [f32; 3] = [4.0, 4.0, 9.0];
    let (a, l) = ((0..3).map(|i| amt[i] * KCAL_PER_G[i]), (0..3).map(|i| if left[i] >= NEARLY_MET_G { left[i] * KCAL_PER_G[i] } else { 0.0 }));
    let (a, l): (Vec<f32>, Vec<f32>) = (a.collect(), l.collect());
    let len = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let score = (0..3).map(|i| a[i] * l[i]).sum::<f32>() / (len(&a) * len(&l));
    let g = |x: f32| format!("{} g", foods::num1((x * 10.0).round() / 10.0));
    let kcal = c.per.calories.map_or("– kcal".to_string(), |k| format!("{} kcal", (k * n).round() as i32));
    let macros_text = format!("{} protein · {} net carbs · {} fat · {kcal}", g(n * amt[0]), g(n * amt[1]), g(n * amt[2]));
    Some((score, Fit { key: c.key.clone(), name: c.name.clone(), tag: c.tag, amount_text, macros_text }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::diary::MacroGoal;

    fn goal(goal: i32, eaten: Option<f32>) -> MacroGoal {
        MacroGoal { goal, eaten, unit_text: String::new(), over: false, said: String::new() }
    }

    fn budget(p: (i32, Option<f32>), c: (i32, Option<f32>), f: (i32, Option<f32>)) -> Budget {
        Budget {
            target: 2000,
            remaining: None,
            over: false,
            label: "Remaining",
            figure: None,
            figure_text: None,
            said: String::new(),
            formula: String::new(),
            raised_note: None,
            exercise_raw: None,
            exercise_counted: None,
            exercise_note: None,
            template: None,
            template_title: None,
            protein: goal(p.0, p.1),
            carbs: goal(c.0, c.1),
            fat: goal(f.0, f.1),
        }
    }

    fn n(kcal: f32, p: f32, c: f32, f: f32) -> Nutrients {
        Nutrients { calories: Some(kcal), protein_g: Some(p), carbs_g: Some(c), fat_g: Some(f), ..Default::default() }
    }

    fn parse_g(s: &str, what: &str) -> f32 {
        s.split(" · ").find(|p| p.ends_with(what)).unwrap().split(' ').next().unwrap().parse().unwrap()
    }

    #[test]
    fn every_everyday_food_is_in_the_database() {
        let db = nutrition::db_foods();
        for (name, _) in EVERYDAY {
            assert!(db.iter().any(|f| f.name == *name), "{name} is not in foods.tsv");
        }
    }

    #[test]
    fn no_suggestion_passes_what_is_left_by_more_than_the_slack() {
        let b = budget((150, Some(100.0)), (200, Some(170.0)), (70, Some(60.0)));
        let r = RecipeFood { uid: "u1".into(), name: "Chili".into(), per_serving: Some(n(400.0, 25.0, 30.0, 12.0)) };
        let v = fits(Some(&b), &[], &[r]);
        assert!(!v.foods.is_empty());
        for f in v.foods.iter().chain(&v.recipes) {
            assert!(parse_g(&f.macros_text, "protein") <= 50.0 + SLACK_G, "{f:?}");
            assert!(parse_g(&f.macros_text, "net carbs") <= 30.0 + SLACK_G, "{f:?}");
            assert!(parse_g(&f.macros_text, "fat") <= 10.0 + SLACK_G, "{f:?}");
        }
        // 12 g fat a serving against 10 left (+3 slack): one serving fits, not 1.5
        assert_eq!(v.recipes[0].amount_text, "1 serving");
    }

    #[test]
    fn protein_left_puts_a_lean_protein_food_first() {
        let b = budget((150, Some(60.0)), (200, Some(198.0)), (70, Some(68.0)));
        let v = fits(Some(&b), &[], &[]);
        let first = &v.foods[0];
        assert!(parse_g(&first.macros_text, "protein") >= 40.0, "{first:?}");
        assert!(v.foods.iter().all(|f| !f.name.starts_with("Nuts")), "{:?}", v.foods);
    }

    #[test]
    fn unknown_macros_are_skipped_never_counted_as_zero() {
        let b = budget((150, Some(100.0)), (200, Some(100.0)), (70, Some(30.0)));
        let blank = MyFood { id: 3, name: "Mystery bar".into(), per_serving: Nutrients { calories: Some(200.0), protein_g: Some(20.0), ..Default::default() }, ..Default::default() };
        let none = RecipeFood { uid: "u2".into(), name: "No estimate".into(), per_serving: None };
        let v = fits(Some(&b), &[blank], &[none]);
        assert!(v.foods.iter().all(|f| f.key != "my:3"));
        assert!(v.recipes.is_empty());
    }

    #[test]
    fn notes_for_unknown_left_met_and_no_target() {
        assert_eq!(fits(Some(&budget((150, None), (200, Some(0.0)), (70, Some(0.0)))), &[], &[]).note, Some(LEFT_UNKNOWN));
        let met = fits(Some(&budget((150, Some(149.0)), (200, Some(210.0)), (70, Some(68.0)))), &[], &[]);
        assert_eq!((met.note, met.foods.len()), (Some(NEARLY_MET), 0));
        assert_eq!(fits(None, &[], &[]), Fits::default());
    }

    #[test]
    fn lists_are_capped_and_servings_go_in_halves() {
        let b = budget((150, Some(0.0)), (200, Some(0.0)), (70, Some(0.0)));
        let recipes: Vec<RecipeFood> = (1..=5).map(|i| RecipeFood { uid: format!("r{i}"), name: format!("Recipe {i}"), per_serving: Some(n(300.0, 20.0, 30.0, 10.0)) }).collect();
        let v = fits(Some(&b), &[], &recipes);
        assert_eq!((v.foods.len(), v.recipes.len()), (MAX_FOODS, MAX_RECIPES));
        assert_eq!(v.recipes[0].amount_text, "1 serving");
        assert!(v.foods.iter().all(|f| f.key.starts_with("db:")), "{:?}", v.foods);
        let half = fits(Some(&budget((150, Some(140.0)), (200, Some(0.0)), (70, Some(0.0)))), &[], &[RecipeFood { per_serving: Some(n(300.0, 20.0, 30.0, 10.0)), ..recipes[0].clone() }]);
        assert_eq!(half.recipes[0].amount_text, "0.5 servings");
    }

    fn chicken() -> Cand {
        Cand { key: "db:x".into(), name: "Chicken".into(), tag: "USDA", per: n(1.2, 0.225, 0.0, 0.026), max_g: Some(200.0), recipe: false }
    }

    #[test]
    fn dense_foods_do_not_win_just_for_filling_more() {
        let b = budget((150, Some(100.0)), (200, Some(170.0)), (70, Some(60.0)));
        let v = fits(Some(&b), &[], &[]);
        assert!(v.foods.iter().all(|f| !f.name.starts_with("Nuts") && !f.name.starts_with("Peanut")), "{:?}", v.foods);
    }

    #[test]
    fn grams_go_down_to_10_g_steps_and_never_below_30() {
        // protein 20 left (+3): 23 / 0.225 = 102 g, so 100 g
        let (_, f) = fit_one(&chicken(), [20.0, 0.0, 0.0]).unwrap();
        assert_eq!(f.amount_text, "100 g");
        assert!(fit_one(&chicken(), [3.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn a_food_that_adds_next_to_nothing_is_left_out() {
        // only fat is open and chicken covers well under a tenth of it
        assert!(fit_one(&chicken(), [0.0, 0.0, 60.0]).is_none());
    }

    #[test]
    fn my_foods_compete_with_bundled_foods_and_unknown_fiber_counts_as_0() {
        let b = budget((150, Some(100.0)), (200, Some(180.0)), (70, Some(68.0)));
        let shake = MyFood { id: 7, name: "Protein shake".into(), per_serving: Nutrients { calories: Some(160.0), protein_g: Some(40.0), carbs_g: Some(4.0), fat_g: Some(1.0), ..Default::default() }, ..Default::default() };
        let v = fits(Some(&b), &[shake], &[]);
        let f = v.foods.iter().find(|f| f.key == "my:7").expect("the shake fits");
        assert_eq!((f.tag, f.amount_text.as_str()), ("My Food", "1 serving"));
        assert!(f.macros_text.contains("4 g net carbs"), "{f:?}");
    }
}
