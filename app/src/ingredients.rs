//! Ingredient & direction parsing, unit conversion and store-section lookup.
//!
//! Each word of an ingredient line is classed as Other, Number, Fraction, Amount, Measurement, Unit,
//! Modifier, Instruction, Ingredient, Punctuation or Adverb. The conversion ratios and "roll-up"
//! thresholds are standard kitchen measures; the word lists are our own.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    Other,
    Number,
    Fraction,
    Amount,
    Measurement,
    Unit,
    Modifier,
    Instruction,
    Ingredient,
    Punctuation,
    Adverb,
}

/// Confidence below this is flagged for review.
pub const REVIEW_THRESHOLD: f32 = 0.75;

// ---------------- word tables ----------------

/// (canonical, plural, kind, variants). kind: 'v' volume, 'w' weight, 'l' length, 't' temperature.
pub const MEASUREMENTS: &[(&str, &str, char, &[&str])] = &[
    ("teaspoon", "teaspoons", 'v', &["tsp", "tsps", "tsp.", "teaspoon", "teaspoons", "teaspoonful", "teaspoonfuls", "tspn", "ts"]),
    ("tablespoon", "tablespoons", 'v', &["tbsp", "tbsps", "tbsp.", "tbs", "tbs.", "tbl", "tbl.", "tablespoon", "tablespoons", "tablespoonful", "tablespoonfuls", "tbspn"]),
    ("fluid ounce", "fluid ounces", 'v', &["fl oz", "fl. oz.", "fl.oz.", "fl oz.", "fluid ounce", "fluid ounces", "floz"]),
    ("cup", "cups", 'v', &["c", "c.", "cup", "cups", "cup(s)"]),
    ("pint", "pints", 'v', &["pt", "pt.", "pts", "pint", "pints"]),
    ("quart", "quarts", 'v', &["qt", "qt.", "qts", "quart", "quarts"]),
    ("gallon", "gallons", 'v', &["gal", "gal.", "gals", "gallon", "gallons"]),
    ("milliliter", "milliliters", 'v', &["ml", "ml.", "mls", "milliliter", "milliliters", "millilitre", "millilitres"]),
    ("centiliter", "centiliters", 'v', &["cl", "centiliter", "centiliters", "centilitre", "centilitres"]),
    ("deciliter", "deciliters", 'v', &["dl", "deciliter", "deciliters", "decilitre", "decilitres"]),
    ("liter", "liters", 'v', &["l", "l.", "liter", "liters", "litre", "litres", "ltr"]),
    ("milligram", "milligrams", 'w', &["mg", "mgs", "milligram", "milligrams", "milligramme", "milligrammes"]),
    ("gram", "grams", 'w', &["g", "g.", "gs", "gm", "gms", "gr", "grs", "gram", "grams", "gramme", "grammes"]),
    ("kilogram", "kilograms", 'w', &["kg", "kg.", "kgs", "kilo", "kilos", "kilogram", "kilograms", "kilogramme", "kilogrammes"]),
    ("ounce", "ounces", 'w', &["oz", "oz.", "ozs", "ounce", "ounces"]),
    ("pound", "pounds", 'w', &["lb", "lb.", "lbs", "lbs.", "pound", "pounds", "#"]),
    ("inch", "inches", 'l', &["in", "in.", "inch", "inches", "\""]),
    ("centimeter", "centimeters", 'l', &["cm", "cm.", "centimeter", "centimeters", "centimetre", "centimetres"]),
    ("millimeter", "millimeters", 'l', &["mm", "millimeter", "millimeters", "millimetre", "millimetres"]),
    ("foot", "feet", 'l', &["ft", "foot", "feet"]),
    ("fahrenheit", "fahrenheit", 't', &["°f", "f", "fahrenheit", "degrees f"]),
    ("celsius", "celsius", 't', &["°c", "celsius", "degrees c", "centigrade"]),
];

/// Countable units (singular, plural).
pub const UNITS: &[(&str, &str)] = &[
    ("clove", "cloves"), ("can", "cans"), ("tin", "tins"), ("package", "packages"), ("pkg", "pkgs"), ("packet", "packets"),
    ("pack", "packs"), ("slice", "slices"), ("stick", "sticks"), ("pinch", "pinches"), ("dash", "dashes"),
    ("head", "heads"), ("bunch", "bunches"), ("sprig", "sprigs"), ("piece", "pieces"), ("jar", "jars"), ("bottle", "bottles"),
    ("bag", "bags"), ("box", "boxes"), ("drop", "drops"), ("leaf", "leaves"), ("stalk", "stalks"), ("handful", "handfuls"),
    ("container", "containers"), ("carton", "cartons"), ("loaf", "loaves"), ("fillet", "fillets"), ("filet", "filets"),
    ("strip", "strips"), ("cube", "cubes"), ("envelope", "envelopes"), ("sheet", "sheets"), ("block", "blocks"),
    ("wedge", "wedges"), ("ear", "ears"), ("link", "links"), ("rasher", "rashers"), ("knob", "knobs"), ("scoop", "scoops"),
    ("splash", "splashes"), ("drizzle", "drizzles"), ("squeeze", "squeezes"), ("tub", "tubs"), ("pod", "pods"),
    ("serving", "servings"), ("dollop", "dollops"), ("sachet", "sachets"), ("pat", "pats"), ("spoonful", "spoonfuls"),
    ("rib", "ribs"), ("bulb", "bulbs"), ("pc", "pcs"),
];

/// Number words (Tag::Amount). Values.
pub const AMOUNT_WORDS: &[(&str, f64)] = &[
    ("a", 1.0), ("an", 1.0), ("one", 1.0), ("two", 2.0), ("three", 3.0), ("four", 4.0), ("five", 5.0), ("six", 6.0),
    ("seven", 7.0), ("eight", 8.0), ("nine", 9.0), ("ten", 10.0), ("eleven", 11.0), ("twelve", 12.0), ("fifteen", 15.0),
    ("twenty", 20.0), ("dozen", 12.0), ("half", 0.5), ("quarter", 0.25), ("third", 1.0 / 3.0),
];

pub const MODIFIERS: &[&str] = &[
    "small", "medium", "large", "extra large", "extra-large", "big", "jumbo", "heaping", "heaped", "level", "scant",
    "generous", "rounded", "good", "thick", "thin", "whole", "fresh", "frozen", "dried", "ripe", "cold", "warm",
    "hot", "lg", "med", "sm", "packed", "loosely packed", "firmly packed",
];

pub const INSTRUCTIONS: &[&str] = &[
    "chopped", "diced", "minced", "sliced", "grated", "shredded", "crushed", "melted", "softened", "beaten", "peeled",
    "seeded", "cored", "trimmed", "halved", "quartered", "cubed", "julienned", "zested", "juiced", "toasted", "cooked",
    "drained", "rinsed", "divided", "sifted", "room temperature", "at room temperature", "to taste", "optional",
    "for garnish", "for serving", "for topping", "crumbled", "mashed", "torn", "pitted", "deveined", "thawed",
];

pub const ADVERBS: &[&str] = &[
    "finely", "freshly", "coarsely", "roughly", "thinly", "thickly", "lightly", "firmly", "loosely", "very", "well",
];

/// Store sections (the aisles of a typical grocery store), with our own keyword lists.
/// Longest matching keyword wins.
pub const AISLES: &[(&str, &[&str])] = &[
    ("Produce", &["bean sprouts", "eggplant", "bell peppers", "green onion", "green onions", "scallions", "bok choy", "lemongrass", "plantain", "jalapenos", "red onion", "limes","apple", "avocado", "banana", "basil", "berries", "berry", "blueberries", "broccoli", "cabbage", "carrot", "carrots", "celery", "chili", "chilies", "cilantro", "cucumber", "garlic", "ginger", "kale", "leek", "lemon", "lemons", "lettuce", "lime", "mango", "mint", "mushroom", "mushrooms", "onion", "onions", "orange", "parsley", "peach", "peaches", "pear", "pepper bell", "bell pepper", "potato", "potatoes", "pumpkin", "raspberries", "rosemary", "scallion", "shallot", "spinach", "squash", "strawberries", "thyme", "tomato", "tomatoes", "cherry tomatoes", "zucchini", "herbs", "sage", "dill", "chives", "jalapeno", "corn", "grapes", "melon", "pineapple", "beet", "radish", "asparagus", "green beans", "peas", "sweet potato", "fruit"]),
    ("Meat", &["chicken thighs", "chicken drumsticks", "ground pork","beef", "ground beef", "steak", "steaks", "ribeye", "chicken", "pork", "bacon", "ham", "lamb", "turkey", "sausage", "veal", "chorizo", "prosciutto", "meatballs", "duck"]),
    ("Seafood", &["shrimp", "prawns", "salmon", "tuna", "cod", "fish", "crab", "lobster", "scallops", "mussels", "clams", "anchovies", "tilapia", "halibut"]),
    ("Dairy", &["milk", "butter", "cream", "heavy cream", "half and half", "half-and-half", "cheese", "cheddar", "mozzarella", "parmesan", "feta", "ricotta", "cream cheese", "yogurt", "sour cream", "buttermilk", "egg", "eggs", "egg yolks", "egg whites", "whipped cream", "ghee"]),
    ("Breads and Cereals", &["corn tortillas", "flour tortillas", "pita","bread", "baguette", "bun", "buns", "rolls", "tortilla", "tortillas", "pita", "bagel", "oats", "rolled oats", "oatmeal", "cereal", "granola", "croutons", "breadcrumbs", "bread crumbs", "english muffin"]),
    ("Bakery", &["pie crust", "croissant", "cake", "muffins", "pizza dough", "puff pastry", "brioche"]),
    ("Baking Goods", &["flour", "all-purpose flour", "sugar", "brown sugar", "powdered sugar", "demerara sugar", "baking powder", "baking soda", "yeast", "cornstarch", "cocoa", "chocolate", "chocolate chips", "vanilla", "vanilla extract", "honey", "maple syrup", "molasses", "sweetener", "liquid sweetener", "shortening", "gelatin", "sprinkles", "almonds", "walnuts", "pecans", "nuts", "coconut", "raisins"]),
    ("Spices and Seasonings", &["garam masala", "curry powder", "five spice", "za'atar", "sumac", "saffron", "star anise", "cinnamon stick", "berbere", "ras el hanout", "smoked paprika", "sesame seeds","salt", "pepper", "black pepper", "cinnamon", "nutmeg", "paprika", "cumin", "oregano", "dried oregano", "chili flakes", "red pepper flakes", "chili powder", "curry", "turmeric", "bay leaf", "bay leaves", "cloves ground", "allspice", "cayenne", "garlic powder", "onion powder", "seasoning", "italian seasoning", "coriander"]),
    ("Oils and Dressings", &["olive oil", "oil", "vegetable oil", "canola oil", "sesame oil", "coconut oil", "vinegar", "balsamic", "vinaigrette", "dressing", "cooking spray"]),
    ("Sauces and Condiments", &["enchilada sauce","ketchup", "mustard", "mayonnaise", "mayo", "soy sauce", "hot sauce", "sriracha", "worcestershire", "salsa", "pesto", "bbq sauce", "barbecue sauce", "teriyaki", "fish sauce", "tomato sauce", "tomato paste", "jam", "peanut butter"]),
    ("Canned and Jar Goods", &["broth", "stock", "beef broth", "chicken broth", "vegetable broth", "soup", "minestrone soup", "canned", "crushed tomatoes", "whole tomatoes", "diced tomatoes", "beans", "black beans", "chickpeas", "kidney beans", "olives", "kalamata olives", "coconut milk", "tuna can", "pumpkin puree"]),
    ("Pasta, Rice and Beans", &["rice noodles", "jasmine rice", "basmati rice", "short-grain rice", "arborio rice", "udon", "soba", "dried chickpeas","pasta", "spaghetti", "penne", "macaroni", "noodles", "lasagna", "rice", "quinoa", "couscous", "lentils", "fettuccine", "linguine"]),
    ("Frozen Foods", &["frozen", "ice cream", "ice", "frozen peas", "phyllo", "filo", "popsicles"]),
    ("Beverages", &["water", "juice", "orange juice", "coffee", "tea", "soda", "sparkling water", "lemonade"]),
    ("Beer, Wine and Spirits", &["wine", "red wine", "white wine", "beer", "vodka", "rum", "whiskey", "bourbon", "brandy", "tequila", "sherry", "liqueur"]),
    ("Snacks", &["chips", "crackers", "popcorn", "pretzels", "cookies"]),
    ("Deli", &["salami", "pastrami", "hummus", "deli"]),
    ("International Cuisine", &["tofu", "silken tofu", "firm tofu", "miso", "white miso", "mirin", "green curry paste", "chili bean paste", "dashi", "wakame", "gochujang", "fish sauce", "coconut milk","tahini", "miso", "curry paste", "rice paper", "nori", "gochujang", "harissa", "coconut cream"]),
    ("Home and Garden", &["foil", "parchment", "plastic wrap", "toothpicks", "skewers"]),
];

pub const DEFAULT_AISLE: &str = "Miscellaneous";

/// Volume conversion ratios per 1 liter, and weight ratios per 1 gram (standard kitchen measures).
/// A deciliter is 10 per liter.
pub const CONVERSIONS: &[(&str, char, f64)] = &[
    ("teaspoon", 'v', 202.88414), ("tablespoon", 'v', 67.628045), ("fluid ounce", 'v', 33.814023),
    ("cup", 'v', 4.2267528), ("pint", 'v', 2.1133764), ("quart", 'v', 1.0566882), ("gallon", 'v', 0.26417205),
    ("milliliter", 'v', 1000.0), ("centiliter", 'v', 100.0), ("deciliter", 'v', 10.0), ("liter", 'v', 1.0),
    ("milligram", 'w', 1000.0), ("gram", 'w', 1.0), ("kilogram", 'w', 0.001), ("ounce", 'w', 0.035273962),
    ("pound", 'w', 0.0022046228),
];

/// When a quantity reaches `quantity` of `from`, show it in `to` (the point where a bigger unit reads better).
pub const THRESHOLDS: &[(&str, f64, &str)] = &[
    ("ounce", 16.0, "pound"), ("milligram", 1000.0, "gram"), ("gram", 1000.0, "kilogram"),
    ("teaspoon", 3.0, "tablespoon"), ("tablespoon", 16.0, "cup"), ("pint", 2.0, "quart"), ("quart", 4.0, "gallon"),
    ("milliliter", 1000.0, "liter"),
];

/// Short display names ("tbsp", "oz", ...).
pub fn short_unit(canon: &str) -> &str {
    match canon {
        "liter" => "l", "tablespoon" => "tbsp", "milliliter" => "ml", "millimeter" => "mm", "centiliter" => "cl",
        "pound" => "lb", "gallon" => "gal", "centimeter" => "cm", "quart" => "qt", "cup" => "cup", "kilogram" => "kg",
        "ounce" => "oz", "gram" => "g", "inch" => "in", "pint" => "pt", "deciliter" => "dl", "teaspoon" => "tsp",
        "fluid ounce" => "fl oz", "milligram" => "mg",
        other => other,
    }
}

// ---------------- helpers ----------------

pub fn measurement_kind(canon: &str) -> Option<char> {
    MEASUREMENTS.iter().find(|m| m.0 == canon).map(|m| m.2)
}

pub fn plural_of(canon: &str) -> String {
    if let Some(m) = MEASUREMENTS.iter().find(|m| m.0 == canon) {
        return m.1.to_string();
    }
    if let Some(u) = UNITS.iter().find(|u| u.0 == canon) {
        return u.1.to_string();
    }
    canon.to_string()
}

/// Replace unicode vulgar fractions, odd dashes and spacing.
pub fn normalize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let fr = |c: char| -> Option<&'static str> {
        Some(match c {
            '½' => "1/2", '⅓' => "1/3", '⅔' => "2/3", '¼' => "1/4", '¾' => "3/4", '⅕' => "1/5", '⅖' => "2/5",
            '⅗' => "3/5", '⅘' => "4/5", '⅙' => "1/6", '⅚' => "5/6", '⅛' => "1/8", '⅜' => "3/8", '⅝' => "5/8",
            '⅞' => "7/8",
            _ => return None,
        })
    };
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if let Some(f) = fr(c) {
            if i > 0 && chars[i - 1].is_ascii_digit() {
                out.push(' ');
            }
            out.push_str(f);
        } else if c == '⁄' {
            out.push('/');
        } else if c == '–' || c == '—' || c == '‒' {
            out.push('-');
        } else if c == '\u{a0}' || c == '\t' {
            out.push(' ');
        } else if c == '’' || c == '‘' {
            out.push('\'');
        } else if c == '“' || c == '”' {
            out.push('"');
        } else if c == ',' && i > 0 && chars[i - 1].is_ascii_digit() && chars.get(i + 1).map(|d| d.is_ascii_digit()).unwrap_or(false) {
            // Comma between digits (locale-aware): "1,5 kg" = 1.5 kg,
            // "1,000 g" = 1000 g (exactly three digits after the comma = thousands).
            let digits_after = chars[i + 1..].iter().take_while(|d| d.is_ascii_digit()).count();
            if digits_after != 3 {
                out.push('.');
            }
        } else {
            out.push(c);
        }
    }
    let collapsed: Vec<&str> = out.split(' ').filter(|w| !w.is_empty()).collect();
    collapsed.join(" ")
}

static DECIMAL_SEPARATOR: std::sync::OnceLock<char> = std::sync::OnceLock::new();

/// The system's decimal separator ('.' or ','), read once at start-up (Windows: Region settings).
pub fn init_decimal_separator() {
    let sep = detect_decimal_separator();
    let _ = DECIMAL_SEPARATOR.set(sep);
}

fn decimal_separator() -> char {
    *DECIMAL_SEPARATOR.get().unwrap_or(&'.')
}

fn detect_decimal_separator() -> char {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        if let Ok(out) = std::process::Command::new("reg")
            .args(["query", r"HKCU\Control Panel\International", "/v", "sDecimal"])
            .creation_flags(0x0800_0000)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(v) = text.lines().find(|l| l.contains("sDecimal")).and_then(|l| l.split_whitespace().last()) {
                if v == "," {
                    return ',';
                }
                if v == "." {
                    return '.';
                }
            }
        }
    }
    let lang = std::env::var("LC_NUMERIC").or_else(|_| std::env::var("LANG")).unwrap_or_default().to_lowercase();
    let comma = ["de", "fr", "es", "it", "nl", "pt", "ru", "pl", "sv", "da", "nb", "fi", "cs", "tr", "id", "vi"];
    if comma.iter().any(|c| lang.starts_with(c)) { ',' } else { '.' }
}

/// Strip list bullets, checkboxes and step numbers from the start of a line.
pub fn strip_bullet(s: &str) -> &str {
    let mut t = s.trim_start();
    loop {
        let before = t;
        for b in ["•", "▢", "☐", "□", "◦", "‣", "∙", "·", "* ", "- ", "– "] {
            if let Some(r) = t.strip_prefix(b) {
                t = r.trim_start();
            }
        }
        if t == before {
            break;
        }
    }
    t
}

fn parse_number(tok: &str) -> Option<f64> {
    let t = tok.trim_end_matches(['.', ',']);
    if t.is_empty() {
        return None;
    }
    if let Some((a, b)) = t.split_once('/') {
        let a: f64 = a.parse().ok()?;
        let b: f64 = b.parse().ok()?;
        if b == 0.0 {
            return None;
        }
        return Some(a / b);
    }
    if t.chars().all(|c| c.is_ascii_digit() || c == '.') && t.chars().any(|c| c.is_ascii_digit()) {
        return t.parse().ok();
    }
    None
}

fn is_fraction(tok: &str) -> bool {
    tok.contains('/') && parse_number(tok).is_some()
}

/// Largest denominator shown as a fraction (sixteenths).
pub const MAX_DENOMINATOR: i64 = 16;
/// Tolerance when turning a typed decimal into a fraction (1E-06).
pub const DECIMAL_TOLERANCE: f64 = 1e-6;
/// How close a simpler fraction must be before it replaces a finer one (estimated; a typed
/// "4.33 tablespoons" must stay a decimal, so this must be tighter than 1/300).
pub const APPROX_TOLERANCE: f64 = 1e-3;

/// Continued-fraction approximation of `x`: stop when the remainder is
/// within `tol * k * k`, or when the denominator would pass `max_den`. Returns (numerator, denominator).
pub fn to_fraction(x: f64, tol: f64, max_den: i64) -> (i64, i64) {
    let neg = x < 0.0;
    let x = x.abs();
    let (mut h0, mut h1) = (1i64, x.floor() as i64);
    let (mut k0, mut k1) = (0i64, 1i64);
    let mut r = x;
    let mut a = r.floor();
    for _ in 0..40 {
        if (r - a) <= tol * (k1 * k1) as f64 || (r - a) < 1e-12 {
            break;
        }
        r = 1.0 / (r - a);
        a = r.floor();
        let ai = a as i64;
        let (h2, k2) = (ai.saturating_mul(h1).saturating_add(h0), ai.saturating_mul(k1).saturating_add(k0));
        if k2 > max_den {
            break;
        }
        h0 = h1;
        h1 = h2;
        k0 = k1;
        k1 = k2;
    }
    let g = gcd(h1, k1).max(1);
    (if neg { -h1 / g } else { h1 / g }, k1 / g)
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 { a.abs() } else { gcd(b, a % b) }
}

/// Fraction text: "n/d" below 1, "w" when whole, "w n/d" otherwise.
fn fraction_text(n: i64, d: i64) -> String {
    if n == 0 {
        return "0".into();
    }
    let whole = n / d;
    let rem = n % d;
    if whole == 0 {
        format!("{}/{}", n, d)
    } else if rem == 0 {
        format!("{}", whole)
    } else {
        format!("{} {}/{}", whole, rem.abs(), d)
    }
}

/// Decimal text: 0 places from 100, 1 place from 10, otherwise 2 places.
pub fn format_decimal(q: f64) -> String {
    let s = if q >= 100.0 { format!("{:.0}", q) } else if q >= 10.0 { format!("{:.1}", q) } else { format!("{:.2}", q) };
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    // shown with the system's decimal separator (it follows the device locale)
    let sep = decimal_separator();
    if sep == '.' { s } else { s.replace('.', &sep.to_string()) }
}

/// Amount as shown for US and count units (denominator limit 16): the exact
/// fraction if its denominator is at most 16, else a close simpler fraction, else a decimal.
pub fn format_qty(q: f64) -> String {
    if q <= 0.0 {
        return "0".into();
    }
    let (n, d) = to_fraction(q, DECIMAL_TOLERANCE, i64::MAX / 4);
    if d <= MAX_DENOMINATOR {
        return fraction_text(n, d);
    }
    let (n2, d2) = to_fraction(q, 0.0, MAX_DENOMINATOR);
    if d2 <= MAX_DENOMINATOR && (n2 as f64 / d2 as f64 - q).abs() <= APPROX_TOLERANCE {
        return fraction_text(n2, d2);
    }
    format_decimal(q)
}

/// Amount for a given unit: metric measurements as decimals, others as fractions.
pub fn format_amount(q: f64, unit: Option<&str>) -> String {
    let metric = matches!(
        unit,
        Some("gram" | "kilogram" | "milligram" | "milliliter" | "centiliter" | "deciliter" | "liter" | "centimeter" | "millimeter")
    );
    if metric { format_decimal(q) } else { format_qty(q) }
}

// ---------------- ingredient parsing ----------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedIngredient {
    pub raw: String,
    pub quantity: Option<f64>,
    pub quantity_max: Option<f64>,
    pub unit: Option<String>,
    pub name: String,
    pub prep: String,
    pub size_note: String,
    pub is_heading: bool,
    pub confidence: f32,
    pub issues: Vec<String>,
    pub tags: Vec<(String, TagName)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TagName(pub Tag);

impl ParsedIngredient {
    pub fn flagged(&self) -> bool {
        self.confidence < REVIEW_THRESHOLD
    }
    pub fn qty_text(&self) -> String {
        match (self.quantity, self.quantity_max) {
            (Some(a), Some(b)) => format!("{}-{}", format_amount(a, self.unit.as_deref()), format_amount(b, self.unit.as_deref())),
            (Some(a), None) => format_amount(a, self.unit.as_deref()),
            _ => String::new(),
        }
    }
    pub fn unit_text(&self) -> String {
        match &self.unit {
            Some(u) => {
                let plural = self.quantity.map(|q| q > 1.0).unwrap_or(false) || self.quantity_max.is_some();
                if plural { plural_of(u) } else { u.clone() }
            }
            None => String::new(),
        }
    }
    /// Re-assemble in tidy form: "1 1/2 cups flour, sifted".
    pub fn tidy(&self) -> String {
        if self.is_heading {
            let h = self.raw.trim().trim_end_matches(':').trim();
            return format!("{}:", h);
        }
        let mut parts = vec![];
        let q = self.qty_text();
        if !q.is_empty() {
            parts.push(q);
        }
        let u = self.unit_text();
        if !u.is_empty() {
            parts.push(u);
        }
        if !self.size_note.is_empty() {
            parts.push(self.size_note.clone());
        }
        if !self.name.is_empty() {
            parts.push(self.name.clone());
        }
        let mut s = parts.join(" ");
        if !self.prep.is_empty() {
            if s.is_empty() {
                s = self.prep.clone();
            } else {
                s.push_str(", ");
                s.push_str(&self.prep);
            }
        }
        s
    }
}

fn match_measurement(words: &[String], i: usize) -> Option<(String, usize)> {
    // try 2-word then 1-word
    for len in [3usize, 2, 1] {
        if i + len > words.len() {
            continue;
        }
        let cand = words[i..i + len].join(" ");
        // Case-sensitive specials: "T" = tablespoon, "t" = teaspoon.
        if len == 1 && cand == "T" {
            return Some(("tablespoon".into(), 1));
        }
        if len == 1 && cand == "t" {
            return Some(("teaspoon".into(), 1));
        }
        let lc = cand.to_lowercase();
        for m in MEASUREMENTS {
            if m.2 == 't' {
                continue;
            }
            if m.3.iter().any(|v| *v == lc) || m.0 == lc || m.1 == lc {
                return Some((m.0.to_string(), len));
            }
        }
    }
    None
}

fn match_unit(word: &str) -> Option<String> {
    let lc = word.to_lowercase();
    let lc = lc.trim_end_matches('.');
    for u in UNITS {
        if u.0 == lc || u.1 == lc {
            return Some(u.0.to_string());
        }
    }
    None
}

fn contains_phrase(hay: &str, phrase: &str) -> bool {
    // whole-word containment
    let h = format!(" {} ", hay);
    h.contains(&format!(" {} ", phrase))
}

pub fn aisle_for(name: &str) -> &'static str {
    let n: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' { c } else { ' ' })
        .collect();
    let n = n.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut best: Option<(&'static str, usize)> = None;
    for (aisle, words) in AISLES {
        for w in *words {
            if contains_phrase(&n, w) {
                let l = w.len();
                if best.map(|b| l > b.1).unwrap_or(true) {
                    best = Some((aisle, l));
                }
            }
        }
    }
    best.map(|b| b.0).unwrap_or(DEFAULT_AISLE)
}

pub fn is_known_ingredient(name: &str) -> bool {
    aisle_for(name) != DEFAULT_AISLE || crate::nutrition::lookup(name).is_some()
}

fn looks_like_heading(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    let lc = t.to_lowercase();
    if t.ends_with(':') && !t.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    if (lc.starts_with("for the ") || lc.starts_with("for ")) && t.len() < 40 && !t.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 4 && letters.iter().all(|c| c.is_uppercase()) && !t.chars().any(|c| c.is_ascii_digit())
}

const WEB_JUNK: &[&str] = &["&amp;", "&nbsp;", "&#", "<span", "</", "<br", "\u{fffd}", "{{", "}}"];
const PAGE_CLUTTER: &[&str] = &[
    "advertisement", "jump to recipe", "print recipe", "pin recipe", "rate this", "subscribe", "click here",
    "affiliate", "sponsored", "watch now", "save recipe", "cook mode", "prevent your screen",
];

pub fn parse_ingredient(line: &str) -> ParsedIngredient {
    let raw = line.trim().to_string();
    let mut p = ParsedIngredient { raw: raw.clone(), confidence: 1.0, ..Default::default() };
    let mut issues: Vec<(f32, String)> = vec![];

    let lc_raw = raw.to_lowercase();
    if WEB_JUNK.iter().any(|j| lc_raw.contains(j)) {
        issues.push((0.5, "Contains leftover web formatting".into()));
    }
    if PAGE_CLUTTER.iter().any(|j| lc_raw.contains(j)) {
        issues.push((0.6, "Looks like page clutter, not an ingredient".into()));
    }

    let text = normalize_text(strip_bullet(&raw));
    if looks_like_heading(&text) {
        p.is_heading = true;
        p.name = text.trim_end_matches(':').to_string();
        p.confidence = if issues.is_empty() { 1.0 } else { 0.5 };
        p.issues = issues.into_iter().map(|i| i.1).collect();
        return p;
    }

    // Pull out parenthetical notes "(14 oz)" / "(about 2 cups)".
    let mut size_note = String::new();
    let mut body = String::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                if depth == 1 {
                    cur.clear();
                    continue;
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    if !size_note.is_empty() {
                        size_note.push(' ');
                    }
                    size_note.push_str(&format!("({})", cur.trim()));
                    continue;
                }
            }
            _ => {}
        }
        if depth > 0 {
            cur.push(c);
        } else {
            body.push(c);
        }
    }
    if depth != 0 {
        issues.push((0.2, "Unbalanced brackets".into()));
        body = text.clone();
        size_note.clear();
    }

    // Split on first comma into main / prep.
    let (main, mut prep) = match body.split_once(',') {
        Some((a, b)) => (a.trim().to_string(), b.trim().to_string()),
        None => (body.trim().to_string(), String::new()),
    };

    // Tokenize main part; separate a glued unit "200g" / "2tbsp" / "1-2".
    let mut words: Vec<String> = vec![];
    for w in main.split_whitespace() {
        let digits: String = w.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '/').collect();
        if !digits.is_empty() && digits.len() < w.len() {
            let rest = &w[digits.len()..];
            if let Some(r) = rest.strip_prefix('-') {
                if r.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    words.push(digits);
                    words.push("-".into());
                    words.push(r.to_string());
                    continue;
                }
            }
            if rest.chars().all(|c| c.is_alphabetic() || c == '.') {
                words.push(digits);
                words.push(rest.to_string());
                continue;
            }
        }
        words.push(w.to_string());
    }

    let mut i = 0;
    // Quantity
    let mut qty: Option<f64> = None;
    let mut qmax: Option<f64> = None;
    let read_qty = |words: &Vec<String>, i: &mut usize| -> Option<f64> {
        let mut total: Option<f64> = None;
        while *i < words.len() {
            let w = &words[*i];
            if let Some(v) = parse_number(w) {
                // "1 1/2": a whole number followed by a fraction adds up.
                match total {
                    None => total = Some(v),
                    Some(t) if is_fraction(w) && t.fract() == 0.0 => total = Some(t + v),
                    _ => break,
                }
                *i += 1;
                if is_fraction(w) {
                    break;
                }
            } else {
                break;
            }
        }
        if total.is_none() && *i < words.len() {
            let lc = words[*i].to_lowercase();
            if let Some((_, v)) = AMOUNT_WORDS.iter().find(|(w, _)| *w == lc) {
                // "a" / "an" only count as amounts when followed by a unit/measurement
                let next_is_unit = *i + 1 < words.len()
                    && (match_measurement(words, *i + 1).is_some() || match_unit(&words[*i + 1]).is_some());
                if (lc != "a" && lc != "an") || next_is_unit {
                    *i += 1;
                    return Some(*v);
                }
            }
        }
        total
    };
    if let Some(v) = read_qty(&words, &mut i) {
        p.tags.push((format_qty(v), TagName(Tag::Number)));
        qty = Some(v);
        // range: "1-2", "1 to 2", "1 or 2"
        if i < words.len() && (words[i] == "-" || words[i].eq_ignore_ascii_case("to") || words[i].eq_ignore_ascii_case("or")) {
            let mut j = i + 1;
            if let Some(v2) = read_qty(&words, &mut j) {
                if v2 > v {
                    qmax = Some(v2);
                    i = j;
                }
            }
        } else if i < words.len() && words[i].starts_with('-') {
            if let Some(v2) = parse_number(&words[i][1..]) {
                qmax = Some(v2);
                i += 1;
            }
        }
    }
    // "2 x 400g tins": two quantities in a row are multiplied.
    if qty.is_some() && i < words.len() && (words[i].eq_ignore_ascii_case("x") || words[i] == "×") {
        let mut j = i + 1;
        if let Some(v2) = read_qty(&words, &mut j) {
            if let Some((u, n)) = match_measurement(&words, j) {
                size_note = format!("({} × {} {})", format_qty(qty.unwrap()), format_amount(v2, Some(&u)), short_unit(&u));
                qty = Some(qty.unwrap() * v2);
                qmax = qmax.map(|m| m * v2);
                p.unit = Some(u);
                i = j + n;
            }
        }
    }

    // Size modifiers before unit ("1 large can", "2 heaping tbsp")
    let mut mods: Vec<String> = vec![];
    while i < words.len() && MODIFIERS.contains(&words[i].to_lowercase().as_str()) {
        let next_unit = i + 1 < words.len() && (match_measurement(&words, i + 1).is_some() || match_unit(&words[i + 1]).is_some());
        if !next_unit {
            break;
        }
        mods.push(words[i].clone());
        i += 1;
    }

    // Unit (unless the "×" rule above already set it)
    if i < words.len() && p.unit.is_none() {
        if let Some((u, n)) = match_measurement(&words, i) {
            // A lone "c"/"l"/"g" without a quantity is probably part of a name.
            if qty.is_some() || n > 1 || words[i].len() > 2 {
                p.tags.push((u.clone(), TagName(Tag::Measurement)));
                p.unit = Some(u);
                i += n;
            }
        } else if let Some(u) = match_unit(&words[i]) {
            p.tags.push((u.clone(), TagName(Tag::Unit)));
            p.unit = Some(u);
            i += 1;
        } else if qty.is_some() {
            let w = words[i].to_lowercase();
            let w = w.trim_end_matches('.');
            if words[i].ends_with('.') && w.len() <= 7 && w.chars().all(|c| c.is_alphabetic()) {
                issues.push((0.4, format!("Unrecognised unit \"{}\"", words[i])));
            }
        }
    }
    if i < words.len() && words[i].eq_ignore_ascii_case("of") {
        i += 1;
    }
    if !mods.is_empty() {
        let m = mods.join(" ");
        size_note = if size_note.is_empty() { m } else { format!("{} {}", m, size_note) };
    }

    // Remaining words: move leading adverbs/instructions into prep ("finely chopped onion").
    let mut rest: Vec<String> = words[i..].to_vec();
    let mut lead_prep: Vec<String> = vec![];
    // Packaged goods keep their product name ("1 can diced tomatoes").
    let packaged = matches!(
        p.unit.as_deref(),
        Some("can" | "tin" | "jar" | "package" | "pkg" | "packet" | "pack" | "bag" | "box" | "bottle" | "carton" | "container")
    );
    while !rest.is_empty() && !packaged {
        let lc = rest[0].to_lowercase();
        if ADVERBS.contains(&lc.as_str()) && rest.len() > 2 {
            lead_prep.push(rest.remove(0));
        } else if INSTRUCTIONS.contains(&lc.as_str()) && rest.len() > 1 {
            lead_prep.push(rest.remove(0));
        } else {
            break;
        }
    }
    if !lead_prep.is_empty() {
        let lp = lead_prep.join(" ");
        prep = if prep.is_empty() { lp } else { format!("{}, {}", lp, prep) };
    }
    let name = rest.join(" ");

    // ----- confidence checks -----
    if rest.iter().any(|w| parse_number(w).is_some() && !w.contains('%')) && qty.is_none() {
        issues.push((0.3, "A quantity appears in an unexpected place".into()));
    } else if rest.iter().skip(1).any(|w| parse_number(w).is_some() && match_measurement(&words, 0).is_none() && w.len() < 4) {
        issues.push((0.15, "Extra number inside the name".into()));
    }
    let name_lc = name.to_lowercase();
    if name.trim().is_empty() && prep.is_empty() {
        issues.push((0.6, "No ingredient name found".into()));
    } else if !name.trim().is_empty() && !is_known_ingredient(&name) && !is_known_ingredient(&prep) {
        issues.push((0.15, "Ingredient not in the food list (check spelling)".into()));
    }
    if raw.len() > 110 || (raw.matches(". ").count() >= 1 && raw.len() > 70) {
        issues.push((0.45, "Looks like a sentence or step, not an ingredient".into()));
    }
    if let Some(q) = qty {
        let big = match p.unit.as_deref().and_then(measurement_kind) {
            Some('w') | Some('v') => {
                let u = p.unit.as_deref().unwrap();
                !(u == "gram" || u == "milliliter" || u == "milligram") && q > 50.0
            }
            _ => q > 200.0,
        };
        if q == 0.0 || big {
            issues.push((0.3, "Quantity looks unusual".into()));
        }
    }
    if qty.is_none() && p.unit.is_some() {
        issues.push((0.1, "Unit without a quantity".into()));
    }
    if name_lc.starts_with("and ") || name_lc.starts_with("or ") {
        issues.push((0.25, "Starts with a joining word; may be split from the previous line".into()));
    }

    p.quantity = qty;
    p.quantity_max = qmax;
    p.name = name;
    p.prep = prep;
    p.size_note = size_note;
    let penalty: f32 = issues.iter().map(|i| i.0).sum();
    p.confidence = (1.0 - penalty).clamp(0.0, 1.0);
    p.issues = issues.into_iter().map(|i| i.1).collect();
    if !p.name.is_empty() {
        p.tags.push((p.name.clone(), TagName(Tag::Ingredient)));
    }
    p
}

// ---------------- scale & convert ----------------

/// Scale typed as a fraction ("1 1/2", "3/4", "2"); anything unreadable means 1
/// (the default is 1/1).
/// One ingredient line as shown in a recipe: (amount column, rest of the line, is a section heading),
/// after scaling and optional metric or US conversion.
pub fn display_row(line: &str, scale: f64, metric: bool, us_units: bool) -> (Option<String>, String, bool) {
    let p = parse_ingredient(line);
    let text_out: String;
    let mut amount: Option<String> = None;
    if p.is_heading {
        text_out = format!("{}:", p.name);
    } else if p.quantity.is_some() && (scale - 1.0).abs() < 1e-3 && !metric && !(us_units && p.unit.as_deref().and_then(|u| to_us(1.0, u)).is_some()) {
        // original text with the amount in bold
        let q = p.qty_text();
        let text = normalize_text(strip_bullet(line));
        let first_word_len = text.split(' ').take(q.split(' ').count()).map(|s| s.len() + 1).sum::<usize>().min(text.len());
        let (a, b) = text.split_at(first_word_len.saturating_sub(1).min(text.len()));
        amount = Some(a.trim().to_string());
        text_out = b.trim_start().to_string();
    } else if let Some(q) = p.quantity {
        let (mut qty, mut unit) = (q * scale, p.unit.clone());
        if metric {
            if let Some(u) = &p.unit {
                match measurement_kind(u) {
                    Some('v') if !u.ends_with("liter") => {
                        let ml = convert(qty, u, "milliliter").unwrap_or(qty);
                        let (q2, u2) = roll_up(ml.round(), "milliliter".into());
                        qty = q2;
                        unit = Some(u2);
                    }
                    Some('w') if u == "ounce" || u == "pound" => {
                        let g = convert(qty, u, "gram").unwrap_or(qty);
                        let (q2, u2) = roll_up(g.round(), "gram".into());
                        qty = q2;
                        unit = Some(u2);
                    }
                    _ => {}
                }
            }
        } else if let Some((q2, u2)) = unit.as_deref().filter(|_| us_units).and_then(|u| to_us(qty, u)) {
            // Convert to US units, rolled up by the usual thresholds.
            qty = q2;
            unit = Some(u2);
        } else if let Some(u) = &unit {
            // Scaling re-applies the unit roll-up.
            if measurement_kind(u).is_some() {
                let (q2, u2) = roll_up(qty, u.clone());
                qty = q2;
                unit = Some(u2);
            }
        }
        let utxt = match &unit {
            Some(u) if measurement_kind(u).is_some() => {
                let s = short_unit(u);
                // whole-word short names ("cup") still take a plural
                if s == u && qty > 1.0 { plural_of(u) } else { s.to_string() }
            }
            Some(u) => if qty > 1.0 { plural_of(u) } else { u.clone() },
            None => String::new(),
        };
        amount = Some(format_amount(qty, unit.as_deref()));
        let mut rest = String::new();
        if !utxt.is_empty() {
            rest.push(' ');
            rest.push_str(&utxt);
        }
        for part in [&p.size_note, &p.name] {
            if !part.is_empty() {
                rest.push(' ');
                rest.push_str(part);
            }
        }
        if !p.prep.is_empty() {
            rest.push_str(", ");
            rest.push_str(&p.prep);
        }
        text_out = rest.trim_start().to_string();
    } else {
        text_out = line.to_string();
    }
    (amount, text_out, p.is_heading)
}

pub fn parse_scale(text: &str) -> f64 {
    let t = text.trim().trim_start_matches(['x', '×']).trim_end_matches(['x', '×']).trim();
    parse_ingredient(&format!("{} x", t)).quantity.filter(|v| *v > 0.0 && v.is_finite()).unwrap_or(1.0)
}

/// Metric amount shown in US units, using the usual roll-up thresholds
/// (volume via teaspoons → tbsp → cup …, weight via ounces → pound).
pub fn to_us(q: f64, unit: &str) -> Option<(f64, String)> {
    match measurement_kind(unit)? {
        'v' if matches!(unit, "milliliter" | "centiliter" | "deciliter" | "liter") => {
            Some(roll_up(convert(q, unit, "teaspoon")?, "teaspoon".into()))
        }
        'w' if matches!(unit, "milligram" | "gram" | "kilogram") => Some(roll_up(convert(q, unit, "ounce")?, "ounce".into())),
        _ => None,
    }
}

// ---------------- trying out different amounts ----------------

/// Step for the − / + buttons when trying out an amount: sensible for the unit and size.
pub fn adjust_step(unit: Option<&str>, q: f64) -> f64 {
    match unit {
        Some("gram" | "milliliter") => if q >= 200.0 { 25.0 } else { 10.0 },
        Some("milligram") => 100.0,
        Some("kilogram" | "liter" | "pound" | "quart" | "gallon") => 0.25,
        Some("ounce" | "fluid ounce") => 1.0,
        Some("cup" | "pint") => 0.25,
        Some("tablespoon" | "teaspoon") => 0.5,
        Some("deciliter" | "centiliter") => 1.0,
        _ => if q < 1.0 { 0.25 } else { 1.0 },
    }
}

/// Replace the amount at the start of an ingredient line, keeping the rest of the wording:
/// "2 cups half-and-half" → "2 1/2 cups half-and-half"; "200g butter" → "250g butter".
/// A range ("2-3 cloves") becomes a single amount. Units keep their plural form in step.
pub fn replace_amount(line: &str, new_q: f64) -> String {
    let p = parse_ingredient(line);
    let Some(old_q) = p.quantity else { return line.to_string() };
    let text = normalize_text(strip_bullet(line));
    let words: Vec<&str> = text.split(' ').collect();
    // how many leading words make up the amount ("1 1/2", "2-3", "2 to 3", "two")
    let mut n = 0;
    let mut glued_rest = String::new();
    while n < words.len() {
        let w = words[n];
        let digits: String = w.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '/').collect();
        if !digits.is_empty() && digits.len() == w.len() {
            n += 1;
        } else if !digits.is_empty() && n == 0 {
            // "200g" or "1-2"
            let rest = &w[digits.len()..];
            if let Some(r) = rest.strip_prefix('-') {
                if r.chars().all(|c| c.is_ascii_digit() || c == '/' || c == '.') {
                    n += 1;
                    continue;
                }
            }
            glued_rest = rest.to_string();
            n += 1;
            break;
        } else if (w == "-" || w.eq_ignore_ascii_case("to") || w.eq_ignore_ascii_case("or")) && n > 0 && p.quantity_max.is_some() {
            n += 1;
        } else if n == 0 && AMOUNT_WORDS.iter().any(|(a, _)| a.eq_ignore_ascii_case(w)) {
            n += 1;
            break;
        } else {
            break;
        }
    }
    if n == 0 {
        return line.to_string();
    }
    let amount = format_amount(new_q, p.unit.as_deref());
    let mut rest: Vec<String> = words[n..].iter().map(|s| s.to_string()).collect();
    // keep the unit word's singular/plural in step with the new amount
    if let (Some(u), Some(first)) = (&p.unit, rest.first_mut()) {
        let plural = plural_of(u);
        let lc = first.to_lowercase();
        if (lc == *u || lc == plural) && new_q > 1.0 {
            *first = plural;
        } else if (lc == *u || lc == plural) && new_q <= 1.0 && old_q > 1.0 {
            *first = u.clone();
        }
    }
    let head = if glued_rest.is_empty() { amount } else { format!("{}{}", amount, glued_rest) };
    if rest.is_empty() { head } else { format!("{} {}", head, rest.join(" ")) }
}

// ---------------- directions ----------------

/// Split directions text into steps: one per paragraph, or by "1." / "Step 1" markers inside a block.
pub fn split_steps(text: &str) -> Vec<String> {
    let mut out = vec![];
    for para in text.split('\n') {
        let t = para.trim();
        if t.is_empty() {
            continue;
        }
        let pieces = split_numbered(t);
        for piece in pieces {
            let s = strip_step_number(&piece).trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
        }
    }
    out
}

fn split_numbered(t: &str) -> Vec<String> {
    // Find " 2. " style markers where the number increases by one.
    let bytes: Vec<char> = t.chars().collect();
    let mut cuts = vec![0usize];
    let mut expect = 2u32;
    let mut i = 1;
    while i + 2 < bytes.len() {
        if bytes[i - 1] == ' ' && bytes[i].is_ascii_digit() {
            let mut j = i;
            let mut n = 0u32;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                n = n * 10 + bytes[j].to_digit(10).unwrap();
                j += 1;
            }
            if n == expect && j + 1 < bytes.len() && (bytes[j] == '.' || bytes[j] == ')') && bytes[j + 1] == ' ' {
                cuts.push(i);
                expect += 1;
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    if cuts.len() < 2 {
        return vec![t.to_string()];
    }
    cuts.push(bytes.len());
    cuts.windows(2).map(|w| bytes[w[0]..w[1]].iter().collect::<String>()).collect()
}

pub fn strip_step_number(s: &str) -> String {
    let t = strip_bullet(s).trim();
    let lc = t.to_lowercase();
    let t2 = if lc.starts_with("step ") { &t[5..] } else { t };
    let digits: String = t2.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() && digits.len() <= 2 {
        let rest = &t2[digits.len()..];
        if let Some(r) = rest.strip_prefix('.').or(rest.strip_prefix(')')).or(rest.strip_prefix(':')) {
            return r.trim().to_string();
        }
        if lc.starts_with("step ") {
            return rest.trim().to_string();
        }
    }
    t.to_string()
}

#[derive(Clone, Debug, Default)]
pub struct StepReport {
    pub text: String,
    pub confidence: f32,
    pub issues: Vec<String>,
}

impl StepReport {
    pub fn flagged(&self) -> bool {
        self.confidence < REVIEW_THRESHOLD
    }
}

pub fn check_step(text: &str, prev: Option<&str>) -> StepReport {
    let mut issues: Vec<(f32, String)> = vec![];
    let t = text.trim();
    let lc = t.to_lowercase();
    if t.chars().count() < 12 {
        issues.push((0.35, "Very short; may be a fragment or heading".into()));
    }
    if t.chars().count() > 700 {
        issues.push((0.3, "Very long; several steps may be merged".into()));
    }
    if WEB_JUNK.iter().any(|j| lc.contains(j)) {
        issues.push((0.5, "Contains leftover web formatting".into()));
    }
    if PAGE_CLUTTER.iter().any(|j| lc.contains(j)) {
        issues.push((0.6, "Contains page clutter (ads, buttons)".into()));
    }
    if t.chars().next().map(|c| c.is_lowercase()).unwrap_or(false) {
        issues.push((0.3, "Starts in lowercase; may continue the previous step".into()));
    }
    let as_ing = parse_ingredient(t);
    if as_ing.quantity.is_some() && as_ing.unit.is_some() && t.len() < 60 && !t.contains('.') && as_ing.confidence > 0.8 {
        issues.push((0.45, "Looks like an ingredient, not a step".into()));
    }
    if let Some(p) = prev {
        if p.trim() == t {
            issues.push((0.5, "Repeats the previous step".into()));
        }
    }
    let penalty: f32 = issues.iter().map(|i| i.0).sum();
    StepReport {
        text: t.to_string(),
        confidence: (1.0 - penalty).clamp(0.0, 1.0),
        issues: issues.into_iter().map(|i| i.1).collect(),
    }
}

// ---------------- times in directions (for timer links) ----------------

#[derive(Clone, Debug, PartialEq)]
pub struct TimeSpan {
    pub start: usize,
    pub end: usize,
    pub seconds: u32,
    pub label: String,
}

/// Find "10 minutes", "1 hour 15 minutes", "4-5 hrs", "30 seconds" in text (byte offsets).
pub fn find_times(text: &str) -> Vec<TimeSpan> {
    let mut out = vec![];
    let words: Vec<(usize, &str)> = text
        .split_inclusive(|c: char| c.is_whitespace())
        .scan(0usize, |pos, w| {
            let s = *pos;
            *pos += w.len();
            Some((s, w))
        })
        .collect();
    let unit_secs = |w: &str| -> Option<u32> {
        let w = w.trim().trim_end_matches(|c: char| !c.is_alphabetic()).to_lowercase();
        match w.as_str() {
            "second" | "seconds" | "sec" | "secs" => Some(1),
            "minute" | "minutes" | "min" | "mins" => Some(60),
            "hour" | "hours" | "hr" | "hrs" => Some(3600),
            _ => None,
        }
    };
    let num_of = |w: &str| -> Option<f64> {
        let w = w.trim();
        let w = w.split('-').next_back().unwrap_or(w);
        let w = w.split(['–']).next_back().unwrap_or(w);
        parse_number(w.trim_end_matches(|c: char| !c.is_ascii_digit()))
            .or_else(|| AMOUNT_WORDS.iter().find(|(a, _)| *a == w.to_lowercase() && a.len() > 2).map(|x| x.1))
    };
    let mut i = 0;
    while i + 1 < words.len() {
        if let (Some(n), Some(us)) = (num_of(words[i].1), unit_secs(words[i + 1].1)) {
            let start = words[i].0;
            let mut end = words[i + 1].0 + words[i + 1].1.trim_end().trim_end_matches([',', '.', ';', ')']).len();
            let mut secs = (n * us as f64) as u32;
            let mut j = i + 2;
            // "1 hour 15 minutes" / "1 hour and 15 minutes"
            if j < words.len() && words[j].1.trim().eq_ignore_ascii_case("and") {
                j += 1;
            }
            if j + 1 < words.len() {
                if let (Some(n2), Some(us2)) = (num_of(words[j].1), unit_secs(words[j + 1].1)) {
                    if us2 < us {
                        secs += (n2 * us2 as f64) as u32;
                        end = words[j + 1].0 + words[j + 1].1.trim_end().trim_end_matches([',', '.', ';', ')']).len();
                        j += 2;
                    }
                }
            }
            out.push(TimeSpan { start, end, seconds: secs, label: text[start..end].to_string() });
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

pub fn parse_duration_minutes(s: &str) -> Option<u32> {
    let spans = find_times(s);
    if !spans.is_empty() {
        return Some(spans.iter().map(|t| t.seconds).sum::<u32>() / 60);
    }
    None
}

pub fn fmt_minutes(m: u32) -> String {
    if m == 0 {
        return String::new();
    }
    if m < 60 {
        format!("{} min", m)
    } else if m % 60 == 0 {
        format!("{} hr", m / 60)
    } else {
        format!("{} hr {} min", m / 60, m % 60)
    }
}

pub fn sum_times(a: &str, b: &str) -> String {
    let t = parse_duration_minutes(a).unwrap_or(0) + parse_duration_minutes(b).unwrap_or(0);
    fmt_minutes(t)
}

/// ISO 8601 duration "PT1H15M" -> "1 hr 15 min".
pub fn iso_duration(s: &str) -> String {
    let s = s.trim();
    if !s.starts_with('P') {
        return s.to_string();
    }
    let mut mins = 0u32;
    let mut num = String::new();
    let mut in_time = false;
    for c in s.chars().skip(1) {
        match c {
            'T' => in_time = true,
            '0'..='9' | '.' => num.push(c),
            'D' => {
                mins += num.parse::<f32>().unwrap_or(0.0) as u32 * 1440;
                num.clear()
            }
            'H' => {
                mins += (num.parse::<f32>().unwrap_or(0.0) * 60.0) as u32;
                num.clear()
            }
            'M' if in_time => {
                mins += num.parse::<f32>().unwrap_or(0.0) as u32;
                num.clear()
            }
            'S' => num.clear(),
            _ => num.clear(),
        }
    }
    fmt_minutes(mins)
}

// ---------------- conversion & combining ----------------

/// Convert `q` of `from` into `to` if both are in the conversions table and of the same kind.
pub fn convert(q: f64, from: &str, to: &str) -> Option<f64> {
    let a = CONVERSIONS.iter().find(|c| c.0 == from)?;
    let b = CONVERSIONS.iter().find(|c| c.0 == to)?;
    if a.1 != b.1 {
        return None;
    }
    Some(q / a.2 * b.2)
}

/// Apply the roll-up thresholds: 48 tsp -> 1 cup.
pub fn roll_up(mut q: f64, mut unit: String) -> (f64, String) {
    loop {
        let Some(t) = THRESHOLDS.iter().find(|t| t.0 == unit) else { break };
        if q < t.1 * (1.0 - 1e-6) {
            break;
        }
        match convert(q, t.0, t.2) {
            Some(nq) => {
                q = nq;
                unit = t.2.to_string();
            }
            None => break,
        }
    }
    (q, unit)
}

/// Simple singular form for matching ("tomatoes" -> "tomato").
pub fn singular(s: &str) -> String {
    let w = s.trim().to_lowercase();
    if w.ends_with("ies") && w.len() > 4 {
        return format!("{}y", &w[..w.len() - 3]);
    }
    if w.ends_with("oes") || w.ends_with("ches") || w.ends_with("shes") || w.ends_with("sses") {
        return w[..w.len() - 2].to_string();
    }
    if w.ends_with('s') && !w.ends_with("ss") && w.len() > 3 {
        return w[..w.len() - 1].to_string();
    }
    w
}

pub fn match_key(name: &str) -> String {
    let n: String = name.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    n.split_whitespace().map(singular).collect::<Vec<_>>().join(" ")
}

/// True if a pantry item name covers the ingredient ("eggs" covers "large eggs").
pub fn pantry_covers(pantry_name: &str, ingredient_name: &str) -> bool {
    let p = match_key(pantry_name);
    let i = match_key(ingredient_name);
    if p.is_empty() || i.is_empty() {
        return false;
    }
    if p == i {
        return true;
    }
    // whole pantry phrase appears at the end of the ingredient ("all-purpose flour" ends with "flour")
    let iw: Vec<&str> = i.split(' ').collect();
    let pw: Vec<&str> = p.split(' ').collect();
    if pw.len() <= iw.len() && iw[iw.len() - pw.len()..] == pw[..] {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic() {
        let p = parse_ingredient("1 1/2 cups all-purpose flour, sifted");
        assert_eq!(p.quantity, Some(1.5));
        assert_eq!(p.unit.as_deref(), Some("cup"));
        assert_eq!(p.name, "all-purpose flour");
        assert_eq!(p.prep, "sifted");
        assert!(!p.flagged());
    }

    #[test]
    fn unicode_and_glued() {
        let p = parse_ingredient("½ tsp salt");
        assert_eq!(p.quantity, Some(0.5));
        assert_eq!(p.unit.as_deref(), Some("teaspoon"));
        let p = parse_ingredient("200g butter");
        assert_eq!(p.quantity, Some(200.0));
        assert_eq!(p.unit.as_deref(), Some("gram"));
        let p = parse_ingredient("1½ cups milk");
        assert_eq!(p.quantity, Some(1.5));
    }

    #[test]
    fn ranges_and_words() {
        let p = parse_ingredient("2-3 cloves garlic, minced");
        assert_eq!(p.quantity, Some(2.0));
        assert_eq!(p.quantity_max, Some(3.0));
        assert_eq!(p.unit.as_deref(), Some("clove"));
        let p = parse_ingredient("two large eggs");
        assert_eq!(p.quantity, Some(2.0));
        assert_eq!(p.name, "large eggs");
        let p = parse_ingredient("1 T sugar");
        assert_eq!(p.unit.as_deref(), Some("tablespoon"));
        let p = parse_ingredient("1 t sugar");
        assert_eq!(p.unit.as_deref(), Some("teaspoon"));
        let p = parse_ingredient("1 can (14 oz) diced tomatoes");
        assert_eq!(p.unit.as_deref(), Some("can"));
        assert_eq!(p.size_note, "(14 oz)");
        assert_eq!(p.name, "diced tomatoes");
    }

    #[test]
    fn flags_problems() {
        assert!(parse_ingredient("2 blorps. zibble").flagged());
        assert!(parse_ingredient("Advertisement").flagged());
        assert!(parse_ingredient("Preheat the oven to 350. Then mix the flour and sugar together in a large bowl until well combined.").flagged());
        assert!(parse_ingredient("For the sauce:").is_heading);
        assert!(!parse_ingredient("Salt and pepper to taste").flagged());
    }

    #[test]
    fn steps_and_times() {
        let s = split_steps("1. Mix it. 2. Bake it for 10 minutes. 3. Eat.");
        assert_eq!(s.len(), 3);
        assert_eq!(s[1], "Bake it for 10 minutes.");
        let t = find_times("Cook for 10 minutes, then rest 1 hour 15 minutes.");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].seconds, 600);
        assert_eq!(t[1].seconds, 4500);
        assert_eq!(iso_duration("PT1H15M"), "1 hr 15 min");
    }

    /// Tiffin's number-display rules.
    #[test]
    fn number_display_rules() {
        // fractions up to sixteenths
        assert_eq!(format_qty(0.5), "1/2");
        assert_eq!(format_qty(1.75), "1 3/4");
        assert_eq!(format_qty(0.0625), "1/16");
        assert_eq!(format_qty(13.0 / 3.0), "4 1/3");
        // a typed 4.33 is 433/100: too fine for sixteenths and not close enough to 4 1/3 → decimal
        assert_eq!(format_qty(4.33), "4.33");
        // metric amounts as decimals: 2 places below 10, 1 below 100, 0 from 100
        assert_eq!(format_amount(2.5, Some("gram")), "2.5");
        assert_eq!(format_amount(12.34, Some("gram")), "12.3");
        assert_eq!(format_amount(250.4, Some("milliliter")), "250");
        // "2 x 400g" multiplies
        let p = parse_ingredient("2 x 400g tins chopped tomatoes");
        assert_eq!(p.quantity, Some(800.0));
        assert_eq!(p.unit.as_deref(), Some("gram"));
        // continued fractions
        assert_eq!(to_fraction(0.3333333, DECIMAL_TOLERANCE, 16), (1, 3));
        assert_eq!(to_fraction(2.125, DECIMAL_TOLERANCE, 1000), (17, 8));
    }

    #[test]
    fn comma_decimals() {
        let p = parse_ingredient("1,5 kg potatoes");
        assert_eq!(p.quantity, Some(1.5));
        assert_eq!(p.unit.as_deref(), Some("kilogram"));
        let p = parse_ingredient("0,25 l milk");
        assert_eq!(p.quantity, Some(0.25));
        let p = parse_ingredient("1,000 g flour");
        assert_eq!(p.quantity, Some(1000.0));
        // a comma after a word is still a separator
        let p = parse_ingredient("2 cups flour, sifted");
        assert_eq!(p.prep, "sifted");
    }

    #[test]
    fn scale_and_us_conversion() {
        assert_eq!(parse_scale("1 1/2"), 1.5);
        assert_eq!(parse_scale("3/4"), 0.75);
        assert_eq!(parse_scale("2x"), 2.0);
        assert_eq!(parse_scale("banana"), 1.0);
        assert_eq!(parse_scale(""), 1.0);
        let (q, u) = to_us(250.0, "milliliter").unwrap();
        assert_eq!(u, "cup");
        assert!((q - 1.0567).abs() < 0.01);
        let (q, u) = to_us(500.0, "gram").unwrap();
        assert_eq!(u, "pound");
        assert!((q - 1.102).abs() < 0.01);
        assert!(to_us(2.0, "cup").is_none());
    }

    #[test]
    fn trying_new_amounts() {
        assert_eq!(replace_amount("2 cups half-and-half or heavy cream", 2.5), "2 1/2 cups half-and-half or heavy cream");
        assert_eq!(replace_amount("1 cup milk", 2.0), "2 cups milk");
        assert_eq!(replace_amount("3 cups flour, sifted", 1.0), "1 cup flour, sifted");
        assert_eq!(replace_amount("200g butter", 250.0), "250g butter");
        assert_eq!(replace_amount("2-3 cloves garlic, minced", 4.0), "4 cloves garlic, minced");
        assert_eq!(replace_amount("1 1/2 tsp salt", 0.5), "1/2 tsp salt");
        assert_eq!(replace_amount("6 egg yolks", 4.0), "4 egg yolks");
        assert_eq!(replace_amount("Pinch of salt", 3.0), "Pinch of salt");
        assert_eq!(adjust_step(Some("cup"), 2.0), 0.25);
        assert_eq!(adjust_step(Some("gram"), 100.0), 10.0);
        assert_eq!(adjust_step(None, 6.0), 1.0);
    }

    #[test]
    fn conversions() {
        let (q, u) = roll_up(48.0, "teaspoon".into());
        assert_eq!(u, "cup");
        assert!((q - 1.0).abs() < 0.01);
        assert!((convert(1.0, "liter", "deciliter").unwrap() - 10.0).abs() < 1e-9);
        assert_eq!(aisle_for("whole milk"), "Dairy");
        assert_eq!(aisle_for("olive oil"), "Oils and Dressings");
        assert_eq!(aisle_for("ground beef"), "Meat");
        assert!(pantry_covers("flour", "all-purpose flour"));
        assert!(pantry_covers("eggs", "egg"));
    }
}
