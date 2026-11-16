//! Automatic nutrition from the ingredient list.
//!
//! Food values come from the official food-composition databases, bundled so it works offline
//! (data/foods.tsv, built by tools/build_foods.py):
//!   * USDA FoodData Central — Foundation Foods, SR Legacy and FNDDS
//!   * Health Canada — Canadian Nutrient File 2026
//! FNDDS rows (mixed dishes) and foods that don't report protein, fat and carbs are used by food
//! search only; recipe lines are matched against the other foods (see `in_estimator`).
//! Each ingredient line is matched to the closest food (your own "My Foods" from package labels
//! first, then a short list of hand-checked matches for common recipe wording, then a scored
//! search of the whole database), weighed in grams using the database's own portion weights
//! (cup, tbsp, medium, slice…), and added up.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::ingredients::{self, parse_ingredient, ParsedIngredient};
use crate::model::{now_secs, Library, MyFood, Nutrients, Recipe};

/// Bump when the database or matching changes enough that saved estimates should be redone.
/// 3: food table rebuilt from Canadian Nutrient File 2026 and Foundation Foods 2026-04.
/// 4: recipe totals keep a value an ingredient does not report unknown (02-08).
pub const ESTIMATOR_VERSION: u32 = 4;

#[allow(dead_code)]
pub struct DbFood {
    pub id: &'static str,
    pub source: char,
    pub name: &'static str,
    /// kcal, protein, fat, carbs, fiber, sugar, sodium per 100 g; a value the source does not report is 0.0 here
    /// and its bit is clear in `known`. Food search reads values through [`DbFood::nutrients`].
    pub per100: [f32; 7],
    /// bit i is set when column i of `per100` was a number in the table (blank means unknown)
    pub known: u8,
    /// grams per millilitre, from the database's cup/tbsp/ml portions
    pub density: Option<f32>,
    /// grams for one "medium", "large", "slice", "clove", "scoop"…
    pub counts: Vec<(&'static str, f32)>,
    pub(crate) tokens: Vec<String>,
    pub(crate) head: Vec<String>,
}

/// Grams for one `key` ("steak", "drumstick", "each"), from this food or, when its own entry
/// lists none (Foundation Foods rarely do), from the closest food with the same heading.
fn count_like(f: &DbFood, key: &str) -> Option<f32> {
    if let Some(g) = f.count(key) {
        return Some(g);
    }
    let d = db();
    let mut best: Option<(usize, f32)> = None;
    for o in d.by_token.get(f.head.first()?)?.iter().map(|&i| &d.foods[i]) {
        if o.head != f.head {
            continue;
        }
        let Some(g) = o.count(key) else { continue };
        let overlap = o.tokens.iter().filter(|t| f.tokens.contains(t)).count();
        if best.map(|b| overlap > b.0).unwrap_or(true) {
            best = Some((overlap, g));
        }
    }
    best.map(|b| b.1)
}

impl DbFood {
    pub fn source_name(&self) -> &'static str {
        match self.source {
            'F' => "USDA Foundation Foods",
            'S' => "USDA SR Legacy",
            'N' => "USDA FNDDS",
            _ => "Canadian Nutrient File",
        }
    }
    /// The seven values, `None` where the source does not report one (never 0 for unknown).
    pub fn nutrients(&self) -> Nutrients {
        let v = |i: usize| (self.known >> i & 1 == 1).then_some(self.per100[i]);
        Nutrients { calories: v(0), protein_g: v(1), fat_g: v(2), carbs_g: v(3), fiber_g: v(4), sugar_g: v(5), sodium_mg: v(6) }
    }
    fn count(&self, k: &str) -> Option<f32> {
        self.counts.iter().find(|(n, _)| *n == k).map(|(_, g)| *g)
    }
}

struct Db {
    foods: Vec<DbFood>,
    by_token: HashMap<String, Vec<usize>>,
}

static DB: OnceLock<Db> = OnceLock::new();
static MY_FOODS: RwLock<Vec<MyFood>> = RwLock::new(Vec::new());

fn keyw(s: &str) -> Vec<String> {
    ingredients::match_key(s).split(' ').filter(|w| !w.is_empty()).map(|w| w.to_string()).collect()
}

/// One table row: `None` for a comment, a blank line or a short row. A blank or non-numeric value
/// cell is unknown (its bit in `known` stays clear); a 0 is a real zero.
fn parse_row(line: &'static str) -> Option<DbFood> {
    if line.starts_with('#') || line.trim().is_empty() {
        return None;
    }
    let c: Vec<&'static str> = line.split('\t').collect();
    if c.len() < 11 {
        return None;
    }
    let mut per100 = [0f32; 7];
    let mut known = 0u8;
    for (i, v) in c[4..11].iter().enumerate() {
        if let Ok(x) = v.parse::<f32>() {
            per100[i] = x;
            known |= 1 << i;
        }
    }
    let mut density = None;
    let mut counts = vec![];
    if let Some(p) = c.get(11) {
        for part in p.split(';').filter(|s| !s.is_empty()) {
            if let Some((k, v)) = part.split_once('=') {
                let g: f32 = v.parse().unwrap_or(0.0);
                if g <= 0.0 {
                    continue;
                }
                if k == "ml" {
                    density = Some(g);
                } else {
                    counts.push((k, g));
                }
            }
        }
    }
    let name = c[3];
    Some(DbFood {
        id: c[0],
        source: c[1].chars().next().unwrap_or('S'),
        name,
        per100,
        known,
        density,
        counts,
        tokens: keyw(name),
        head: keyw(name.split(',').next().unwrap_or(name)),
    })
}

/// Whether recipe lines may be matched to this food. Not FNDDS dishes, and not a food missing protein,
/// fat or carbs: such a food would make every recipe using it show unknown macros, so recipe lines match
/// complete foods. Food search sees every food.
fn in_estimator(f: &DbFood) -> bool {
    f.source != 'N' && f.known & 0b1110 == 0b1110
}

fn db() -> &'static Db {
    DB.get_or_init(|| {
        let foods: Vec<DbFood> = include_str!("../data/foods.tsv").lines().filter_map(parse_row).collect();
        let mut by_token: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, f) in foods.iter().enumerate().filter(|(_, f)| in_estimator(f)) {
            let mut seen = vec![];
            for t in &f.tokens {
                if !seen.contains(t) {
                    by_token.entry(t.clone()).or_default().push(i);
                    seen.push(t.clone());
                }
            }
        }
        Db { foods, by_token }
    })
}

/// Every bundled food, including those kept out of the recipe estimator (food search shows them all).
pub(crate) fn db_foods() -> &'static [DbFood] {
    &db().foods
}

/// Number of foods in the bundled database.
pub fn food_count() -> usize {
    db().foods.len()
}

/// Replace the foods entered from package labels (call after loading or editing the library).
pub fn set_my_foods(foods: &[MyFood]) {
    if let Ok(mut m) = MY_FOODS.write() {
        *m = foods.to_vec();
    }
}

// ---------------------------------------------------------------------------------------------
// Matching

/// Hand-checked matches for common recipe wording that a word search gets wrong
/// ("cauliflower rice" is cauliflower, "almond milk" is not almonds). `state` picks the cooked or
/// dry version when the line says so. The food name must exist in data/foods.tsv (a test checks).
const ALIASES: &[(&str, &str, &str)] = &[
    // (ingredient words, state "", "cooked", "dry" or "canned", exact database name)
    ("cauliflower rice", "", "Cauliflower, raw"),
    ("riced cauliflower", "", "Cauliflower, raw"),
    ("almond milk", "", "Beverages, almond milk, unsweetened, shelf stable"),
    ("sweetened almond milk", "", "Beverages, almond milk, sweetened, vanilla flavor, ready-to-drink"),
    ("soy milk", "", "Soymilk, original and vanilla, unfortified"),
    ("oat milk", "", "Oat milk, unsweetened, plain, refrigerated"),
    ("coconut milk", "", "Nuts, coconut milk, canned (liquid expressed from grated meat and water)"),
    ("milk", "", "Milk, whole, 3.25% milkfat, with added vitamin D"),
    ("whole milk", "", "Milk, whole, 3.25% milkfat, with added vitamin D"),
    ("skim milk", "", "Milk, nonfat, fluid, with added vitamin A and vitamin D (fat free or skim)"),
    ("nonfat milk", "", "Milk, nonfat, fluid, with added vitamin A and vitamin D (fat free or skim)"),
    ("lactose free milk", "", "Milk, lowfat, fluid, 1% milkfat, with added vitamin A and vitamin D"),
    ("2% milk", "", "Milk, reduced fat, fluid, 2% milkfat, with added vitamin A and vitamin D"),
    ("1% milk", "", "Milk, lowfat, fluid, 1% milkfat, with added vitamin A and vitamin D"),
    ("greek yogurt", "", "Yogurt, Greek, plain, whole milk"),
    ("nonfat greek yogurt", "", "Yogurt, Greek, plain, nonfat"),
    ("non fat greek yogurt", "", "Yogurt, Greek, plain, nonfat"),
    ("plain nonfat greek yogurt", "", "Yogurt, Greek, plain, nonfat"),
    ("yogurt", "", "Yogurt, plain, whole milk"),
    ("heavy cream", "", "Cream, fluid, heavy whipping"),
    ("whipping cream", "", "Cream, fluid, heavy whipping"),
    ("sour cream", "", "Cream, sour, cultured"),
    ("cream cheese", "", "Cheese, cream"),
    ("butter", "", "Butter, salted"),
    ("unsalted butter", "", "Butter, without salt"),
    ("egg", "", "Egg, whole, raw, fresh"),
    ("large egg", "", "Egg, whole, raw, fresh"),
    ("egg yolk", "", "Egg, yolk, raw, fresh"),
    ("egg white", "", "Egg, white, raw, fresh"),
    ("liquid egg white", "", "Egg, white, raw, fresh"),
    ("parmesan", "", "Cheese, parmesan, grated"),
    ("cheddar", "", "Cheese, cheddar"),
    ("mozzarella", "", "Cheese, mozzarella, whole milk"),
    ("feta", "", "Cheese, feta"),
    ("cottage cheese", "", "Cheese, cottage, creamed, large or small curd"),
    // grains and pasta: dry unless the line says cooked
    ("rice", "", "Rice, white, long-grain, regular, raw, enriched"),
    ("rice", "cooked", "Rice, white, long-grain, regular, enriched, cooked"),
    ("white rice", "", "Rice, white, long-grain, regular, raw, enriched"),
    ("white rice", "cooked", "Rice, white, long-grain, regular, enriched, cooked"),
    ("jasmine rice", "", "Rice, white, long-grain, regular, raw, enriched"),
    ("jasmine rice", "cooked", "Rice, white, long-grain, regular, enriched, cooked"),
    ("basmati rice", "", "Rice, white, long-grain, regular, raw, enriched"),
    ("basmati rice", "cooked", "Rice, white, long-grain, regular, enriched, cooked"),
    ("brown rice", "", "Rice, brown, long-grain, raw (Includes foods for USDA's Food Distribution Program)"),
    ("brown rice", "cooked", "Rice, brown, long-grain, cooked (Includes foods for USDA's Food Distribution Program)"),
    ("black rice", "", "Rice, brown, long-grain, raw (Includes foods for USDA's Food Distribution Program)"),
    ("black rice", "cooked", "Rice, brown, long-grain, cooked (Includes foods for USDA's Food Distribution Program)"),
    ("wild rice", "", "Wild rice, raw"),
    ("wild rice", "cooked", "Wild rice, cooked"),
    ("quinoa", "", "Quinoa, uncooked"),
    ("quinoa", "cooked", "Quinoa, cooked"),
    ("bulgur", "", "Bulgur, dry"),
    ("bulgur wheat", "", "Bulgur, dry"),
    ("bulgur", "cooked", "Bulgur, cooked"),
    ("couscous", "", "Couscous, dry"),
    ("couscous", "cooked", "Couscous, cooked"),
    ("oat", "", "Cereals, oats, regular and quick, not fortified, dry"),
    ("rolled oat", "", "Cereals, oats, regular and quick, not fortified, dry"),
    ("oatmeal", "", "Cereals, oats, regular and quick, not fortified, dry"),
    ("pasta", "", "Pasta, dry, enriched"),
    ("pasta", "cooked", "Pasta, cooked, enriched, without added salt"),
    ("spaghetti", "", "Pasta, dry, enriched"),
    ("spaghetti", "cooked", "Pasta, cooked, enriched, without added salt"),
    ("penne", "", "Pasta, dry, enriched"),
    ("macaroni", "", "Pasta, dry, enriched"),
    ("rice noodle", "", "Rice noodles, dry"),
    ("rice noodle", "cooked", "Rice noodles, cooked"),
    ("flour", "", "Wheat flour, white, all-purpose, enriched, bleached"),
    ("all purpose flour", "", "Wheat flour, white, all-purpose, enriched, bleached"),
    ("whole wheat flour", "", "Flour, whole wheat, unenriched"),
    ("vital wheat gluten", "", "Vital wheat gluten"),
    ("seitan", "", "Vital wheat gluten"),
    ("bread", "", "Bread, whole-wheat, commercially prepared"),
    ("toast", "", "Bread, whole-wheat, commercially prepared"),
    ("sprouted whole grain toast", "", "Bread, whole-wheat, commercially prepared"),
    ("sprouted wheat bread", "", "Bread, whole-wheat, commercially prepared"),
    ("corn tortilla", "", "Tortillas, ready-to-bake or -fry, corn"),
    ("flour tortilla", "", "Tortillas, ready-to-bake or -fry, flour, refrigerated"),
    // legumes: dry unless cooked/canned
    ("lentil", "", "Lentils, raw"),
    ("lentil", "cooked", "Lentils, mature seeds, cooked, boiled, without salt"),
    ("red lentil", "", "Lentils, pink or red, raw"),
    ("red split lentil", "", "Lentils, pink or red, raw"),
    ("green lentil", "", "Lentils, raw"),
    ("brown lentil", "", "Lentils, raw"),
    ("chickpea", "", "Chickpeas (garbanzo beans, bengal gram), mature seeds, raw"),
    ("chickpea", "cooked", "Chickpeas (garbanzo beans, bengal gram), mature seeds, cooked, boiled, without salt"),
    ("chickpea", "canned", "Chickpeas (garbanzo beans, bengal gram), mature seeds, canned, drained, rinsed in tap water"),
    ("black bean", "", "Beans, black, mature seeds, raw"),
    ("black bean", "cooked", "Beans, black, mature seeds, cooked, boiled, without salt"),
    ("black bean", "canned", "Beans, black, mature seeds, canned, low sodium"),
    ("kidney bean", "", "Beans, kidney, all types, mature seeds, raw"),
    ("kidney bean", "cooked", "Beans, kidney, all types, mature seeds, cooked, boiled, without salt"),
    ("kidney bean", "canned", "Beans, kidney, red, mature seeds, canned, drained solids"),
    ("edamame", "", "Edamame, frozen, prepared"),
    ("shelled edamame", "", "Edamame, frozen, prepared"),
    ("tofu", "", "Tofu, raw, firm, prepared with calcium sulfate"),
    ("firm tofu", "", "Tofu, raw, firm, prepared with calcium sulfate"),
    ("extra firm tofu", "", "Tofu, raw, firm, prepared with calcium sulfate"),
    ("silken tofu", "", "Tofu, soft, prepared with calcium sulfate and magnesium chloride (nigari)"),
    ("tempeh", "", "Tempeh"),
    ("hummus", "", "Hummus, commercial"),
    ("peanut butter", "", "Peanut butter, smooth style, without salt"),
    ("powdered peanut butter", "", "Peanut flour, defatted"),
    ("peanut butter powder", "", "Peanut flour, defatted"),
    // meat and fish (raw weight)
    ("chicken breast", "", "Chicken, breast, boneless, skinless, raw"),
    ("boneless skinless chicken breast", "", "Chicken, breast, boneless, skinless, raw"),
    ("lean chicken breast", "", "Chicken, breast, boneless, skinless, raw"),
    ("chicken thigh", "", "Chicken, broilers or fryers, dark meat, thigh, meat only, raw"),
    ("whole chicken", "", "Chicken, broilers or fryers, meat and skin, raw"),
    ("chicken", "", "Chicken, broilers or fryers, meat and skin, raw"),
    ("ground turkey", "", "Turkey, ground, raw"),
    ("lean ground turkey", "", "Turkey, ground, 93% lean, 7% fat, raw"),
    ("extra lean ground turkey", "", "Turkey, ground, fat free, raw"),
    ("99% lean ground turkey", "", "Turkey, ground, fat free, raw"),
    ("fat free ground turkey", "", "Turkey, ground, fat free, raw"),
    ("ground beef", "", "Beef, ground, 80% lean meat / 20% fat, raw"),
    ("lean ground beef", "", "Beef, ground, 93% lean meat / 7% fat, raw"),
    ("ground pork", "", "Pork, fresh, ground, raw"),
    ("bacon", "", "Pork, cured, bacon, unprepared"),
    ("salmon", "", "Fish, salmon, Atlantic, farmed, raw"),
    ("shrimp", "", "Crustaceans, shrimp, raw"),
    ("prawn", "", "Crustaceans, shrimp, raw"),
    ("tuna", "canned", "Fish, tuna, light, canned in water, drained solids"),
    // vegetables and fruit
    ("broccoli", "", "Broccoli, raw"),
    ("broccoli floret", "", "Broccoli, raw"),
    ("snap pea", "", "Peas, edible-podded, raw"),
    ("sugar snap pea", "", "Peas, edible-podded, raw"),
    ("snow pea", "", "Peas, edible-podded, raw"),
    ("pea", "", "Peas, green, frozen, unprepared"),
    ("sweet potato", "", "Sweet potato, raw, unprepared (Includes foods for USDA's Food Distribution Program)"),
    ("potato", "", "Potatoes, flesh and skin, raw"),
    ("spinach", "", "Spinach, raw"),
    ("baby spinach", "", "Spinach, raw"),
    ("kale", "", "Kale, raw"),
    ("lettuce", "", "Lettuce, cos or romaine, raw"),
    ("romaine", "", "Lettuce, cos or romaine, raw"),
    ("butter lettuce", "", "Lettuce, butterhead (includes boston and bibb types), raw"),
    ("cauliflower", "", "Cauliflower, raw"),
    ("zucchini", "", "Squash, summer, zucchini, includes skin, raw"),
    ("bell pepper", "", "Peppers, sweet, red, raw"),
    ("red bell pepper", "", "Peppers, sweet, red, raw"),
    ("green bell pepper", "", "Peppers, sweet, green, raw"),
    ("roasted red pepper", "", "Peppers, sweet, red, canned, solids and liquids"),
    ("onion", "", "Onions, raw"),
    ("red onion", "", "Onions, raw"),
    ("green onion", "", "Onions, spring or scallions (includes tops and bulb), raw"),
    ("scallion", "", "Onions, spring or scallions (includes tops and bulb), raw"),
    ("garlic", "", "Garlic, raw"),
    ("garlic clove", "", "Garlic, raw"),
    ("ginger", "", "Ginger root, raw"),
    ("fresh ginger", "", "Ginger root, raw"),
    ("ginger garlic paste", "", "Garlic, raw"),
    ("carrot", "", "Carrots, raw"),
    ("tomato", "", "Tomatoes, red, ripe, raw, year round average"),
    ("cherry tomato", "", "Tomatoes, red, ripe, raw, year round average"),
    ("crushed tomato", "", "Tomatoes, crushed, canned"),
    ("diced tomato", "", "Tomatoes, red, ripe, canned, packed in tomato juice"),
    ("canned diced tomato", "", "Tomatoes, red, ripe, canned, packed in tomato juice"),
    ("tomato paste", "", "Tomato products, canned, paste, without salt added (Includes foods for USDA's Food Distribution Program)"),
    ("tomato sauce", "", "Tomato products, canned, sauce"),
    ("mushroom", "", "Mushrooms, white, raw"),
    ("cucumber", "", "Cucumber, with peel, raw"),
    ("eggplant", "", "Eggplant, raw"),
    ("avocado", "", "Avocados, raw, all commercial varieties"),
    ("cabbage", "", "Cabbage, raw"),
    ("celery", "", "Celery, raw"),
    ("corn", "", "Corn, sweet, yellow, raw"),
    ("banana", "", "Bananas, raw"),
    ("apple", "", "Apples, raw, with skin (Includes foods for USDA's Food Distribution Program)"),
    ("blueberry", "", "Blueberries, raw"),
    ("raspberry", "", "Raspberries, raw"),
    ("strawberry", "", "Strawberries, raw"),
    ("mixed berry", "", "Blueberries, raw"),
    ("berry", "", "Blueberries, raw"),
    ("lemon", "", "Lemons, raw, without peel"),
    ("lime", "", "Limes, raw"),
    ("lemon juice", "", "Lemon juice, raw"),
    ("lime juice", "", "Lime juice, raw"),
    ("lemon zest", "", "Lemon peel, raw"),
    ("orange juice", "", "Orange juice, raw"),
    // fats, nuts, seeds
    ("olive oil", "", "Oil, olive, salad or cooking"),
    ("extra virgin olive oil", "", "Oil, olive, salad or cooking"),
    ("olive oil cooking spray", "", "Oil, olive, salad or cooking"),
    ("cooking spray", "", "Oil, olive, salad or cooking"),
    ("oil", "", "Oil, canola"),
    ("vegetable oil", "", "Oil, canola"),
    ("canola oil", "", "Oil, canola"),
    ("sesame oil", "", "Oil, sesame, salad or cooking"),
    ("coconut oil", "", "Oil, coconut"),
    ("chia seed", "", "Seeds, chia seeds, dried"),
    ("hemp heart", "", "Seeds, hemp seed, hulled"),
    ("hemp seed", "", "Seeds, hemp seed, hulled"),
    ("flaxseed", "", "Seeds, flaxseed"),
    ("ground flaxseed", "", "Seeds, flaxseed"),
    ("tahini", "", "Seeds, sesame butter, tahini, from roasted and toasted kernels (most common type)"),
    ("sesame seed", "", "Seeds, sesame seeds, whole, dried"),
    ("almond", "", "Nuts, almonds"),
    ("walnut", "", "Nuts, walnuts, english"),
    ("peanut", "", "Peanuts, all types, raw"),
    // sweeteners, baking, pantry
    ("sugar", "", "Sugars, granulated"),
    ("brown sugar", "", "Sugars, brown"),
    ("honey", "", "Honey"),
    ("maple syrup", "", "Syrups, maple"),
    ("vanilla", "", "Vanilla extract"),
    ("vanilla extract", "", "Vanilla extract"),
    ("cocoa powder", "", "Cocoa, dry powder, unsweetened"),
    ("baking powder", "", "Leavening agents, baking powder, double-acting, sodium aluminum sulfate"),
    ("baking soda", "", "Leavening agents, baking soda"),
    ("cornstarch", "", "Cornstarch"),
    ("salt", "", "Salt, table"),
    ("sea salt", "", "Salt, table"),
    ("kosher salt", "", "Salt, table"),
    ("pepper", "", "Spices, pepper, black"),
    ("black pepper", "", "Spices, pepper, black"),
    ("nutritional yeast", "", "Leavening agents, yeast, baker's, active dry"),
    ("soy sauce", "", "Soy sauce made from soy and wheat (shoyu)"),
    ("low sodium soy sauce", "", "Soy sauce made from soy and wheat (shoyu), low sodium"),
    ("tamari", "", "Soy sauce made from soy (tamari)"),
    ("rice vinegar", "", "Vinegar, distilled"),
    ("vinegar", "", "Vinegar, distilled"),
    ("balsamic vinegar", "", "Vinegar, balsamic"),
    ("apple cider vinegar", "", "Vinegar, cider"),
    ("fish sauce", "", "Sauce, fish, ready-to-serve"),
    ("broth", "", "Soup, chicken broth, ready-to-serve"),
    ("chicken broth", "", "Soup, chicken broth, ready-to-serve"),
    ("vegetable broth", "", "Soup, vegetable broth, ready to serve"),
    ("water", "", "Beverages, water, tap, drinking"),
    ("protein powder", "", "Beverages, Protein powder whey based"),
    ("whey protein", "", "Beverages, Protein powder whey based"),
    ("vanilla protein powder", "", "Beverages, Protein powder whey based"),
    ("dijon mustard", "", "Mustard, prepared, yellow"),
    ("mustard", "", "Mustard, prepared, yellow"),
    ("mayonnaise", "", "Salad dressing, mayonnaise, regular"),
    ("salsa", "", "Sauce, salsa, ready-to-serve"),
    ("sriracha", "", "Sauce, hot chile, sriracha"),
    ("hot sauce", "", "Sauce, ready-to-serve, pepper or hot"),
    ("gochujang", "", "Sauce, hoisin, ready-to-serve"),
    ("harissa", "", "Sauce, hot chile, sriracha"),
    ("harissa paste", "", "Sauce, hot chile, sriracha"),
    ("curry paste", "", "Sauce, hot chile, sriracha"),
    ("miso", "", "Miso"),
    // spices
    ("cinnamon", "", "Spices, cinnamon, ground"),
    ("cardamom", "", "Spices, cardamom"),
    ("clove", "", "Spices, cloves, ground"),
    ("ground clove", "", "Spices, cloves, ground"),
    ("nutmeg", "", "Spices, nutmeg, ground"),
    ("cumin", "", "Spices, cumin seed"),
    ("turmeric", "", "Spices, turmeric, ground"),
    ("paprika", "", "Spices, paprika"),
    ("smoked paprika", "", "Spices, paprika"),
    ("chili powder", "", "Spices, chili powder"),
    ("chili flake", "", "Spices, pepper, red or cayenne"),
    ("red pepper flake", "", "Spices, pepper, red or cayenne"),
    ("cayenne", "", "Spices, pepper, red or cayenne"),
    ("curry powder", "", "Spices, curry powder"),
    ("garam masala", "", "Spices, curry powder"),
    ("coriander", "", "Spices, coriander seed"),
    ("oregano", "", "Spices, oregano, dried"),
    ("dried oregano", "", "Spices, oregano, dried"),
    ("thyme", "", "Thyme, fresh"),
    ("dried thyme", "", "Spices, thyme, dried"),
    ("basil", "", "Basil, fresh"),
    ("parsley", "", "Parsley, fresh"),
    ("cilantro", "", "Coriander (cilantro) leaves, raw"),
    ("mint", "", "Spearmint, fresh"),
    ("rosemary", "", "Rosemary, fresh"),
    ("garlic powder", "", "Spices, garlic powder"),
    ("onion powder", "", "Spices, onion powder"),
    ("ground ginger", "", "Spices, ginger, ground"),
    ("bay leaf", "", "Spices, bay leaf"),
    ("star anise", "", "Spices, anise seed"),
    ("cinnamon stick", "", "Spices, cinnamon, ground"),
    ("baguette", "", "Bread, french or vienna (includes sourdough)"),
    ("french bread", "", "Bread, french or vienna (includes sourdough)"),
    ("sourdough", "", "Bread, french or vienna (includes sourdough)"),
    ("pie crust", "", "Pie crust, refrigerated, regular, unbaked"),
    ("pizza dough", "", "Bread, white, commercially prepared (includes soft bread crumbs)"),
    ("vinaigrette", "", "Salad dressing, italian dressing, commercial, regular"),
    ("italian dressing", "", "Salad dressing, italian dressing, commercial, regular"),
    ("mirin", "", "Alcoholic beverage, rice (sake)"),
    ("sake", "", "Alcoholic beverage, rice (sake)"),
    ("dashi", "", "Soup, stock, fish, home-prepared"),
    ("chili bean paste", "", "Miso"),
    ("doubanjiang", "", "Miso"),
];

/// Weight of one whole item when the databases list none (g).
const WHOLE_ITEM: &[(&str, f32)] = &[
    ("baguette", 250.0),
    ("pizza dough", 450.0),
    ("loaf", 450.0),
];

/// Hand-checked matches that are only the closest thing the databases have; the recipe page
/// names them so the real product can be added to My Foods.
const STAND_INS: &[&str] = &[
    "black rice", "gochujang", "harissa", "harissa paste", "curry paste", "mirin", "chili bean paste",
    "doubanjiang", "protein powder", "whey protein", "vanilla protein powder", "seitan", "nutritional yeast",
    "pizza dough", "ginger garlic paste", "garam masala", "sprouted whole grain toast", "sprouted wheat bread",
    "mixed berry", "rice vinegar", "star anise", "lactose free milk", "powdered peanut butter",
    "peanut butter powder", "cooking spray", "olive oil cooking spray",
];

/// If this line was matched through a stand-in, the ingredient's name ("gochujang").
fn stand_in_for(line: &str, food_name: &str) -> Option<&'static str> {
    let lk = keyw(line);
    STAND_INS
        .iter()
        .filter(|s| ingredients::match_key(s).split(' ').all(|w| lk.iter().any(|x| x == w)))
        .filter(|s| ALIASES.iter().any(|(a, _, n)| a == *s && *n == food_name))
        .max_by_key(|s| s.len())
        .copied()
}

/// Ingredient words that describe preparation, not the food.
const DESCRIPTIVE: &[&str] = &[
    "fresh", "chopped", "diced", "minced", "sliced", "grated", "shredded", "cubed", "halved", "peeled",
    "large", "small", "medium", "organic", "finely", "roughly", "thinly", "crushed", "pressed",
    "softened", "melted", "packed", "heaping", "level", "trimmed", "rinsed", "drained", "divided",
    "about", "approx", "approximately", "optional", "plus", "more", "for", "serving", "garnish",
    "to", "taste", "and", "or", "of", "the", "a", "an", "into", "piece", "pieces", "cut", "inch",
    "room", "temperature", "cold", "warm", "hot", "boneless", "skinless", "raw", "dry", "dried",
    "uncooked", "cooked", "canned", "frozen", "thawed", "ripe", "whole", "plain", "unsweetened",
    "g", "gram", "ml", "oz", "cup", "tbsp", "tsp", "scoop", "before", "hydrating",
];
const NEUTRAL_FOOD: &[&str] = &[
    "raw", "fresh", "n", "s", "as", "and", "or", "with", "without", "of", "all", "type", "unspecified",
    "regular", "plain", "commercial", "commercially", "includes", "include", "food", "distribution",
    "program", "usda", "s", "for", "variety", "varieties", "mature", "seed", "kernel", "kernels",
    "unprepared", "salt", "added", "enriched", "year", "round", "average", "red", "ripe", "edible",
    "flesh", "skin", "fluid", "whole", "ready", "to", "serve", "prepared",
];
const COOKED: &[&str] = &[
    "cooked", "boiled", "roasted", "baked", "steamed", "braised", "grilled", "broiled", "fried",
    "stewed", "microwaved", "simmered", "sauteed", "toasted",
];
const RAW: &[&str] = &["raw", "dry", "uncooked", "unprepared", "unheated"];
const ODD: &[&str] = &[
    "babyfood", "infant", "imitation", "dehydrated", "flavored", "fortified", "candied", "powder",
    "frozen", "breaded", "batter", "meal", "school", "lunch", "industrial", "glaze", "concentrate",
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum State {
    Any,
    Cooked,
    Dry,
    Canned,
}

fn line_state(line: &str) -> State {
    let w = keyw(line);
    let has = |s: &str| w.iter().any(|x| x == s);
    if has("canned") || has("can") || has("tinned") {
        return State::Canned;
    }
    if COOKED.iter().any(|c| has(c)) {
        // "cooked seitan", "100 g cooked brown rice", but not "for cooking"
        return State::Cooked;
    }
    if has("dry") || has("uncooked") || has("raw") || has("dried") {
        return State::Dry;
    }
    State::Any
}

/// What a line matched.
#[derive(Clone, Debug)]
pub enum Matched {
    Mine(usize),
    Db(usize),
}

fn find_db_by_name(name: &str) -> Option<usize> {
    let d = db();
    let mut best: Option<usize> = None;
    for (i, f) in d.foods.iter().enumerate() {
        if in_estimator(f) && f.name.eq_ignore_ascii_case(name) {
            let rank = |c: char| match c {
                'F' => 0,
                'S' => 1,
                _ => 2,
            };
            if best.map(|b| rank(f.source) < rank(d.foods[b].source)).unwrap_or(true) {
                best = Some(i);
            }
        }
    }
    best
}

/// Words naming a part of a food ("lettuce leaves", "broccoli florets"): not the food itself.
const PARTS: &[&str] = &[
    "leaf", "leave", "floret", "fillet", "piece", "slice", "cube", "strip", "spear", "sprig", "stalk",
    "wedge", "chunk", "half", "clove",
];

/// The main word of an ingredient name ("broccoli" in "broccoli florets").
fn head_word(name_key: &str) -> Option<String> {
    let w: Vec<&str> = name_key.split(' ').filter(|w| !w.is_empty() && !DESCRIPTIVE.contains(w)).collect();
    let food: Vec<&str> = w.iter().copied().filter(|x| !PARTS.contains(x)).collect();
    food.last().or(w.last()).map(|s| s.to_string())
}

/// Hand-checked match: the alias with the most words that all appear in the line and that
/// includes the ingredient's main word ("non-fat plain Greek yogurt" → "non fat greek yogurt";
/// "milk chocolate chips" does not match "milk").
fn alias_match(name_key: &str, scope: &[String], state: State) -> Option<usize> {
    let head = head_word(name_key)?;
    let in_scope = |w: &str| scope.iter().any(|s| s == w) || name_key.split(' ').any(|n| n == w);
    let mut best: Option<(usize, bool, &str, &str)> = None; // (words, state-specific, alias, state)
    for (a, st, _) in ALIASES {
        let ak = ingredients::match_key(a);
        let aw: Vec<&str> = ak.split(' ').collect();
        if !aw.contains(&head.as_str()) || !aw.iter().all(|w| in_scope(w)) {
            continue;
        }
        let st_ok = match (*st, state) {
            ("", _) => true,
            ("cooked", State::Cooked) => true,
            ("canned", State::Canned) => true,
            ("dry", State::Dry) => true,
            _ => false,
        };
        if !st_ok {
            continue;
        }
        let key = (aw.len(), !st.is_empty());
        if best.map(|b| key > (b.0, b.1)).unwrap_or(true) {
            best = Some((aw.len(), !st.is_empty(), a, st));
        }
    }
    let (_, _, a, st) = best?;
    // canned beans without their own canned entry: the cooked one is closest
    if state == State::Canned && st.is_empty() {
        if let Some(c) = ALIASES.iter().find(|(x, s, _)| *x == a && *s == "cooked") {
            return find_db_by_name(c.2);
        }
    }
    let entry = ALIASES.iter().find(|(x, s, _)| *x == a && *s == st)?;
    find_db_by_name(entry.2)
}

fn search(name_key: &str, line_key: &[String], state: State) -> Option<usize> {
    let d = db();
    let q: Vec<String> = name_key
        .split(' ')
        .filter(|w| !w.is_empty() && !DESCRIPTIVE.contains(w) && !w.chars().all(|c| c.is_ascii_digit()))
        .map(|w| w.to_string())
        .collect();
    let headword = q.last()?.clone();
    let cands = d.by_token.get(&headword)?;
    let has_line = |s: &str| line_key.iter().any(|x| x == s);
    let mut best: Option<(f32, usize)> = None;
    for &i in cands {
        let f = &d.foods[i];
        let mut s = 0.0f32;
        for t in &q {
            if f.tokens.contains(t) {
                s += if f.head.contains(t) { 2.0 } else { 1.2 };
            } else {
                s -= 1.5;
            }
        }
        if f.head.iter().all(|h| q.contains(h)) {
            s += 1.5;
        }
        let extras = f.tokens.iter().filter(|t| !q.contains(t) && !NEUTRAL_FOOD.contains(&t.as_str()) && !has_line(t)).count();
        s -= 0.3 * extras.min(8) as f32;
        let f_cooked = f.tokens.iter().any(|t| COOKED.contains(&t.as_str()));
        let f_raw = f.tokens.iter().any(|t| RAW.contains(&t.as_str()));
        let f_canned = f.tokens.iter().any(|t| t == "canned");
        match state {
            State::Cooked => s += if f_cooked { 1.5 } else if f_raw { -1.5 } else { 0.0 },
            State::Dry => s += if f_raw { 1.5 } else if f_cooked { -2.0 } else { 0.0 },
            State::Canned => s += if f_canned { 1.5 } else if f_cooked { 0.5 } else { -0.5 },
            State::Any => s += if f_raw { 0.8 } else if f_cooked { -0.8 } else if f_canned { -0.4 } else { 0.0 },
        }
        for o in ODD {
            if f.tokens.iter().any(|t| t == o) && !has_line(o) {
                s -= 1.0;
            }
        }
        // brand names are written in capitals in the USDA data
        if f.name.split(|c: char| !c.is_alphabetic()).any(|w| w.len() > 3 && w.chars().all(|c| c.is_ascii_uppercase())) {
            s -= 1.5;
        }
        s += match f.source {
            'F' => 0.4,
            'S' => 0.3,
            _ => 0.0,
        };
        if best.map(|b| s > b.0).unwrap_or(true) {
            best = Some((s, i));
        }
    }
    // must at least share the main word in the food's own heading
    best.filter(|(s, i)| *s > 0.5 && d.foods[*i].tokens.contains(&headword)).map(|b| b.1)
}

fn my_food_match(line_key: &str) -> Option<usize> {
    let mine = MY_FOODS.read().ok()?;
    let padded = format!(" {} ", line_key);
    let mut best: Option<(usize, usize)> = None;
    for (i, f) in mine.iter().enumerate() {
        for n in std::iter::once(f.name.as_str()).chain(f.aliases.split(',')) {
            let k = ingredients::match_key(n);
            if !k.is_empty() && padded.contains(&format!(" {} ", k)) && best.map(|b| k.len() > b.1).unwrap_or(true) {
                best = Some((i, k.len()));
            }
        }
    }
    best.map(|b| b.0)
}

/// Pick the food for an ingredient line.
pub fn match_line(p: &ParsedIngredient, line: &str) -> Option<Matched> {
    let line_key = ingredients::match_key(line);
    if let Some(i) = my_food_match(&line_key) {
        return Some(Matched::Mine(i));
    }
    let state = line_state(line);
    let lk: Vec<String> = keyw(line);
    // "water or unsweetened almond milk": the first choice is what's counted;
    // "skim or lactose-free milk" / "green or brown lentils": the first word goes with the food
    let mut opts = p.name.split(" or ");
    let first = opts.next().unwrap_or(&p.name).trim().to_string();
    let others: Vec<String> = p.name.split(" or ").skip(1).flat_map(keyw).collect();
    let name = match opts.next() {
        Some(second) if second.trim().contains(' ') && alias_match(&ingredients::match_key(&first), &[], state).is_none() => {
            format!("{} {}", first, second.split_whitespace().last().unwrap_or(""))
        }
        _ => first,
    };
    let name_key = ingredients::match_key(&name);
    // words that only belong to the other choices don't count ("lactose free" in "skim or lactose-free milk")
    let lk: Vec<String> = lk.into_iter().filter(|w| !others.contains(w) || name_key.split(' ').any(|n| n == w)).collect();
    if let Some(i) = alias_match(&name_key, &lk, state) {
        return Some(Matched::Db(i));
    }
    if let Some(i) = search(&name_key, &lk, state) {
        return Some(Matched::Db(i));
    }
    for other in p.name.split(" or ").skip(1) {
        let ok = ingredients::match_key(other);
        if let Some(i) = alias_match(&ok, &lk, state).or_else(|| search(&ok, &lk, state)) {
            return Some(Matched::Db(i));
        }
    }
    // the name may be in the prep text ("1 can, chickpeas")
    let pk = ingredients::match_key(&p.prep);
    alias_match(&pk, &lk, state).or_else(|| search(&pk, &lk, state)).map(Matched::Db)
}

/// True if the food database knows this ingredient name (used to judge parsed names).
/// (The ingredient parser calls this, so it must not parse.)
pub fn lookup(name: &str) -> Option<&'static str> {
    let k = ingredients::match_key(name);
    if k.is_empty() {
        return None;
    }
    alias_match(&k, &[], State::Any).or_else(|| search(&k, &[], State::Any)).map(|i| db().foods[i].name)
}

// ---------------------------------------------------------------------------------------------
// Weighing

/// Weight of standard count units (g) when the food's own data has no weight for them.
fn unit_grams(unit: &str) -> Option<f32> {
    Some(match unit {
        "clove" => 5.0,
        "slice" => 30.0,
        "can" | "tin" => 400.0,
        "stick" => 113.0,
        "pinch" => 0.4,
        "dash" => 0.6,
        "sprig" => 1.0,
        "bunch" => 60.0,
        "handful" => 30.0,
        "package" | "pkg" | "pack" | "packet" => 250.0,
        "jar" => 450.0,
        "bottle" => 750.0,
        "leaf" => 0.5,
        "drop" => 0.05,
        "piece" | "pc" => 50.0,
        "cube" => 10.0,
        "head" => 500.0,
        "stalk" => 40.0,
        "fillet" | "filet" => 170.0,
        "scoop" => 30.0,
        "spray" => 0.3,
        _ => return None,
    })
}

fn size_word(p: &ParsedIngredient, line: &str) -> Option<&'static str> {
    let w = keyw(&format!("{} {}", p.size_note, line));
    ["small", "medium", "large"].into_iter().find(|s| w.iter().any(|x| x == s))
}

fn amount(p: &ParsedIngredient) -> Option<f32> {
    p.quantity.map(|a| match p.quantity_max {
        Some(b) => (a + b) / 2.0,
        None => a,
    }).map(|v| v as f32)
}

/// "1 whole chicken (4 lbs)", "1 can (400 g) chickpeas": the weight written in brackets.
fn bracket_weight(line: &str) -> Option<f32> {
    let lc = line.to_lowercase();
    let mut rest = lc.as_str();
    while let Some(i) = rest.find('(') {
        let inner = &rest[i + 1..];
        let end = inner.find(')').unwrap_or(inner.len());
        let part = inner[..end].trim().trim_start_matches("about ").trim_start_matches("approx. ");
        let num: String = part.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
        if let Ok(q) = num.parse::<f64>() {
            let unit = part[num.len()..].trim().split(|c: char| !c.is_alphabetic()).next().unwrap_or("");
            let p = parse_ingredient(&format!("1 {} x", unit));
            if let Some(u) = p.unit {
                if ingredients::measurement_kind(&u) == Some('w') {
                    return ingredients::convert(q, &u, "gram").map(|g| g as f32);
                }
            }
        }
        rest = &inner[end..];
    }
    None
}

fn grams_db(p: &ParsedIngredient, line: &str, f: &DbFood) -> Option<f32> {
    let q = amount(p)?;
    let lk = ingredients::match_key(line);
    // nutritional yeast flakes are far lighter than baker's yeast granules
    let density = if lk.contains("nutritional yeast") { Some(0.25) } else { f.density };
    let count_unit = p.unit.as_deref().map(|u| ingredients::measurement_kind(u).is_none()).unwrap_or(true);
    let g = if let (true, Some(w)) = (count_unit, bracket_weight(line)) {
        // a whole bird is weighed with its bones; USDA: 1 lb ready-to-cook chicken → 276 g meat and skin
        let edible = if lk.contains("whole") && (f.name.starts_with("Chicken") || f.name.starts_with("Turkey")) { 0.61 } else { 1.0 };
        Some(q * w * edible)
    } else {
        match p.unit.as_deref() {
            Some(u) => match ingredients::measurement_kind(u) {
                Some('w') => ingredients::convert(q as f64, u, "gram").map(|g| g as f32),
                Some('v') => ingredients::convert(q as f64, u, "milliliter").map(|ml| ml as f32 * density.unwrap_or(1.0)),
                Some(_) => None,
                None => {
                    let mut u1 = ingredients::singular(u);
                    if u1 == "leave" {
                        u1 = "leaf".into();
                    }
                    if f.name.starts_with("Spices") && f.count(&u1).is_none() {
                        // a cinnamon stick, a bay leaf, a star anise: a gram or two
                        return Some(q * 2.0);
                    }
                    count_like(f, &u1).or_else(|| unit_grams(&u1)).map(|g| g * q)
                }
            },
            None => {
                // "6 butter lettuce leaves", "2 garlic cloves": the part named in the line
                let part = keyw(&p.name).into_iter().rev().find_map(|w| {
                    let w = if w == "leave" { "leaf".to_string() } else { w };
                    if PARTS.contains(&w.as_str()) { f.count(&w).or_else(|| unit_grams(&w)) } else { None }
                });
                let name_words = keyw(&p.name);
                let per = part
                    .or_else(|| size_word(p, line).and_then(|s| count_like(f, s)))
                    // "2 ribeye steaks", "8 chicken drumsticks"
                    .or_else(|| name_words.iter().rev().filter(|w| !f.head.contains(w)).find_map(|w| count_like(f, w)))
                    // recipes mean large eggs unless they say otherwise
                    .or_else(|| if f.head.iter().any(|h| h == "egg") { f.count("large") } else { None })
                    .or_else(|| WHOLE_ITEM.iter().find(|(k, _)| lk.contains(k)).map(|(_, g)| *g))
                    .or_else(|| count_like(f, "each"))
                    .or_else(|| count_like(f, "medium"))
                    .or_else(|| if f.name.starts_with("Spices") { Some(2.0) } else { None });
                per.map(|g| g * q)
            }
        }
    }?;
    // oil for deep frying: most stays in the pan; fried foods soak up roughly a tenth of it
    if f.name.starts_with("Oil") && (lk.contains("for frying") || lk.contains("deep fry") || lk.contains("for deep frying")) {
        return Some(g * 0.1);
    }
    // ready-made / home-cooked seitan is vital wheat gluten plus about 1.5 times its weight in water
    if f.name == "Vital wheat gluten" && !lk.contains("gluten") {
        return Some(g * 0.4);
    }
    Some(g)
}

fn grams_mine(p: &ParsedIngredient, f: &MyFood) -> Option<f32> {
    let q = amount(p)?;
    let serving_unit = ingredients::singular(f.serving.split_whitespace().last().unwrap_or("serving"));
    match p.unit.as_deref() {
        Some(u) => match ingredients::measurement_kind(u) {
            Some('w') => ingredients::convert(q as f64, u, "gram").map(|g| g as f32),
            Some('v') => {
                // volume: use the label's own volume if it gave one ("1 cup (240 ml)"), else water
                let ml = ingredients::convert(q as f64, u, "milliliter")? as f32;
                Some(ml * if f.serving_ml > 0.0 { f.serving_g / f.serving_ml } else { 1.0 })
            }
            Some(_) => None,
            // "scoop", "serving", "bar"… all mean one label serving
            None => {
                let _ = &serving_unit;
                Some(q * f.serving_g)
            }
        },
        None => Some(q * f.serving_g),
    }
}

// ---------------------------------------------------------------------------------------------
// Results

#[derive(Clone, Debug, Default)]
pub struct LineResult {
    pub line: String,
    pub grams: Option<f32>,
    pub matched: Option<String>,
    pub source: String,
    pub note: String,
    /// Matched to the closest food the databases have (not the thing itself): its name.
    pub stand_in: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NutritionResult {
    pub total: Nutrients,
    pub lines: Vec<LineResult>,
    pub counted: usize,
    pub skipped: usize,
}

fn small_amount(line: &str) -> bool {
    let lc = line.to_lowercase();
    ["to taste", "garnish", "topping", "pinch", "for serving", "optional", "dash"].iter().any(|w| lc.contains(w))
}

pub fn calculate(r: &Recipe) -> NutritionResult {
    let mut res = NutritionResult { total: Nutrients::ZERO, ..Default::default() };
    let mine = MY_FOODS.read().map(|m| m.clone()).unwrap_or_default();
    let d = db();
    for line in r.ingredient_lines() {
        let p = parse_ingredient(line);
        if p.is_heading || p.name.trim().is_empty() {
            continue;
        }
        let mut lr = LineResult { line: line.to_string(), ..Default::default() };
        let (per100, grams): (Nutrients, Option<f32>) = match match_line(&p, line) {
            Some(Matched::Mine(i)) => {
                let f = &mine[i];
                lr.matched = Some(f.name.clone());
                lr.source = "My Foods (from the label)".into();
                let k = if f.serving_g > 0.0 { 100.0 / f.serving_g } else { 0.0 };
                (f.per_serving.scaled(k), grams_mine(&p, f))
            }
            Some(Matched::Db(i)) => {
                let f = &d.foods[i];
                lr.matched = Some(f.name.to_string());
                lr.source = f.source_name().into();
                lr.stand_in = stand_in_for(line, f.name).map(|s| s.to_string());
                (f.nutrients(), grams_db(&p, line, f))
            }
            None => {
                lr.note = "Not found in the food databases".into();
                if !small_amount(line) {
                    res.skipped += 1;
                }
                res.lines.push(lr);
                continue;
            }
        };
        match grams {
            Some(g) => {
                lr.grams = Some(g);
                res.total = res.total.plus(&per100.scaled(g / 100.0));
                res.counted += 1;
            }
            None => {
                if small_amount(line) || p.quantity.is_none() && (line.to_lowercase().contains("spray") || line.to_lowercase().contains("salt")) {
                    lr.note = "Small amount, not counted".into();
                } else {
                    lr.note = "No amount to weigh".into();
                    res.skipped += 1;
                }
            }
        }
        res.lines.push(lr);
    }
    // nothing counted: every value is unknown, not a known 0
    if res.counted == 0 {
        res.total = Nutrients::default();
    }
    res
}

/// A recipe total from per-serving values typed in by hand: a blank stays unknown, a typed 0 stays 0, a negative becomes 0, a non-finite value is unknown.
pub fn hand_typed(per_serving: &Nutrients, servings: f32) -> Nutrients {
    let f = |v: Option<f32>| v.map(|v| v * servings).filter(|v| v.is_finite()).map(|v| v.max(0.0));
    Nutrients {
        calories: f(per_serving.calories),
        protein_g: f(per_serving.protein_g),
        fat_g: f(per_serving.fat_g),
        carbs_g: f(per_serving.carbs_g),
        fiber_g: f(per_serving.fiber_g),
        sugar_g: f(per_serving.sugar_g),
        sodium_mg: f(per_serving.sodium_mg),
    }
}

/// Re-estimate every recipe whose nutrition wasn't typed in by hand (after My Foods change).
/// Returns how many changed.
pub fn refresh_estimates(lib: &mut Library) -> usize {
    set_my_foods(&lib.my_foods);
    let mut n = 0;
    for r in lib.recipes.iter_mut().filter(|r| !r.nutrition_edited) {
        let new = calculate(r).total;
        if r.nutrition.as_ref() != Some(&new) {
            r.nutrition = Some(new);
            r.modified = now_secs();
            n += 1;
        }
    }
    n
}

/// Refuse a name another food already has. The form calls this before saving; the inbox doesn't, because it replaces by name on purpose.
pub fn check_my_food_name(lib: &Library, f: &MyFood) -> Result<(), String> {
    let name = f.name.trim();
    match lib.my_foods.iter().any(|x| x.id != f.id && x.name.trim().eq_ignore_ascii_case(name)) {
        true => Err(crate::foods::NAME_TAKEN.to_string()),
        false => Ok(()),
    }
}

fn my_food_slot(lib: &Library, f: &MyFood) -> Option<usize> {
    lib.my_foods.iter().position(|x| if f.id != 0 { x.id == f.id } else { x.name.trim().eq_ignore_ascii_case(f.name.trim()) })
}

/// Add a food to My Foods, or replace one, then re-estimate. A food with an id replaces that id only
/// (so a rename never lands on another food); a new food (id 0) replaces the one with the same name.
pub fn save_my_food(lib: &mut Library, mut f: MyFood) {
    f.modified = now_secs();
    match my_food_slot(lib, &f) {
        Some(i) => {
            f.id = lib.my_foods[i].id;
            lib.my_foods[i] = f;
        }
        None => {
            f.id = lib.new_id();
            lib.my_foods.push(f);
        }
    }
    lib.my_foods.sort_by_key(|x| x.name.to_lowercase());
    refresh_estimates(lib);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Nutrients;

    fn kcal(ings: &str) -> f32 {
        calculate(&Recipe { ingredients: ings.into(), servings: "1".into(), ..Default::default() }).total.calories.expect("calories known")
    }
    fn total(ings: &str) -> Nutrients {
        calculate(&Recipe { ingredients: ings.into(), servings: "1".into(), ..Default::default() }).total
    }
    fn matched(line: &str) -> String {
        let r = calculate(&Recipe { ingredients: line.into(), ..Default::default() });
        r.lines[0].matched.clone().unwrap_or_default()
    }

    #[test]
    fn every_alias_is_in_the_database() {
        for (a, _, name) in ALIASES {
            assert!(find_db_by_name(name).is_some(), "alias {:?} points at missing food {:?}", a, name);
        }
        assert!(food_count() > 10_000);
    }

    #[test]
    fn tsv_row_blank_is_unknown_zero_is_zero() {
        let f = parse_row("X1\tS\t1\tTest food\t100\t1\t1\t1\t\t0\t5\t").unwrap();
        let n = f.nutrients();
        assert_eq!((n.fiber_g, n.sugar_g, n.sodium_mg, n.calories), (None, Some(0.0), Some(5.0), Some(100.0)));
        assert_eq!(f.per100[4], 0.0);
        // a food missing macros is kept (not None), with those macros unknown
        let f = parse_row("X2\tF\t1\tTest food two\t50\t\t2\t\t1\t1\t1\t").unwrap();
        let n = f.nutrients();
        assert_eq!((n.protein_g, n.carbs_g, n.fat_g, n.calories), (None, None, Some(2.0), Some(50.0)));
        assert!(!in_estimator(&f));
        assert!(parse_row("# comment").is_none() && parse_row("").is_none() && parse_row("a\tb").is_none());
    }

    #[test]
    fn database_has_every_source() {
        let count = |s: char| db().foods.iter().filter(|f| f.source == s).count();
        for (s, min) in [('F', 300), ('S', 7_000), ('C', 5_500), ('N', 5_000)] {
            assert!(count(s) >= min, "source {} has {} foods, minimum {}", s, count(s), min);
        }
    }

    #[test]
    fn fndds_never_matches_recipe_lines() {
        let d = db();
        assert!(d.by_token.values().flatten().all(|&i| d.foods[i].source != 'N'));
        let mut none = 0;
        for f in d.foods.iter().filter(|f| f.source == 'N') {
            match find_db_by_name(f.name) {
                None => none += 1,
                // the same words can also name an F, S or C food; that one is fine
                Some(i) => assert_ne!(d.foods[i].source, 'N', "{}", f.name),
            }
        }
        assert!(none > 1_000, "{}", none);
    }

    #[test]
    fn incomplete_foods_never_match_recipe_lines() {
        let d = db();
        assert!(d.by_token.values().flatten().all(|&i| in_estimator(&d.foods[i])));
        let mut incomplete = 0;
        for f in d.foods.iter().filter(|f| f.known & 0b1110 != 0b1110) {
            incomplete += 1;
            assert!(find_db_by_name(f.name).map(|i| in_estimator(&d.foods[i])).unwrap_or(true), "{}", f.name);
        }
        assert!(incomplete >= 1, "the table has no food missing a macro to test with");
    }

    /// (exact table name, published kcal per 100 g from USDA SR Legacy / FDC or CNF), each within 10%.
    #[test]
    fn food_spot_checks() {
        let table: [(&str, f32); 25] = [
            ("Egg, whole, raw, fresh", 143.0),
            ("Bananas, raw", 89.0),
            ("Rice, white, long-grain, regular, raw, enriched", 365.0),
            ("Rice, white, long-grain, regular, enriched, cooked", 130.0),
            ("Oil, olive, salad or cooking", 884.0),
            ("Chicken, broiler or fryers, breast, skinless, boneless, meat only, raw", 120.0),
            ("Cereals, oats, regular and quick, not fortified, dry", 380.0),
            ("Milk, whole, 3.25% milkfat, with added vitamin D", 61.0),
            ("Cheese, cheddar", 405.0),
            ("Lentils, raw", 352.0),
            ("Lentils, mature seeds, cooked, boiled, without salt", 116.0),
            ("Tofu, raw, firm, prepared with calcium sulfate", 144.0),
            ("Peanut butter, smooth, vitamin and mineral fortified", 590.0),
            ("Broccoli, raw", 34.0),
            ("Yogurt, Greek, plain, whole milk", 97.0),
            ("Butter, salted", 717.0),
            ("Nuts, almonds", 579.0),
            ("Spinach, raw", 23.0),
            ("Sugars, granulated", 387.0),
            ("Wheat flour, white, all-purpose, enriched, bleached", 364.0),
            ("Avocados, raw, all commercial varieties", 160.0),
            ("Fish, salmon, Atlantic, farmed, raw", 208.0),
            ("Apple, raw, with skin", 52.0),
            ("Chicken, breast, boneless, skinless, raw", 120.0),
            ("Milk, fluid, whole, pasteurized, homogenized, 3.25% M.F.", 61.0),
        ];
        let d = db();
        for (name, kcal) in table {
            let f = &d.foods[find_db_by_name(name).unwrap_or_else(|| panic!("{} is not in the table", name))];
            assert!((f.per100[0] - kcal).abs() <= kcal * 0.10, "{}: table {} kcal, published {}", name, f.per100[0], kcal);
        }
        let protein = |n: &str| d.foods[find_db_by_name(n).unwrap()].per100[1];
        assert!((protein("Egg, whole, raw, fresh") - 12.6).abs() < 1.3);
        assert!((protein("Chicken, broiler or fryers, breast, skinless, boneless, meat only, raw") - 22.5).abs() < 2.3);
    }

    #[test]
    fn cnf_milk_250ml_density() {
        // CNF 2026 gives grams per measure (250 ml of whole milk is about 258 g), not a multiple of 100 g
        let f = &db().foods[find_db_by_name("Milk, fluid, whole, pasteurized, homogenized, 3.25% M.F.").unwrap()];
        assert_eq!(f.source, 'C');
        let g = f.density.unwrap();
        assert!(g > 1.02 && g < 1.04, "{}", g);
    }

    #[test]
    fn count_keys_are_words() {
        for f in &db().foods {
            for (k, _) in &f.counts {
                assert!(k.len() >= 3 && !k.chars().any(|c| c.is_ascii_digit()), "{}: count key {:?}", f.name, k);
            }
        }
    }

    #[test]
    fn butter_and_flour() {
        // 1 cup flour 125 g ≈ 455; 2 tbsp butter 28 g ≈ 201; 2 eggs 100 g ≈ 143
        let k = kcal("1 cup flour\n2 tbsp butter\n2 eggs");
        assert!(k > 740.0 && k < 840.0, "{}", k);
    }

    #[test]
    fn the_old_mistakes_are_fixed() {
        // 1 cup unsweetened almond milk ≈ 35 kcal, not a cup of almonds
        let k = kcal("1 cup unsweetened almond milk");
        assert!(k > 20.0 && k < 50.0, "{}", k);
        assert!(matched("150 g cauliflower rice").starts_with("Cauliflower"));
        assert!(matched("100 g cooked brown rice").contains("cooked"));
        assert!(matched("75 grams (dry) black rice").contains("raw"));
        assert!(matched("120 g plain nonfat Greek yogurt").contains("nonfat"));
        assert!(matched("5 oz 99% lean chicken breast, diced").contains("breast"));
        assert!(matched("200 milliliters skim or lactose-free milk").contains("nonfat"));
        assert!(matched("50 g red split lentils (dry)").starts_with("Lentils, pink"));
        assert!(matched("100 g canned chickpeas, drained and rinsed").contains("canned"));
        assert!(matched("45 g protein powder (about 1.5 scoops)").contains("Protein powder"));
        assert!(matched("1 tbsp tahini").contains("tahini"));
        assert!(matched("15 g chia seeds").contains("chia"));
        assert!(matched("10 g hemp hearts").contains("hemp"));
        // 180 g raw chicken breast ≈ 200 kcal (not 390 from whole chicken with skin)
        let k = kcal("180 grams chicken breast, raw, skinless/boneless");
        assert!(k > 180.0 && k < 230.0, "{}", k);
    }

    #[test]
    fn counts_use_the_database_portions() {
        // a medium banana is ~118 g ≈ 105 kcal; a small one less
        let m = kcal("1 medium banana");
        let s = kcal("1 small banana");
        assert!(m > 95.0 && m < 115.0, "{}", m);
        assert!(s < m, "{} {}", s, m);
        // 2 garlic cloves ≈ 6 g
        let r = calculate(&Recipe { ingredients: "2 garlic cloves, minced".into(), ..Default::default() });
        assert!(r.lines[0].grams.unwrap() < 12.0);
    }

    #[test]
    fn my_foods_come_first() {
        let mine = MyFood {
            name: "Gold Standard Whey".into(),
            aliases: "test whey blend".into(),
            serving: "1 scoop".into(),
            serving_g: 31.0,
            per_serving: Nutrients { calories: Some(120.0), protein_g: Some(24.0), ..Default::default() },
            ..Default::default()
        };
        set_my_foods(&[mine]);
        let t = total("1.5 scoops test whey blend");
        set_my_foods(&[]);
        let (k, p) = (t.calories.expect("calories known"), t.protein_g.expect("protein known"));
        assert!((k - 180.0).abs() < 0.5, "{}", k);
        assert!((p - 36.0).abs() < 0.5, "{}", p);
        assert_eq!(t.fiber_g, None, "a value the label did not give stays unknown");
    }

    #[test]
    fn unknown_fiber_stays_unknown_in_recipe_totals() {
        assert_eq!(matched("200 g chicken breast"), "Chicken, breast, boneless, skinless, raw");
        let food = db().foods.iter().find(|f| f.name == "Chicken, breast, boneless, skinless, raw").unwrap().nutrients();
        assert_eq!((food.fiber_g, food.sugar_g), (None, None));
        let t = total("200 g chicken breast
2 tbsp butter");
        assert_eq!((t.fiber_g, t.sugar_g), (None, None));
        for v in [t.calories, t.protein_g, t.fat_g, t.sodium_mg] {
            assert!(v.is_some_and(|v| v > 0.0), "{:?}", t);
        }
        assert!(t.carbs_g.is_some());
    }

    #[test]
    fn reported_zero_stays_zero_in_recipe_totals() {
        assert_eq!(matched("2 tbsp butter"), "Butter, salted");
        assert_eq!(matched("2 eggs"), "Egg, whole, raw, fresh");
        let t = total("2 tbsp butter
2 eggs");
        assert_eq!(t.fiber_g, Some(0.0));
        assert!(t.sugar_g.is_some_and(|v| v > 0.0) && t.sodium_mg.is_some_and(|v| v > 0.0), "{:?}", t);
    }

    #[test]
    fn no_matched_lines_is_unknown_not_zero() {
        let empty = calculate(&Recipe::default());
        assert_eq!((empty.counted, empty.total), (0, Nutrients::default()));
        let r = calculate(&Recipe { ingredients: "3 flurbs of zzqx".into(), ..Default::default() });
        assert!(r.lines[0].matched.is_none());
        assert_eq!((r.counted, r.total), (0, Nutrients::default()));
    }

    #[test]
    fn hand_typed_keeps_blank_unknown_and_zero_zero() {
        let typed = Nutrients { calories: Some(150.0), protein_g: Some(0.0), fat_g: Some(-3.0), carbs_g: Some(f32::NAN), sugar_g: Some(2.5), ..Default::default() };
        let t = hand_typed(&typed, 4.0);
        assert_eq!((t.calories, t.protein_g, t.fat_g, t.carbs_g), (Some(600.0), Some(0.0), Some(0.0), None));
        assert_eq!((t.fiber_g, t.sugar_g, t.sodium_mg), (None, Some(10.0), None));
    }

    #[test]
    fn a_food_name_never_lands_on_another_food() {
        let food = |id, name: &str| MyFood { id, name: name.into(), ..Default::default() };
        let lib = Library { my_foods: vec![food(1, "Apple"), food(2, "Bread")], ..Default::default() };
        assert!(check_my_food_name(&lib, &food(0, " bread ")).is_err(), "adding a taken name is refused");
        assert!(check_my_food_name(&lib, &food(1, "Bread")).is_err(), "renaming onto another food is refused");
        assert!(check_my_food_name(&lib, &food(2, "bread")).is_ok(), "keeping its own name is fine");
        assert_eq!(my_food_slot(&lib, &food(2, "Apple")), Some(1), "an edit finds its slot by id, not by the new name");
        assert_eq!(my_food_slot(&lib, &food(0, "apple")), Some(0), "the inbox still replaces by name");
    }

    #[test]
    fn liquids_use_the_plain_cup_not_the_whipped_one() {
        // 2 cups heavy cream ≈ 476 g ≈ 1620 kcal (the database also lists "1 cup, whipped" = 120 g)
        let r = calculate(&Recipe { ingredients: "2 cups heavy cream".into(), ..Default::default() });
        let g = r.lines[0].grams.unwrap();
        assert!(g > 460.0 && g < 490.0, "{}", g);
    }

    #[test]
    fn pieces_and_pantry_items() {
        let g = |l: &str| calculate(&Recipe { ingredients: l.into(), ..Default::default() }).lines[0].grams;
        let steak = g("2 ribeye steaks").unwrap();
        assert!(steak > 300.0 && steak < 700.0, "{}", steak);
        let legs = g("8 chicken drumsticks").unwrap();
        assert!(legs > 700.0 && legs < 1300.0, "{}", legs);
        assert!(g("1 cinnamon stick").unwrap() < 5.0);
        assert!(g("1 pie crust").unwrap() > 150.0);
        assert!(g("4 cups dashi or water").is_some());
        // 4 cups of frying oil: only ~90 g ends up in the food
        let oil = g("4 cups vegetable oil, for frying").unwrap();
        assert!(oil < 120.0, "{}", oil);
    }

    #[test]
    fn stand_ins_are_named() {
        let r = calculate(&Recipe { ingredients: "1.5 tbsp gochujang (Korean chili paste)\n100 g broccoli florets".into(), ..Default::default() });
        assert_eq!(r.lines[0].stand_in.as_deref(), Some("gochujang"));
        assert_eq!(r.lines[1].stand_in, None);
    }

    #[test]
    fn tricky_lines_from_real_recipes() {
        assert!(matched("200 g non-fat plain Greek yogurt").contains("nonfat"));
        assert!(matched("55 g dried green or brown lentils, rinsed").starts_with("Lentils"));
        assert!(matched("150 ml water or unsweetened almond milk").contains("water"));
        assert!(matched("150 g ground turkey, 99% lean, raw").contains("fat free"));
        assert!(matched("1 cup milk chocolate chips") != "Milk, whole, 3.25% milkfat, with added vitamin D");
        // cooked seitan ≈ 150 kcal per 100 g, not the 370 of dry gluten powder
        let k = kcal("150 g cooked seitan, cubed");
        assert!(k > 180.0 && k < 260.0, "{}", k);
        let k = kcal("60 g vital wheat gluten (dry, before hydrating), made into seitan");
        assert!(k > 200.0 && k < 240.0, "{}", k);
        // a 4 lb whole chicken: about 1.1 kg of meat and skin
        let r = calculate(&Recipe { ingredients: "1 whole chicken (4 lbs)".into(), ..Default::default() });
        let g = r.lines[0].grams.unwrap();
        assert!(g > 1000.0 && g < 1200.0, "{}", g);
        let r = calculate(&Recipe { ingredients: "1/2 medium cucumber, thinly sliced".into(), ..Default::default() });
        assert!(r.lines[0].grams.unwrap() > 100.0);
        let r = calculate(&Recipe { ingredients: "2 tbsp nutritional yeast".into(), ..Default::default() });
        assert!(r.lines[0].grams.unwrap() < 10.0);
    }
}
