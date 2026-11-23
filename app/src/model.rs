//! Library data: recipes, categories, pantry, grocery lists, meals, menus and settings.
//! Fields follow what a home cook needs from a Recipe, PantryItem, Meal and MenuItem.

use serde::{Deserialize, Serialize};

/// The number that decides whether library.json is too new for this build. A file with a higher
/// `format` opens read-only (`storage::LoadOutcome::TooNew`) and is never re-saved. Bump it only
/// for changes an older build would lose when it re-saves the file. Format 2 adds unknown
/// nutrients (left out of the file) to My Foods and to recipe nutrition, and pieces to My Foods;
/// an older build would turn them into 0 and drop them. Recipe nutrition joined format 2 in 02-08,
/// before any format-2 build was installed, so it needed no bump: an old file's plain numbers load as known.
/// Format 3 adds a barcode to My Foods; an older build would drop it.
pub const LIBRARY_FORMAT: u32 = 3;

pub const CUISINES_PARENT: &str = "Cuisines";
/// World cuisines offered as sub-categories of "Cuisines".
pub const CUISINES: &[&str] = &[
    "African", "American", "Brazilian", "British", "Caribbean", "Chinese", "Ethiopian", "Filipino", "French", "German",
    "Greek", "Indian", "Indonesian", "Italian", "Japanese", "Korean", "Lebanese", "Mexican", "Middle Eastern",
    "Moroccan", "Peruvian", "Spanish", "Thai", "Turkish", "Vietnamese",
];

/// Words that point to a cuisine, for filing imported recipes (our own list).
pub const CUISINE_HINTS: &[(&str, &[&str])] = &[
    ("Indian", &["garam masala", "tikka", "masala", "curry leaves", "paneer", "dal", "naan", "biryani", "korma", "tandoori", "ghee"]),
    ("Thai", &["pad thai", "green curry paste", "red curry paste", "lemongrass", "thai basil", "galangal", "kaffir"]),
    ("Japanese", &["miso", "mirin", "dashi", "teriyaki", "wakame", "nori", "sushi", "ramen", "panko", "sake"]),
    ("Chinese", &["chili bean paste", "doubanjiang", "hoisin", "five spice", "shaoxing", "bok choy", "szechuan", "sichuan", "oyster sauce"]),
    ("Korean", &["gochujang", "gochugaru", "kimchi", "bibimbap", "bulgogi"]),
    ("Vietnamese", &["pho", "banh mi", "rice paper", "nuoc cham"]),
    ("Mexican", &["tortilla", "tortillas", "enchilada", "taco", "tacos", "salsa", "chipotle", "jalapeno", "masa", "queso fresco", "tomatillo"]),
    ("Italian", &["parmesan", "mozzarella", "risotto", "pasta", "spaghetti", "pesto", "prosciutto", "ricotta", "lasagna", "marinara"]),
    ("French", &["ratatouille", "gruyere", "crepe", "crepes", "dijon", "bouquet garni", "creme fraiche"]),
    ("Greek", &["feta", "tzatziki", "kalamata", "phyllo", "souvlaki", "moussaka"]),
    ("Middle Eastern", &["tahini", "za'atar", "sumac", "falafel", "shakshuka", "harissa", "pita", "hummus"]),
    ("Spanish", &["paella", "saffron", "chorizo", "smoked paprika", "manchego"]),
    ("Caribbean", &["jerk", "allspice", "scotch bonnet", "plantain"]),
    ("Moroccan", &["tagine", "ras el hanout", "preserved lemon", "couscous"]),
    ("Ethiopian", &["berbere", "injera", "niter kibbeh"]),
];

/// The most likely cuisine for a recipe from its name and ingredients (two or more hints, or one in the name).
pub fn guess_cuisine(name: &str, ingredients: &str) -> Option<&'static str> {
    let n = format!(" {} ", name.to_lowercase());
    let i = format!(" {} ", ingredients.to_lowercase().replace('\n', " "));
    let mut best: Option<(&'static str, usize)> = None;
    for (c, words) in CUISINE_HINTS {
        let mut score = 0;
        for w in *words {
            if n.contains(&format!(" {} ", w)) || n.contains(&format!(" {}", w)) {
                score += 2;
            }
            if i.contains(w) {
                score += 1;
            }
        }
        if score >= 2 && best.map(|b| score > b.1).unwrap_or(true) {
            best = Some((c, score));
        }
    }
    best.map(|b| b.0)
}
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------- calendar helpers (day numbers = days since 1970-01-01) ----------

pub fn days_from_civil(y: i32, m: u32, d: u32) -> i32 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i32 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i32) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 0 = Sunday
pub fn weekday(day: i32) -> u32 {
    ((day + 4).rem_euclid(7)) as u32
}

pub fn today() -> i32 {
    (now_secs() / 86400) as i32
}

pub fn fmt_date(day: i32) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{}/{}/{}", m, d, y)
}

pub fn parse_date(s: &str) -> Option<i32> {
    let parts: Vec<&str> = s.trim().split(['/', '-']).collect();
    if parts.len() != 3 {
        return None;
    }
    let nums: Vec<i32> = parts.iter().filter_map(|p| p.trim().parse().ok()).collect();
    if nums.len() != 3 {
        return None;
    }
    let (y, m, d) = if parts[0].len() == 4 {
        (nums[0], nums[1], nums[2])
    } else if nums[0] > 12 && nums[1] <= 12 {
        (nums[2], nums[1], nums[0]) // 25/12/2026 can only be day-first
    } else {
        (nums[2], nums[0], nums[1])
    };
    if !(1..=12).contains(&m) || d < 1 || d as u32 > days_in_month(y, m as u32) {
        return None;
    }
    Some(days_from_civil(y, m as u32, d as u32))
}

#[test]
fn parse_date_rejects_impossible_dates() {
    assert_eq!(parse_date("12/25/2026"), parse_date("25/12/2026"));
    assert_eq!(parse_date("2026-12-25"), parse_date("12/25/2026"));
    assert!(parse_date("12/25/2026").is_some());
    assert_eq!(parse_date("13/13/2026"), None);
    assert_eq!(parse_date("2/30/2026"), None);
    assert_eq!(parse_date("tomorrow"), None);
}

/// First day of the week (US default Sunday = 0).
pub const FIRST_DAY_OF_WEEK: u32 = 0;

pub fn start_of_week(day: i32) -> i32 {
    day - ((weekday(day) + 7 - FIRST_DAY_OF_WEEK) % 7) as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    Expired,
    Today,
    ThisWeek,
    ThisMonth,
    Later,
}

/// Pantry expiry status, checked in this order: before today → expired; same day; same week;
/// same year and month; otherwise later.
pub fn expiry_status(expires: i32, today: i32) -> Expiry {
    if expires < today {
        Expiry::Expired
    } else if expires == today {
        Expiry::Today
    } else if start_of_week(expires) == start_of_week(today) {
        Expiry::ThisWeek
    } else {
        let (y1, m1, _) = civil_from_days(expires);
        let (y2, m2, _) = civil_from_days(today);
        if y1 == y2 && m1 == m2 { Expiry::ThisMonth } else { Expiry::Later }
    }
}

pub const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October",
    "November", "December",
];

pub fn days_in_month(y: i32, m: u32) -> u32 {
    let next = if m == 12 { days_from_civil(y + 1, 1, 1) } else { days_from_civil(y, m + 1, 1) };
    (next - days_from_civil(y, m, 1)) as u32
}

// ---------- data ----------

/// Calories, protein, fat, carbs, fiber, sugar and sodium; a value that isn't known is `None`, never 0.
/// Unknown is written by leaving the key out (never `null`), so an older build reads it as 0
/// instead of failing to parse the file. A plain number in an older file reads as known, 0 included. A sum with an unknown part is unknown.
#[derive(Clone, Copy, Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Nutrients {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calories: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protein_g: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fat_g: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carbs_g: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fiber_g: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sugar_g: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sodium_mg: Option<f32>,
}

impl Nutrients {
    /// Every value known and 0: where a total starts.
    pub const ZERO: Nutrients = Nutrients { calories: Some(0.0), protein_g: Some(0.0), fat_g: Some(0.0), carbs_g: Some(0.0), fiber_g: Some(0.0), sugar_g: Some(0.0), sodium_mg: Some(0.0) };

    /// Every known value times `k`; unknown stays unknown.
    pub fn scaled(&self, k: f32) -> Nutrients {
        let s = |v: Option<f32>| v.map(|v| v * k);
        Nutrients { calories: s(self.calories), protein_g: s(self.protein_g), fat_g: s(self.fat_g), carbs_g: s(self.carbs_g), fiber_g: s(self.fiber_g), sugar_g: s(self.sugar_g), sodium_mg: s(self.sodium_mg) }
    }

    /// The sum; a field is unknown when it is unknown on either side (a total built from unknown parts shows "–").
    pub fn plus(&self, o: &Nutrients) -> Nutrients {
        let p = |a: Option<f32>, b: Option<f32>| Some(a? + b?);
        Nutrients {
            calories: p(self.calories, o.calories),
            protein_g: p(self.protein_g, o.protein_g),
            fat_g: p(self.fat_g, o.fat_g),
            carbs_g: p(self.carbs_g, o.carbs_g),
            fiber_g: p(self.fiber_g, o.fiber_g),
            sugar_g: p(self.sugar_g, o.sugar_g),
            sodium_mg: p(self.sodium_mg, o.sodium_mg),
        }
    }
}

/// A food entered from its package label ("My Foods"); used before the food databases.
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default)]
pub struct MyFood {
    pub id: u64,
    pub name: String,
    /// Other names recipes use for it, comma separated ("protein powder, whey").
    pub aliases: String,
    /// Serving as printed on the label ("1 scoop", "2 tbsp").
    pub serving: String,
    pub serving_g: f32,
    /// Millilitres in one serving, if the label gives it (lets cups/tbsp be weighed).
    pub serving_ml: f32,
    /// What one serving holds; a value the label didn't give is unknown (None), never 0.
    pub per_serving: Nutrients,
    /// Name of one piece of this food ("slice"); empty when it has none.
    pub piece: String,
    /// Grams in one piece; 0 means the food has no piece weight.
    pub piece_g: f32,
    /// Where the numbers came from ("Photo of the label", "Open Food Facts, barcode …").
    pub source: String,
    /// The pack's barcode (EAN-13, UPC-A or EAN-8 digits), so scanning it again finds this food; empty when not known. Added in format 3.
    pub barcode: String,
    pub modified: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Recipe {
    pub id: u64,
    pub uid: String,
    pub name: String,
    pub ingredients: String,
    pub directions: String,
    pub description: String,
    pub notes: String,
    pub servings: String,
    pub difficulty: String,
    pub prep_time: String,
    pub cook_time: String,
    pub total_time: String,
    pub rating: u8,
    pub favorite: bool,
    pub pinned: bool,
    pub in_trash: bool,
    pub source: String,
    pub source_url: String,
    pub video_url: String,
    pub categories: Vec<String>,
    /// Nutrition per whole recipe; a value no counted ingredient reports is unknown. `nutrition_edited` means
    /// the person typed it in (or it came from the recipe's page).
    pub nutrition: Option<Nutrients>,
    pub nutrition_edited: bool,
    pub created: u64,
    pub modified: u64,
    pub photo_seed: u32,
    /// Uploaded photo: a file name in the library's photos folder (empty = painted placeholder).
    pub photo: String,
    /// Outstanding importer warnings the person hasn't cleared yet.
    pub review_flags: Vec<String>,
    pub merged_count: u32,
    pub sent_to_tracker: Option<u64>,
}

impl Default for Recipe {
    fn default() -> Self {
        let t = now_secs();
        Recipe {
            id: 0,
            uid: String::new(),
            name: String::new(),
            ingredients: String::new(),
            directions: String::new(),
            description: String::new(),
            notes: String::new(),
            servings: String::new(),
            difficulty: String::new(),
            prep_time: String::new(),
            cook_time: String::new(),
            total_time: String::new(),
            rating: 0,
            favorite: false,
            pinned: false,
            in_trash: false,
            source: String::new(),
            source_url: String::new(),
            video_url: String::new(),
            categories: vec![],
            nutrition: None,
            nutrition_edited: false,
            created: t,
            modified: t,
            photo_seed: 0,
            photo: String::new(),
            review_flags: vec![],
            merged_count: 0,
            sent_to_tracker: None,
        }
    }
}

impl Recipe {
    pub fn ingredient_lines(&self) -> Vec<&str> {
        self.ingredients.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect()
    }
    pub fn steps(&self) -> Vec<String> {
        crate::ingredients::split_steps(&self.directions)
    }
    pub fn servings_count(&self) -> f32 {
        let n: String = self
            .servings
            .trim()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        n.parse::<f32>().ok().filter(|v| *v > 0.0).unwrap_or(1.0)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct PantryItem {
    pub id: u64,
    pub name: String,
    pub quantity: String,
    pub aisle: String,
    pub bought: Option<i32>,
    pub expires: Option<i32>,
    pub in_stock: bool,
    pub modified: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct GroceryItem {
    pub id: u64,
    pub name: String,
    pub note: String,
    pub quantity: String,
    pub aisle: String,
    pub recipe: String,
    pub purchased: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct GroceryList {
    pub id: u64,
    pub name: String,
    pub items: Vec<GroceryItem>,
    pub modified: u64,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
pub enum MealType {
    #[default]
    Breakfast,
    Lunch,
    Dinner,
    Snack,
}

impl MealType {
    pub const ALL: [MealType; 4] = [MealType::Breakfast, MealType::Lunch, MealType::Dinner, MealType::Snack];
    pub fn label(&self) -> &'static str {
        match self {
            MealType::Breakfast => "Breakfast",
            MealType::Lunch => "Lunch",
            MealType::Dinner => "Dinner",
            MealType::Snack => "Snack",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct Meal {
    pub id: u64,
    pub day: i32,
    pub kind: MealType,
    pub recipe_id: Option<u64>,
    pub name: String,
    pub modified: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct MenuEntry {
    pub day: u32,
    pub kind: MealType,
    pub recipe_id: Option<u64>,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct Menu {
    pub id: u64,
    pub name: String,
    pub days: u32,
    pub entries: Vec<MenuEntry>,
    pub modified: u64,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    pub sync_folder: String,
    pub auto_sync: bool,
    pub device_name: String,
    pub last_sync: Option<u64>,
    pub update_channel: UpdateChannel,
    pub tracker_service: String,
    pub tracker_auto_send: bool,
    pub tracker_folder: String,
    pub snapshots_to_keep: usize,
    pub unit_style_short: bool,
    pub update_remind_after: Option<u64>,
    pub dark_mode: bool,
    /// Last chosen sorts, restored on the next start (recipes: 0 name, 1 rating, 2 most recent)
    pub recipe_sort: u8,
    pub pantry_sort: u8,
    pub grocery_by_recipe: bool,
    /// The person's contact email for Open Food Facts requests; empty until they give one. Sent only to Open Food Facts.
    pub off_email: String,
    /// The sidebar's tab order as the person arranged it; empty is the default order. Read it through `sidebar::order`.
    pub sidebar_order: Vec<String>,
    /// The tab Tiffin opens on; empty is Home. Read it through `sidebar::launch_page`.
    pub launch_page: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sync_folder: String::new(),
            auto_sync: true,
            device_name: std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This computer".into()),
            last_sync: None,
            update_channel: UpdateChannel::Stable,
            tracker_service: "Cronometer".into(),
            tracker_auto_send: false,
            tracker_folder: String::new(),
            snapshots_to_keep: 50,
            unit_style_short: false,
            update_remind_after: None,
            dark_mode: false,
            recipe_sort: 0,
            pantry_sort: 0,
            grocery_by_recipe: false,
            off_email: String::new(),
            sidebar_order: vec![],
            launch_page: String::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct RuleHealth {
    pub domain: String,
    pub last_result: String,
    pub last_checked: u64,
    pub needs_update: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Library {
    pub format: u32,
    pub app_version: String,
    pub next_id: u64,
    pub recipes: Vec<Recipe>,
    pub categories: Vec<String>,
    pub pantry: Vec<PantryItem>,
    pub grocery_lists: Vec<GroceryList>,
    pub meals: Vec<Meal>,
    pub menus: Vec<Menu>,
    pub settings: Settings,
    /// uid -> deletion time, so a deletion syncs instead of the item coming back.
    pub tombstones: Vec<(String, u64)>,
    pub rule_health: Vec<RuleHealth>,
    /// Site rules loaded from a rules update file (override the built-in set by domain).
    pub rule_overrides: Vec<crate::rules::SiteRule>,
    /// Foods entered from package labels.
    pub my_foods: Vec<MyFood>,
    /// Which nutrition estimator last filled in the saved estimates.
    pub nutrition_estimator: u32,
    /// Top-level category -> label colour index, fixed when the category first appears so a
    /// shelf never changes colour when others are added.
    pub label_colors: Vec<(String, u8)>,
}

impl Default for Library {
    fn default() -> Self {
        Library {
            format: LIBRARY_FORMAT,
            app_version: APP_VERSION.into(),
            next_id: 1,
            recipes: vec![],
            label_colors: vec![],
            categories: vec![],
            pantry: vec![],
            grocery_lists: vec![],
            meals: vec![],
            menus: vec![],
            settings: Settings::default(),
            tombstones: vec![],
            rule_health: vec![],
            rule_overrides: vec![],
            my_foods: vec![],
            nutrition_estimator: 0,
        }
    }
}

impl Library {
    pub fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn new_uid(&mut self) -> String {
        let id = self.new_id();
        format!("{:x}-{:x}-{}", now_secs(), id, self.settings.device_name.len())
    }

    /// Add the "Cuisines" group and its sub-categories if missing (also run on older libraries).
    pub fn ensure_cuisines(&mut self) -> bool {
        let mut changed = false;
        for c in std::iter::once(CUISINES_PARENT.to_string()).chain(CUISINES.iter().map(|c| format!("{}/{}", CUISINES_PARENT, c))) {
            if !self.categories.contains(&c) {
                self.categories.push(c);
                changed = true;
            }
        }
        changed
    }

    pub fn recipe(&self, id: u64) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.id == id)
    }
    pub fn recipe_mut(&mut self, id: u64) -> Option<&mut Recipe> {
        self.recipes.iter_mut().find(|r| r.id == id)
    }

    pub fn add_recipe(&mut self, mut r: Recipe) -> u64 {
        r.id = self.new_id();
        if r.uid.is_empty() {
            r.uid = self.new_uid();
        }
        if r.photo_seed == 0 {
            r.photo_seed = (r.id as u32).wrapping_mul(2654435761) | 1;
        }
        for c in &r.categories {
            if !self.categories.contains(c) {
                self.categories.push(c.clone());
            }
        }
        let id = r.id;
        self.recipes.push(r);
        id
    }
}

// ---------- sample library (shown on first run and in screenshot mode) ----------

struct S<'a> {
    name: &'a str,
    rating: u8,
    cats: &'a [&'a str],
    ing: &'a str,
    dir: &'a str,
    serv: &'a str,
    prep: &'a str,
    cook: &'a str,
}

pub fn sample_library(today: i32) -> Library {
    let mut lib = Library::default();
    lib.categories = [
        "Breakfast", "Breakfast/Pancakes", "Desserts", "Desserts/Cakes", "Meat", "Meat/Beef", "Pizza",
        "Sauces", "Seafood", "Seafood/Shellfish", "Sides", "Soups", "Vegetarian",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    lib.ensure_cuisines();

    let samples = [
        S { name: "Buttermilk Waffles with Berries", rating: 4, cats: &["Breakfast"], ing: "2 cups all-purpose flour\n2 tbsp sugar\n2 tsp baking powder\n1/2 tsp salt\n2 large eggs\n1 3/4 cups buttermilk\n1/2 cup butter, melted\n1 cup mixed berries", dir: "Heat the waffle iron. Whisk the flour, sugar, baking powder and salt in a large bowl.\nBeat the eggs with the buttermilk and melted butter, then stir into the dry ingredients until just combined.\nCook in the waffle iron for 4 minutes until golden. Serve with berries.", serv: "4 servings", prep: "10 min", cook: "20 min" },
        S { name: "Overnight Oats with Blueberries", rating: 3, cats: &["Breakfast", "Vegetarian"], ing: "1 cup rolled oats\n1 cup milk\n1/2 cup plain yogurt\n1 tbsp honey\n1 cup blueberries", dir: "Stir the oats, milk, yogurt and honey together in a jar.\nCover and chill for 8 hours or overnight.\nTop with blueberries before serving.", serv: "2 servings", prep: "5 min", cook: "0 min" },
        S { name: "Tomato Basil Toasts", rating: 2, cats: &["Cuisines/Italian", "Sides"], ing: "1 baguette, sliced\n4 tomatoes, diced\n2 cloves garlic, minced\n1/4 cup fresh basil, chopped\n3 tbsp olive oil\nSalt and pepper to taste", dir: "Toast the baguette slices under the grill for 2 minutes.\nMix the tomatoes, garlic, basil and olive oil. Season.\nSpoon onto the toasts and serve at once.", serv: "6 servings", prep: "15 min", cook: "5 min" },
        S { name: "Maple Bacon Cupcakes", rating: 3, cats: &["Desserts", "Desserts/Cakes"], ing: "1 1/2 cups all-purpose flour\n1 cup sugar\n1/2 cup butter\n2 eggs\n1/2 cup milk\n1/4 cup maple syrup\n4 slices bacon, cooked and crumbled", dir: "Heat the oven to 350°F.\nCream the butter and sugar, beat in the eggs, then fold in flour and milk.\nBake for 18 minutes. Cool, drizzle with maple syrup and top with bacon.", serv: "12 cupcakes", prep: "20 min", cook: "18 min" },
        S { name: "Dark Chocolate Tart", rating: 4, cats: &["Desserts"], ing: "1 pie crust\n8 oz dark chocolate, chopped\n1 cup heavy cream\n2 tbsp butter\n1 pinch salt", dir: "Blind bake the crust for 15 minutes.\nHeat the cream until steaming and pour over the chocolate. Stir in butter and salt.\nPour into the crust and chill for 2 hours.", serv: "8 servings", prep: "15 min", cook: "15 min" },
        S { name: "Weeknight Spaghetti Bolognese", rating: 3, cats: &["Cuisines/Italian", "Meat", "Meat/Beef"], ing: "1 lb ground beef\n1 onion, diced\n2 cloves garlic, minced\n1 can crushed tomatoes (28 oz)\n2 tbsp tomato paste\n1 lb spaghetti\n1/2 cup parmesan cheese, grated", dir: "Brown the beef with the onion for 8 minutes.\nAdd garlic, tomatoes and paste. Simmer for 25 minutes.\nCook the spaghetti, drain and toss with the sauce. Top with parmesan.", serv: "6 servings", prep: "10 min", cook: "35 min" },
        S { name: "Layered Jar Salad", rating: 3, cats: &["Vegetarian"], ing: "2 tbsp vinaigrette\n1/2 cup chickpeas\n1/2 cucumber, diced\n1 cup cherry tomatoes\n1/4 cup feta cheese\n2 cups lettuce", dir: "Pour the dressing into the bottom of a large jar.\nLayer chickpeas, cucumber, tomatoes and feta, then the lettuce on top.\nShake into a bowl to serve.", serv: "1 serving", prep: "10 min", cook: "0 min" },
        S { name: "Spiced Carrot Cake", rating: 5, cats: &["Desserts", "Desserts/Cakes"], ing: "2 cups all-purpose flour\n2 tsp cinnamon\n1 tsp baking soda\n4 eggs\n1 1/2 cups sugar\n1 cup vegetable oil\n3 cups carrots, grated\n8 oz cream cheese", dir: "Heat the oven to 350°F and grease two cake pans.\nWhisk the eggs, sugar and oil, then fold in flour, cinnamon, soda and carrots.\nBake for 30 minutes. Cool and frost with cream cheese.", serv: "12 servings", prep: "25 min", cook: "30 min" },
        S { name: "Orange Cream Cupcakes", rating: 3, cats: &["Desserts"], ing: "1 1/2 cups flour\n3/4 cup sugar\n1/2 cup butter\n2 eggs\n1 orange, zested and juiced\n1 cup whipped cream", dir: "Heat the oven to 350°F.\nMix everything except the cream until smooth and bake for 20 minutes.\nPipe whipped cream on the cooled cupcakes.", serv: "12 cupcakes", prep: "15 min", cook: "20 min" },
        S { name: "Peach Smoothie", rating: 5, cats: &["Breakfast", "Vegetarian"], ing: "2 peaches, sliced\n1 banana\n3/4 cup orange juice\n1/2 cup yogurt\n1 cup ice", dir: "Blend everything until smooth, about 1 minute.\nPour into glasses and serve right away.", serv: "2 servings", prep: "5 min", cook: "0 min" },
        S { name: "Caramel Custard with Raspberries", rating: 3, cats: &["Desserts"], ing: "3/4 cup sugar\n4 eggs\n2 cups milk\n1 tsp vanilla extract\n1 cup raspberries", dir: "Melt half the sugar into a caramel and pour into ramekins.\nWhisk eggs, milk, the rest of the sugar and vanilla. Pour over the caramel.\nBake in a water bath at 325°F for 40 minutes. Chill for 4 hours and serve with raspberries.", serv: "6 servings", prep: "20 min", cook: "40 min" },
        S { name: "Thin Crepes", rating: 3, cats: &["Breakfast", "Breakfast/Pancakes"], ing: "1 cup flour\n2 eggs\n1 1/4 cups milk\n2 tbsp butter, melted\n1 pinch salt", dir: "Whisk everything into a smooth batter and rest for 30 minutes.\nCook thin rounds in a hot buttered pan for 1 minute per side.", serv: "10 crepes", prep: "5 min", cook: "20 min" },
        S { name: "Roasted Pumpkin Soup", rating: 4, cats: &["Soups", "Vegetarian"], ing: "2 lbs pumpkin, cubed\n1 onion, chopped\n3 cups vegetable broth, low sodium\n1/2 cup cream or half and half\n1 tbsp olive oil\n1 tsp nutmeg", dir: "Roast the pumpkin with oil at 400°F for 35 minutes.\nSoften the onion, add pumpkin and broth and simmer for 15 minutes.\nBlend until smooth, stir in the cream and nutmeg.", serv: "4 servings", prep: "15 min", cook: "50 min" },
        S { name: "Avocado Egg Toast", rating: 4, cats: &["Breakfast"], ing: "2 slices sprouted wheat bread, toasted\n1 avocado\n2 eggs\n1 tsp lemon juice\n1 pinch chili flakes", dir: "Mash the avocado with lemon juice and spread on the toast.\nFry or poach the eggs for 3 minutes and place on top.\nFinish with chili flakes.", serv: "2 servings", prep: "5 min", cook: "5 min" },
        S { name: "Garlic Butter Shrimp", rating: 4, cats: &["Seafood", "Seafood/Shellfish"], ing: "1 lb shrimp, peeled\n4 tbsp butter\n4 cloves garlic, minced\n1 lemon, juiced\n2 tbsp parsley, chopped", dir: "Melt the butter and cook the garlic for 1 minute.\nAdd the shrimp and cook for 3 minutes per side.\nFinish with lemon juice and parsley.", serv: "4 servings", prep: "10 min", cook: "8 min" },
        S { name: "Margherita Flatbread Pizza", rating: 4, cats: &["Pizza", "Cuisines/Italian", "Vegetarian"], ing: "1 pizza dough\n1/2 cup tomato sauce\n8 oz mozzarella cheese\n1/4 cup fresh basil leaves\n1 tbsp olive oil", dir: "Heat the oven to 500°F.\nStretch the dough, spread the sauce and add the mozzarella.\nBake for 10 minutes and top with basil and oil.", serv: "4 servings", prep: "15 min", cook: "10 min" },
        S { name: "Grilled Steaks with Mushrooms", rating: 5, cats: &["Meat", "Meat/Beef"], ing: "2 ribeye steaks\n8 oz mushrooms, sliced\n1/2 cup beef broth\n2 tbsp butter\n2 sprigs thyme", dir: "Grill the steaks for 5 minutes per side and rest for 10 minutes.\nSauté the mushrooms in butter with thyme, add the broth and reduce for 5 minutes.\nSpoon over the steaks.", serv: "2 servings", prep: "10 min", cook: "25 min" },
        S { name: "Simple Marinara Sauce", rating: 3, cats: &["Sauces", "Cuisines/Italian"], ing: "2 cans whole tomatoes (28 oz)\n3 tbsp olive oil\n4 cloves garlic, sliced\n1 tsp dried oregano\nSalt to taste", dir: "Warm the oil and garlic for 2 minutes.\nAdd tomatoes and oregano, crush and simmer for 30 minutes.", serv: "6 servings", prep: "5 min", cook: "30 min" },
        S { name: "Vanilla Bean Crème Brûlée", rating: 5, cats: &["Desserts"], ing: "2 cups heavy cream\n6 egg yolks\n1/4 cup granulated sugar\n1 tsp vanilla extract\nPinch of salt\nDemerara sugar, for topping\nFresh mixed berries, for garnish", dir: "Heat the oven to 325°F. Whisk the egg yolks and sugar until smooth, then whisk in the cream, vanilla and salt.\nPour into ramekins set in a roasting pan and add hot water halfway up their sides.\nBake for 35 minutes until just set. Chill for 4 hours.\nSprinkle with demerara sugar and torch until caramelised. Serve with berries.", serv: "4 servings", prep: "15 min", cook: "35 min" },
        S { name: "Lemon Herb Roast Chicken", rating: 4, cats: &["Meat"], ing: "1 whole chicken (4 lbs)\n1 lemon, halved\n4 cloves garlic\n2 tbsp butter, softened\n1 tbsp fresh thyme\nSalt and pepper", dir: "Heat the oven to 425°F.\nRub the chicken with butter, thyme, salt and pepper; stuff with lemon and garlic.\nRoast for 1 hour 15 minutes and rest for 15 minutes before carving.", serv: "6 servings", prep: "15 min", cook: "1 hr 15 min" },
        // ---- international (indices 20+) ----
        S { name: "Chicken Tikka Masala", rating: 5, cats: &["Cuisines/Indian", "Meat"], ing: "1 1/2 lbs chicken breast, cubed\n1 cup plain yogurt\n2 tbsp garam masala\n1 onion, diced\n4 cloves garlic, minced\n1 tbsp ginger, grated\n1 can crushed tomatoes (15 oz)\n1/2 cup heavy cream\n2 tbsp butter\n1/4 cup cilantro, chopped", dir: "Toss the chicken with the yogurt and half the garam masala; marinate for 30 minutes.\nSear the chicken in butter for 6 minutes and set aside.\nSoften the onion, garlic and ginger, add the rest of the garam masala and the tomatoes, and simmer for 15 minutes.\nStir in the cream and chicken and simmer for 10 minutes. Top with cilantro.", serv: "4 servings", prep: "40 min", cook: "35 min" },
        S { name: "Chana Masala", rating: 4, cats: &["Cuisines/Indian", "Vegetarian"], ing: "2 cans chickpeas (15 oz), drained\n1 onion, diced\n3 cloves garlic, minced\n1 tbsp ginger, grated\n1 can diced tomatoes (15 oz)\n1 tbsp garam masala\n1 tsp cumin\n2 tbsp vegetable oil\n1 lemon, juiced", dir: "Cook the onion in oil for 8 minutes, then add garlic, ginger and spices for 1 minute.\nAdd the tomatoes and chickpeas and simmer for 20 minutes.\nFinish with lemon juice and serve with rice.", serv: "4 servings", prep: "10 min", cook: "30 min" },
        S { name: "Pad Thai", rating: 5, cats: &["Cuisines/Thai", "Seafood"], ing: "8 oz rice noodles\n8 oz shrimp, peeled\n2 eggs\n3 tbsp fish sauce\n2 tbsp brown sugar\n2 tbsp lime juice\n1 cup bean sprouts\n3 green onions, sliced\n1/4 cup peanuts, chopped\n2 tbsp vegetable oil", dir: "Soak the noodles in hot water for 8 minutes and drain.\nStir-fry the shrimp in oil for 2 minutes, push aside and scramble the eggs.\nAdd noodles, fish sauce, sugar and lime juice and toss for 2 minutes.\nFold in sprouts and green onions; top with peanuts.", serv: "3 servings", prep: "15 min", cook: "10 min" },
        S { name: "Thai Green Curry", rating: 4, cats: &["Cuisines/Thai", "Meat"], ing: "1 lb chicken thighs, sliced\n3 tbsp green curry paste\n1 can coconut milk (14 oz)\n1 cup green beans\n1 bell pepper, sliced\n1 tbsp fish sauce\n1 tsp brown sugar\n1/4 cup fresh basil leaves", dir: "Fry the curry paste in a little coconut milk for 2 minutes.\nAdd the chicken and cook for 5 minutes, then the rest of the coconut milk.\nAdd the vegetables, fish sauce and sugar and simmer for 10 minutes. Finish with basil.", serv: "4 servings", prep: "15 min", cook: "20 min" },
        S { name: "Beef Street Tacos", rating: 5, cats: &["Cuisines/Mexican", "Meat", "Meat/Beef"], ing: "1 lb ground beef\n1 tbsp chili powder\n1 tsp cumin\n12 small corn tortillas\n1 onion, finely diced\n1/2 cup cilantro, chopped\n2 limes, cut in wedges\n1/2 cup salsa", dir: "Brown the beef with the chili powder and cumin for 8 minutes.\nWarm the tortillas in a dry pan for 30 seconds per side.\nFill with beef, onion and cilantro; serve with lime and salsa.", serv: "4 servings", prep: "10 min", cook: "10 min" },
        S { name: "Black Bean Enchiladas", rating: 4, cats: &["Cuisines/Mexican", "Vegetarian"], ing: "2 cans black beans (15 oz), drained\n8 flour tortillas\n2 cups enchilada sauce\n2 cups cheddar cheese, shredded\n1 cup corn\n1 tsp cumin", dir: "Heat the oven to 375°F.\nMash half the beans with the corn and cumin and fold in the rest.\nRoll the filling in the tortillas, cover with sauce and cheese, and bake for 20 minutes.", serv: "4 servings", prep: "15 min", cook: "20 min" },
        S { name: "Miso Soup with Tofu", rating: 4, cats: &["Cuisines/Japanese", "Soups", "Vegetarian"], ing: "4 cups dashi or water\n3 tbsp white miso\n7 oz silken tofu, cubed\n2 tbsp dried wakame\n2 green onions, sliced", dir: "Bring the dashi to a simmer and add the wakame for 2 minutes.\nWhisk the miso with a ladle of broth and stir it back in; do not boil.\nAdd the tofu and green onions and serve.", serv: "4 servings", prep: "5 min", cook: "10 min" },
        S { name: "Chicken Teriyaki", rating: 4, cats: &["Cuisines/Japanese", "Meat"], ing: "1 1/2 lbs chicken thighs\n1/4 cup soy sauce\n2 tbsp mirin\n2 tbsp honey\n1 tbsp ginger, grated\n1 tbsp vegetable oil\n1 tbsp sesame seeds", dir: "Sear the chicken in oil for 5 minutes per side.\nAdd the soy sauce, mirin, honey and ginger and simmer for 5 minutes until glossy.\nSlice, spoon over the sauce and scatter sesame seeds.", serv: "4 servings", prep: "5 min", cook: "15 min" },
        S { name: "Mapo Tofu", rating: 4, cats: &["Cuisines/Chinese"], ing: "1 lb firm tofu, cubed\n4 oz ground pork\n2 tbsp chili bean paste\n2 cloves garlic, minced\n1 tbsp ginger, minced\n1 cup chicken broth\n1 tbsp cornstarch\n2 green onions, sliced", dir: "Brown the pork for 4 minutes, then add garlic, ginger and chili bean paste for 1 minute.\nAdd the broth and tofu and simmer for 5 minutes.\nThicken with cornstarch mixed in water and top with green onions.", serv: "4 servings", prep: "10 min", cook: "12 min" },
        S { name: "Vegetable Fried Rice", rating: 3, cats: &["Cuisines/Chinese", "Vegetarian", "Sides"], ing: "3 cups cooked rice, cold\n2 eggs\n1 cup frozen peas and carrots\n3 green onions, sliced\n2 tbsp soy sauce\n1 tbsp sesame oil\n2 tbsp vegetable oil", dir: "Scramble the eggs in a hot wok and set aside.\nStir-fry the vegetables for 3 minutes, add the rice and fry for 5 minutes.\nAdd soy sauce, sesame oil, eggs and green onions and toss.", serv: "4 servings", prep: "10 min", cook: "10 min" },
        S { name: "Greek Village Salad", rating: 4, cats: &["Cuisines/Greek", "Vegetarian", "Sides"], ing: "4 tomatoes, cut in wedges\n1 cucumber, sliced\n1/2 red onion, sliced\n1/2 cup kalamata olives\n7 oz feta cheese\n3 tbsp olive oil\n1 tsp dried oregano", dir: "Arrange the tomatoes, cucumber, onion and olives in a bowl.\nTop with the feta, drizzle with oil and sprinkle with oregano.", serv: "4 servings", prep: "10 min", cook: "0 min" },
        S { name: "Shakshuka", rating: 5, cats: &["Cuisines/Middle Eastern", "Breakfast", "Vegetarian"], ing: "2 tbsp olive oil\n1 onion, diced\n1 bell pepper, diced\n3 cloves garlic, minced\n1 tsp cumin\n1 tsp paprika\n1 can crushed tomatoes (28 oz)\n6 eggs\n1/4 cup feta cheese", dir: "Soften the onion and pepper in oil for 8 minutes; add garlic and spices for 1 minute.\nAdd the tomatoes and simmer for 10 minutes.\nMake six wells, crack in the eggs, cover and cook for 6 minutes. Top with feta.", serv: "4 servings", prep: "10 min", cook: "25 min" },
        S { name: "Bibimbap", rating: 4, cats: &["Cuisines/Korean"], ing: "2 cups cooked rice\n8 oz ground beef\n2 tbsp soy sauce\n1 cup spinach\n1 carrot, julienned\n1 cup bean sprouts\n4 eggs\n2 tbsp gochujang\n1 tbsp sesame oil", dir: "Brown the beef with soy sauce for 6 minutes.\nBlanch the spinach and sprouts for 1 minute; sauté the carrot for 2 minutes.\nFry the eggs. Top bowls of rice with everything, gochujang and sesame oil.", serv: "4 servings", prep: "20 min", cook: "15 min" },
        S { name: "Chicken Pho", rating: 4, cats: &["Cuisines/Vietnamese", "Soups"], ing: "8 cups chicken broth\n1 onion, halved\n1 piece ginger (3 inch)\n2 star anise\n1 cinnamon stick\n1 lb chicken breast\n8 oz rice noodles\n2 tbsp fish sauce\n1 cup bean sprouts\n1/2 cup fresh basil leaves\n1 lime, cut in wedges", dir: "Char the onion and ginger, then simmer with the broth and spices for 30 minutes.\nPoach the chicken in the broth for 15 minutes, then shred it.\nSoak the noodles and divide into bowls with the chicken; ladle over the strained broth with fish sauce.\nServe with sprouts, basil and lime.", serv: "4 servings", prep: "15 min", cook: "45 min" },
        S { name: "Seafood Paella", rating: 5, cats: &["Cuisines/Spanish", "Seafood", "Seafood/Shellfish"], ing: "2 cups short-grain rice\n4 cups chicken broth\n1 pinch saffron\n1 onion, diced\n1 bell pepper, diced\n2 tsp smoked paprika\n1 lb shrimp, peeled\n1 lb mussels\n1 cup frozen peas\n3 tbsp olive oil", dir: "Soften the onion and pepper in oil for 6 minutes; stir in paprika and rice.\nAdd the broth and saffron and simmer without stirring for 15 minutes.\nNestle in the shrimp, mussels and peas, cover and cook for 8 minutes.", serv: "6 servings", prep: "15 min", cook: "30 min" },
        S { name: "Jerk Chicken", rating: 4, cats: &["Cuisines/Caribbean", "Meat"], ing: "8 chicken drumsticks\n4 green onions\n2 jalapenos\n4 cloves garlic\n1 tbsp allspice\n1 tsp thyme\n2 limes, juiced\n2 tbsp soy sauce\n2 tbsp brown sugar", dir: "Blend everything except the chicken into a paste and coat the chicken; marinate for 2 hours.\nRoast at 425°F for 40 minutes, turning once.", serv: "4 servings", prep: "15 min", cook: "40 min" },
        S { name: "Ratatouille", rating: 4, cats: &["Cuisines/French", "Vegetarian", "Sides"], ing: "1 eggplant, cubed\n2 zucchini, sliced\n1 bell pepper, diced\n1 onion, diced\n4 tomatoes, chopped\n3 cloves garlic, minced\n1/4 cup olive oil\n1 tsp thyme", dir: "Cook the onion and pepper in oil for 5 minutes.\nAdd the eggplant and zucchini and cook for 10 minutes.\nAdd the tomatoes, garlic and thyme and simmer for 25 minutes.", serv: "6 servings", prep: "20 min", cook: "40 min" },
        S { name: "Crispy Falafel", rating: 4, cats: &["Cuisines/Middle Eastern", "Vegetarian"], ing: "2 cups dried chickpeas, soaked overnight\n1 onion, chopped\n4 cloves garlic\n1 cup parsley\n2 tsp cumin\n1 tsp coriander\n1 tsp baking powder\n4 cups vegetable oil, for frying", dir: "Pulse everything except the oil into a coarse paste and chill for 1 hour.\nShape into balls and fry at 350°F for 4 minutes until deep golden.\nServe in pita with tahini.", serv: "6 servings", prep: "20 min", cook: "15 min" },
    ];

    for (i, s) in samples.iter().enumerate() {
        let r = Recipe {
            name: s.name.into(),
            rating: s.rating,
            categories: s.cats.iter().map(|c| c.to_string()).collect(),
            ingredients: s.ing.into(),
            directions: s.dir.into(),
            servings: s.serv.into(),
            prep_time: s.prep.into(),
            cook_time: s.cook.into(),
            source: "Family Recipes".into(),
            difficulty: if i % 3 == 0 { "Easy".into() } else { "Medium".into() },
            favorite: i % 4 == 1,
            pinned: i == 7 || i == 18,
            created: 1_700_000_000 + i as u64 * 86_400,
            modified: 1_700_000_000 + i as u64 * 86_400,
            photo_seed: (i as u32 + 1) * 7919,
            ..Default::default()
        };
        lib.add_recipe(r);
    }
    for r in lib.recipes.iter_mut() {
        r.total_time = crate::ingredients::sum_times(&r.prep_time, &r.cook_time);
        r.nutrition = Some(crate::nutrition::calculate(r).total);
    }

    // Pantry
    let p = |name: &str, qty: &str, bought: i32, exp: Option<i32>, stock: bool| PantryItem {
        id: 0,
        name: name.into(),
        quantity: qty.into(),
        aisle: crate::ingredients::aisle_for(name).into(),
        bought: Some(bought),
        expires: exp,
        in_stock: stock,
        modified: 0,
    };
    let mut pantry = vec![
        p("cornstarch", "", today - 20, None, true),
        p("flour", "5 lbs", today - 60, None, false),
        p("brown sugar", "4 cups", today - 30, Some(today + 120), true),
        p("kalamata olives", "", today - 45, None, true),
        p("tomato sauce", "", today - 40, None, true),
        p("cheddar cheese", "8 oz", today - 10, None, true),
        p("eggs", "12", today - 3, None, true),
        p("milk", "1 gallon", today - 20, Some(today - 6), true),
        p("garlic", "10 cloves", today - 7, None, true),
        p("olive oil", "1 bottle", today - 50, None, true),
        p("salt", "", today - 200, None, true),
        p("butter", "1 lb", today - 4, Some(today + 30), true),
    ];
    for it in pantry.iter_mut() {
        it.id = lib.new_id();
    }
    lib.pantry = pantry;

    // Grocery lists
    let rec = |lib: &Library, i: usize| lib.recipes[i].name.clone();
    let gi = |name: &str, note: &str, qty: &str, recipe: String, purchased: bool| GroceryItem {
        id: 0,
        name: name.into(),
        note: note.into(),
        quantity: qty.into(),
        aisle: crate::ingredients::aisle_for(name).into(),
        recipe,
        purchased,
    };
    let mut items = vec![
        gi("baguette", "or crusty long loaf bread, sliced (12 pieces)", "1/2", rec(&lib, 2), false),
        gi("sprouted wheat bread, toasted", "", "1 slice", rec(&lib, 13), false),
        gi("orange juice", "", "3/4 cup", rec(&lib, 9), false),
        gi("water", "", "1 cup / 250 ml", rec(&lib, 12), true),
        gi("beef broth", "", "1/2 cup", rec(&lib, 16), false),
        gi("minestrone soup", "organic", "3 cans", "My Items".into(), false),
        gi("vegetable or chicken broth, low sodium", "", "3 cups / 750ml", rec(&lib, 12), false),
        gi("butter", "", "4.33 tablespoons", rec(&lib, 13), false),
        gi("cream or half and half", "any type", "1/2 cup", rec(&lib, 12), false),
        gi("shrimp", "peeled", "1 lb", rec(&lib, 14), false),
        gi("basil", "fresh", "1/4 cup", rec(&lib, 2), false),
        gi("tomatoes", "", "4", rec(&lib, 2), false),
    ];
    for it in items.iter_mut() {
        it.id = lib.new_id();
    }
    let id1 = lib.new_id();
    lib.grocery_lists.push(GroceryList { id: id1, name: "My Grocery List".into(), items, modified: 0 });
    for n in ["Weekly Groceries", "Bulk Shopping", "Specialty Items"] {
        let id = lib.new_id();
        lib.grocery_lists.push(GroceryList { id, name: n.into(), items: vec![], modified: 0 });
    }

    // Meals around today
    let (y, m, _) = civil_from_days(today);
    let first = days_from_civil(y, m, 1);
    let plan: [(i32, MealType, usize); 24] = [
        (0, MealType::Breakfast, 0), (0, MealType::Lunch, 6), (0, MealType::Dinner, 16),
        (1, MealType::Breakfast, 1), (1, MealType::Lunch, 2), (1, MealType::Dinner, 5),
        (2, MealType::Breakfast, 9), (2, MealType::Dinner, 15),
        (3, MealType::Breakfast, 13), (3, MealType::Dinner, 14),
        (4, MealType::Breakfast, 11), (4, MealType::Lunch, 6), (4, MealType::Snack, 7),
        (5, MealType::Breakfast, 1), (5, MealType::Dinner, 19),
        (6, MealType::Breakfast, 9), (6, MealType::Lunch, 12),
        (7, MealType::Breakfast, 0), (8, MealType::Lunch, 12), (8, MealType::Dinner, 5),
        (9, MealType::Breakfast, 13), (10, MealType::Dinner, 14), (11, MealType::Breakfast, 11), (11, MealType::Snack, 18),
    ];
    for (d, kind, ri) in plan {
        let id = lib.new_id();
        let rid = lib.recipes[ri].id;
        lib.meals.push(Meal { id, day: first + d, kind, recipe_id: Some(rid), name: lib.recipes[ri].name.clone(), modified: 0 });
    }

    // A menu
    let mid = lib.new_id();
    let r = |i: usize| (Some(lib.recipes[i].id), lib.recipes[i].name.clone());
    let entries = vec![
        MenuEntry { day: 1, kind: MealType::Dinner, recipe_id: r(5).0, name: r(5).1 },
        MenuEntry { day: 2, kind: MealType::Dinner, recipe_id: r(14).0, name: r(14).1 },
        MenuEntry { day: 3, kind: MealType::Dinner, recipe_id: r(15).0, name: r(15).1 },
        MenuEntry { day: 4, kind: MealType::Dinner, recipe_id: r(19).0, name: r(19).1 },
    ];
    lib.menus.push(Menu { id: mid, name: "Easy Weeknight Dinners".into(), days: 4, entries, modified: 0 });
    lib
}

/// Search words: the text split on spaces, empty parts dropped.
pub fn search_words(search: &str) -> Vec<String> {
    search.split_whitespace().map(|w| w.to_lowercase()).collect()
}

/// A recipe matches when ANY search word appears, case-insensitively,
/// in ANY searched field — name, ingredients, directions, description, notes, source, source
/// address. No words → everything matches.
pub fn recipe_matches(r: &Recipe, words: &[String]) -> bool {
    if words.is_empty() {
        return true;
    }
    let fields = [&r.name, &r.ingredients, &r.directions, &r.description, &r.notes, &r.source, &r.source_url];
    fields.iter().any(|f| {
        let f = f.to_lowercase();
        words.iter().any(|w| f.contains(w.as_str()))
    })
}

/// How many label colours there are (saffron … olive); the colours themselves live in each UI.
pub const LABEL_COUNT: usize = 10;

fn top_of(category: &str) -> String {
    category.split('/').next().unwrap_or("").trim().to_lowercase()
}

/// The label colour index of a category ("Desserts/Cakes" uses its top level); None = unlabelled.
pub fn label_index(colors: &[(String, u8)], category: &str) -> Option<usize> {
    let top = top_of(category);
    colors.iter().find(|(c, _)| *c == top).map(|(_, i)| *i as usize % LABEL_COUNT)
}

/// Give every top-level category without a colour the least-used one. Returns true if any changed.
pub fn assign_label_colors(lib: &mut Library) -> bool {
    let mut changed = false;
    let mut tops: Vec<String> = lib.categories.iter().filter(|c| !c.contains('/')).map(|c| top_of(c)).collect();
    tops.sort();
    for t in tops {
        if lib.label_colors.iter().any(|(c, _)| *c == t) {
            continue;
        }
        let mut uses = [0usize; LABEL_COUNT];
        for (_, i) in &lib.label_colors {
            uses[*i as usize % LABEL_COUNT] += 1;
        }
        let i = (0..LABEL_COUNT).min_by_key(|i| uses[*i]).unwrap_or(0);
        lib.label_colors.push((t, i as u8));
        changed = true;
    }
    changed
}

/// A recipe's label text and colour index. A course category ("Desserts") wins over a cuisine; a
/// recipe filed only under a cuisine shows the cuisine ("Mexican") in the Cuisines colour.
pub fn recipe_label(colors: &[(String, u8)], categories: &[String]) -> (String, Option<usize>) {
    let is_cuisine = |c: &String| c.split('/').next().map(|t| t.eq_ignore_ascii_case("cuisines")).unwrap_or(false);
    let pick = categories.iter().find(|c| !is_cuisine(c)).or_else(|| categories.first());
    match pick {
        Some(c) => {
            let text = if is_cuisine(c) { c.rsplit('/').next().unwrap_or(c) } else { c.split('/').next().unwrap_or(c) };
            (text.to_string(), label_index(colors, c))
        }
        None => ("Uncategorized".into(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nutrients_unknown_is_omitted_and_sums_stay_unknown() {
        let only_cal = Nutrients { calories: Some(120.0), ..Default::default() };
        assert_eq!(serde_json::to_string(&only_cal).unwrap(), r#"{"calories":120.0}"#);
        let back: Nutrients = serde_json::from_str(r#"{"calories":120.0}"#).unwrap();
        assert_eq!((back.calories, back.sugar_g), (Some(120.0), None));
        let a = Nutrients { calories: Some(100.0), protein_g: Some(5.0), ..Default::default() };
        let b = Nutrients { calories: Some(50.0), ..Default::default() };
        let sum = a.plus(&b);
        assert_eq!(sum.calories, Some(150.0));
        assert_eq!(sum.protein_g, None, "one unknown part makes the total unknown");
        let half = a.scaled(0.5);
        assert_eq!((half.calories, half.protein_g, half.fat_g), (Some(50.0), Some(2.5), None));
    }

    #[test]
    fn library_without_the_off_email_still_loads() {
        let lib: Library = serde_json::from_str(r#"{"format":1,"settings":{"sync_folder":"x","auto_sync":false}}"#).unwrap();
        assert_eq!((lib.settings.off_email.as_str(), lib.settings.sync_folder.as_str()), ("", "x"));
        let kept: Settings = serde_json::from_str(r#"{"off_email":"me@example.com"}"#).unwrap();
        assert_eq!(kept.off_email, "me@example.com");
    }

    #[test]
    fn old_plain_recipe_nutrition_loads_as_known() {
        let old = r#"{"id":5,"name":"Old soup","nutrition":{"calories":250.0,"protein_g":10.0,"fat_g":0.0,"carbs_g":30.0,"fiber_g":0.0,"sugar_g":5.0,"sodium_mg":200.0},"nutrition_edited":true}"#;
        let r: Recipe = serde_json::from_str(old).unwrap();
        let n = r.nutrition.expect("nutrition loads");
        assert_eq!((n.calories, n.protein_g, n.fat_g, n.carbs_g), (Some(250.0), Some(10.0), Some(0.0), Some(30.0)));
        assert_eq!((n.fiber_g, n.sugar_g, n.sodium_mg), (Some(0.0), Some(5.0), Some(200.0)));
        let again = serde_json::to_value(&r).unwrap();
        assert_eq!(again["nutrition"]["fiber_g"], 0.0, "a reported 0 is written back");
        for none in [r#"{"nutrition":null}"#, "{}"] {
            assert!(serde_json::from_str::<Recipe>(none).unwrap().nutrition.is_none());
        }
        let lib: Library = serde_json::from_str(&format!(r#"{{"format":1,"recipes":[{old}]}}"#)).unwrap();
        assert_eq!(lib.recipes[0].nutrition.unwrap().fiber_g, Some(0.0));
    }
}
