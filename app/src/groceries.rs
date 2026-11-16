//! Grocery lists from recipes and the meal plan: amounts combined, grouped by store section,
//! and anything already in stock in the pantry left out.

use crate::ingredients::{self, convert, format_qty, measurement_kind, parse_ingredient, roll_up};
use crate::model::{GroceryItem, Library, Recipe};

#[derive(Clone, Debug, Default)]
pub struct BuildReport {
    pub items: Vec<GroceryItem>,
    pub in_pantry: Vec<String>,
    pub recipes: usize,
}

struct Acc {
    key: String,
    name: String,
    note: String,
    qty: Option<f64>,
    unit: Option<String>,
    extra: Vec<String>,
    recipes: Vec<String>,
}

fn pantry_has(lib: &Library, name: &str, today: i32) -> Option<String> {
    lib.pantry
        .iter()
        .filter(|p| p.in_stock && p.expires.map(|e| e >= today).unwrap_or(true))
        .find(|p| ingredients::pantry_covers(&p.name, name))
        .map(|p| p.name.clone())
}

pub fn build(lib: &Library, recipes: &[&Recipe], skip_pantry: bool, today: i32) -> BuildReport {
    // The same recipe listed more than once (e.g. planned on several days) counts as cooking it
    // that many times.
    let mut counted: Vec<(&Recipe, u32)> = vec![];
    for r in recipes {
        match counted.iter_mut().find(|(c, _)| std::ptr::eq(*c, *r) || (c.id != 0 && c.id == r.id)) {
            Some((_, n)) => *n += 1,
            None => counted.push((r, 1)),
        }
    }
    build_times(lib, &counted, skip_pantry, today)
}

/// Like [`build`], but each recipe comes with how many times it will be cooked; every amount in
/// it is multiplied by that count ("2 eggs" cooked 5 times → 10 eggs).
pub fn build_times(lib: &Library, recipes: &[(&Recipe, u32)], skip_pantry: bool, today: i32) -> BuildReport {
    let mut accs: Vec<Acc> = vec![];
    let mut rep = BuildReport { recipes: recipes.len(), ..Default::default() };
    for &(r, times) in recipes {
        let times = times.max(1) as f64;
        for line in r.ingredient_lines() {
            let mut p = parse_ingredient(line);
            p.quantity = p.quantity.map(|q| q * times);
            p.quantity_max = p.quantity_max.map(|q| q * times);
            if p.is_heading || p.name.trim().is_empty() {
                continue;
            }
            let lc = line.to_lowercase();
            if lc.contains("to taste") && p.quantity.is_none() {
                // "salt and pepper to taste": assume a staple
                if skip_pantry {
                    continue;
                }
            }
            if skip_pantry {
                if let Some(pn) = pantry_has(lib, &p.name, today) {
                    if !rep.in_pantry.contains(&pn) {
                        rep.in_pantry.push(pn);
                    }
                    continue;
                }
            }
            let key = ingredients::match_key(&p.name);
            let kind = p.unit.as_deref().and_then(measurement_kind);
            let acc = accs.iter_mut().find(|a| {
                a.key == key
                    && match (&a.unit, &p.unit) {
                        (None, None) => true,
                        (Some(x), Some(y)) => x == y || (measurement_kind(x).is_some() && measurement_kind(x) == kind),
                        _ => false,
                    }
            });
            match acc {
                Some(a) => {
                    match (a.qty, p.quantity, &a.unit, &p.unit) {
                        (Some(q0), Some(q1), Some(u0), Some(u1)) => {
                            let add = if u0 == u1 { Some(q1) } else { convert(q1, u1, u0) };
                            match add {
                                Some(v) => a.qty = Some(q0 + v),
                                None => a.extra.push(p.qty_text() + " " + &p.unit_text()),
                            }
                        }
                        (Some(q0), Some(q1), None, None) => a.qty = Some(q0 + q1),
                        _ => {
                            let t = line.to_string();
                            if !a.extra.contains(&t) {
                                a.extra.push(t);
                            }
                        }
                    }
                    if !a.recipes.contains(&r.name) {
                        a.recipes.push(r.name.clone());
                    }
                }
                None => accs.push(Acc {
                    key,
                    name: p.name.clone(),
                    note: p.prep.clone(),
                    qty: p.quantity,
                    unit: p.unit.clone(),
                    extra: vec![],
                    recipes: vec![r.name.clone()],
                }),
            }
        }
    }
    for a in accs {
        let mut qty = amount_text(a.qty, a.unit.as_ref());
        if !a.extra.is_empty() {
            if !qty.is_empty() {
                qty.push_str(" + ");
            }
            qty.push_str(&a.extra.join(" + "));
        }
        rep.items.push(GroceryItem {
            id: 0,
            aisle: ingredients::aisle_for(&a.name).to_string(),
            name: a.name,
            note: a.note,
            quantity: qty,
            recipe: a.recipes.join(", "),
            purchased: false,
        });
    }
    rep.items.sort_by(|x, y| x.aisle.cmp(&y.aisle).then(x.name.cmp(&y.name)));
    rep
}

/// "1.5 cup" → "1 ½ cups"; measured amounts roll up to a bigger unit when they get large.
fn amount_text(qty: Option<f64>, unit: Option<&String>) -> String {
    match (qty, unit) {
        (Some(q), Some(u)) => {
            let (q, u) = if measurement_kind(u).is_some() { roll_up(q, u.clone()) } else { (q, u.clone()) };
            let unit_canon = u.clone();
            let unit = if q > 1.0 { ingredients::plural_of(&u) } else { u };
            format!("{} {}", crate::ingredients::format_amount(q, Some(&unit_canon)), unit)
        }
        (Some(q), None) => format_qty(q),
        _ => String::new(),
    }
}

/// A plain amount on a list ("2 cups", "3"), or None for anything fancier ("1-2", "a + b").
fn plain_amount(text: &str) -> Option<(f64, Option<String>)> {
    let t = text.trim();
    if t.is_empty() || t.contains('+') {
        return None;
    }
    let p = parse_ingredient(&format!("{t} x"));
    match (p.quantity, p.quantity_max) {
        (Some(q), None) if p.name == "x" => Some((q, p.unit)),
        _ => None,
    }
}

/// Put an item on a list. If the same thing is already there and not bought yet, the amounts
/// are added together (or joined with "+" when they can't be) instead of listing it twice.
/// Returns true when it was combined with an existing item.
pub fn add_to_list(items: &mut Vec<GroceryItem>, new: GroceryItem) -> bool {
    let key = ingredients::match_key(&new.name);
    let Some(old) = items.iter_mut().find(|i| !i.purchased && ingredients::match_key(&i.name) == key) else {
        items.push(new);
        return false;
    };
    let summed = match (plain_amount(&old.quantity), plain_amount(&new.quantity)) {
        (Some((q0, u0)), Some((q1, u1))) => match (&u0, &u1) {
            (None, None) => Some(amount_text(Some(q0 + q1), None)),
            (Some(a), Some(b)) => (if a == b { Some(q1) } else { convert(q1, b, a) }).map(|v| amount_text(Some(q0 + v), Some(a))),
            _ => None,
        },
        _ => None,
    };
    old.quantity = match summed {
        Some(t) => t,
        None if new.quantity.trim().is_empty() => old.quantity.clone(),
        None if old.quantity.trim().is_empty() => new.quantity.clone(),
        None => format!("{} + {}", old.quantity.trim(), new.quantity.trim()),
    };
    for r in item_recipes(&new.recipe).map(str::to_string).collect::<Vec<_>>() {
        if !item_recipes(&old.recipe).any(|x| x == r) {
            old.recipe = if old.recipe.trim().is_empty() { r } else { format!("{}, {}", old.recipe, r) };
        }
    }
    if old.note.trim().is_empty() {
        old.note = new.note;
    }
    true
}

/// Step an amount like "2 cups" or "1-2 cloves" by ±1 (stepping down stops at 0;
/// both ends of a range move). No number → "1" when stepping up.
pub fn step_quantity(text: &str, delta: f64) -> String {
    let p = parse_ingredient(&format!("{} x", text.trim()));
    let rest = text.trim().splitn(2, ' ').nth(1).unwrap_or("").to_string();
    match p.quantity {
        None => if delta > 0.0 { format!("1 {}", text.trim()).trim().to_string() } else { text.to_string() },
        Some(q) => {
            let q2 = (q + delta).max(0.0);
            let mut s = format_qty(q2);
            if let Some(m) = p.quantity_max {
                s = format!("{}-{}", s, format_qty((m + delta).max(0.0)));
            }
            let unit_part = match &p.unit {
                Some(u) => {
                    let plural = q2 != 1.0 || p.quantity_max.is_some();
                    if plural { ingredients::plural_of(u) } else { u.clone() }
                }
                None => {
                    // keep whatever followed the number
                    let words: Vec<&str> = rest.split_whitespace().collect();
                    let skip = if p.quantity_max.is_some() && words.first().map(|w| *w == "-" || *w == "to").unwrap_or(false) { 2 } else { 0 };
                    words.iter().skip(skip).copied().collect::<Vec<_>>().join(" ")
                }
            };
            if unit_part.is_empty() { s } else { format!("{} {}", s, unit_part) }
        }
    }
}

/// Recipes planned between two days (inclusive).
pub fn planned_recipes(lib: &Library, from: i32, to: i32) -> Vec<&Recipe> {
    lib.meals
        .iter()
        .filter(|m| m.day >= from && m.day <= to)
        .filter_map(|m| m.recipe_id.and_then(|id| lib.recipe(id)))
        .collect()
}

// ---------------- list helpers shared by every interface ----------------

/// The recipes an item came from (combined items list several, comma-separated).
pub fn item_recipes(recipe: &str) -> impl Iterator<Item = &str> {
    recipe.split(", ").map(|s| s.trim()).filter(|s| !s.is_empty())
}

/// Distinct recipe names on a list, for the Filters menu, sorted.
pub fn list_recipe_names(items: &[GroceryItem]) -> Vec<String> {
    let mut v: Vec<String> = items.iter().flat_map(|i| item_recipes(&i.recipe).map(|s| s.to_string()).collect::<Vec<_>>()).collect();
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup();
    v
}

/// Does a grocery item go, for the chosen "which"?
pub fn clear_matches(which: u8, purchased: bool) -> bool {
    match which {
        1 => purchased,
        2 => !purchased,
        _ => true,
    }
}

/// Store-walk order of sections on the grocery list (unknown sections go last, alphabetically).
pub const STORE_ORDER: &[&str] = &[
    "Breads and Cereals", "Beverages", "Canned and Jar Goods", "Dairy", "Bakery", "Baking Goods", "Produce", "Meat", "Seafood",
    "Deli", "Frozen Foods", "Pasta, Rice and Beans", "Oils and Dressings", "Sauces and Condiments", "Spices and Seasonings",
    "International Cuisine", "Snacks", "Beer, Wine and Spirits", "Baby Products", "Health and Beauty", "Pet Products",
    "Cleaning Supplies", "Home and Garden", "Miscellaneous",
];

pub fn aisle_rank(a: &str) -> (usize, String) {
    (STORE_ORDER.iter().position(|x| *x == a).unwrap_or(STORE_ORDER.len()), a.to_string())
}

pub fn all_aisles() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = ingredients::AISLES.iter().map(|a| a.0).collect();
    for s in STORE_ORDER {
        if !v.contains(s) {
            v.push(s);
        }
    }
    v.sort();
    v
}

/// "2 cups milk" -> (name "milk", quantity "2 cups")
pub fn split_quick_add(text: &str) -> (String, String) {
    let p = parse_ingredient(text);
    if p.quantity.is_some() && !p.name.is_empty() {
        let mut q = p.qty_text();
        let u = p.unit_text();
        if !u.is_empty() {
            q = format!("{} {}", q, u);
        }
        (p.name, q)
    } else {
        (text.trim().to_string(), String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PantryItem;

    #[test]
    fn adding_to_a_list_combines_what_is_already_there() {
        let it = |name: &str, q: &str, r: &str| GroceryItem { name: name.into(), quantity: q.into(), recipe: r.into(), ..Default::default() };
        let mut l = vec![it("milk", "1 cup", "Pancakes"), it("eggs", "2", "Pancakes")];
        assert!(add_to_list(&mut l, it("milk", "1 cup", "Soup")));
        assert!(add_to_list(&mut l, it("eggs", "3", "Soup")));
        assert!(add_to_list(&mut l, it("eggs", "1-2", "Cake")));
        assert!(!add_to_list(&mut l, it("flour", "2 cups", "Cake")));
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].quantity, "2 cups");
        assert_eq!(l[0].recipe, "Pancakes, Soup");
        assert_eq!(l[1].quantity, "5 + 1-2");
        // a bought item isn't topped up; the new one goes on the list by itself
        l[2].purchased = true;
        assert!(!add_to_list(&mut l, it("flour", "1 cup", "Bread")));
        assert_eq!(l.len(), 4);
    }

    #[test]
    fn step_amounts_like_the_original() {
        assert_eq!(step_quantity("2 cups", 1.0), "3 cups");
        assert_eq!(step_quantity("1 cup", -1.0), "0 cups");
        assert_eq!(step_quantity("0 cans", -1.0), "0 cans");
        assert_eq!(step_quantity("1-2 cloves", 1.0), "2-3 cloves");
        assert_eq!(step_quantity("", 1.0), "1");
    }

    #[test]
    fn combines_and_skips_pantry() {
        let mut lib = Library::default();
        lib.pantry.push(PantryItem { name: "eggs".into(), in_stock: true, ..Default::default() });
        let a = Recipe { name: "A".into(), ingredients: "1 cup milk\n2 eggs\n2 tbsp butter".into(), ..Default::default() };
        let b = Recipe { name: "B".into(), ingredients: "1 cup milk\n3 tbsp butter\n1 onion".into(), ..Default::default() };
        let rep = build(&lib, &[&a, &b], true, 0);
        assert_eq!(rep.in_pantry, vec!["eggs".to_string()]);
        let milk = rep.items.iter().find(|i| i.name == "milk").unwrap();
        assert_eq!(milk.quantity, "2 cups");
        assert_eq!(milk.aisle, "Dairy");
        let butter = rep.items.iter().find(|i| i.name == "butter").unwrap();
        assert_eq!(butter.quantity, "5 tablespoons");
        assert_eq!(butter.recipe, "A, B");
        assert!(rep.items.iter().any(|i| i.name == "onion" && i.aisle == "Produce"));
    }

    #[test]
    fn cooking_a_recipe_several_times_multiplies_amounts() {
        let lib = Library::default();
        let a = Recipe { id: 7, name: "A".into(), ingredients: "1 cup milk
2 eggs
salt to taste".into(), ..Default::default() };
        let rep = build_times(&lib, &[(&a, 5)], false, 0);
        assert_eq!(rep.items.iter().find(|i| i.name == "milk").unwrap().quantity, "5 cups");
        assert_eq!(rep.items.iter().find(|i| i.name == "eggs").unwrap().quantity, "10");
        // the same recipe planned on three days counts three times
        let rep = build(&lib, &[&a, &a, &a], false, 0);
        assert_eq!(rep.items.iter().find(|i| i.name == "eggs").unwrap().quantity, "6");
        assert_eq!(rep.items.iter().find(|i| i.name == "milk").unwrap().recipe, "A");
    }

    #[test]
    fn recipe_names_for_filter() {
        let items = vec![
            GroceryItem { recipe: "Pad Thai, Pho".into(), ..Default::default() },
            GroceryItem { recipe: "pho".into(), ..Default::default() },
            GroceryItem { recipe: "Apple Pie".into(), ..Default::default() },
            GroceryItem { recipe: "".into(), ..Default::default() },
        ];
        assert_eq!(list_recipe_names(&items), vec!["Apple Pie", "Pad Thai", "Pho", "pho"]);
        assert!(item_recipes("Pad Thai, Pho").any(|r| r == "Pho"));
    }

    #[test]
    fn which_items_go() {
        assert!(clear_matches(0, true) && clear_matches(0, false));
        assert!(clear_matches(1, true) && !clear_matches(1, false));
        assert!(!clear_matches(2, true) && clear_matches(2, false));
    }
}
