//! Grocery lists: the classic Groceries screen's actions, plus "Add to Grocery List" from a recipe.

use crate::{mutate, AppState, Shared, GONE};
use recipe_box::groceries::{self, aisle_rank, all_aisles, clear_matches, item_recipes, list_recipe_names, split_quick_add};
use recipe_box::ingredients;
use recipe_box::model::{self, GroceryItem, GroceryList, Library, PantryItem};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct ListInfo {
    id: u64,
    name: String,
    to_buy: usize,
}

#[derive(Serialize)]
pub struct Item {
    id: u64,
    name: String,
    note: String,
    quantity: String,
    aisle: String,
    /// store-walk position of the aisle, for sorting
    rank: usize,
    recipe: String,
    purchased: bool,
    /// label colour of the first recipe it came from
    label: Option<usize>,
}

#[derive(Serialize)]
pub struct Groceries {
    lists: Vec<ListInfo>,
    /// the list shown (the asked-for one, or the first)
    list: u64,
    items: Vec<Item>,
    recipes: Vec<String>,
    aisles: Vec<&'static str>,
    by_recipe: bool,
    pantry: usize,
    today: i32,
}

fn list_mut(lib: &mut Library, id: u64) -> Result<&mut GroceryList, String> {
    lib.grocery_lists.iter_mut().find(|l| l.id == id).ok_or_else(|| "That grocery list isn't there any more.".into())
}

/// Pantry items that are in stock and not past their date.
fn pantry_names(lib: &Library, today: i32) -> Vec<String> {
    lib.pantry.iter().filter(|p| p.in_stock && p.expires.is_none_or(|e| e >= today)).map(|p| p.name.clone()).collect()
}

pub fn groceries(state: &Shared, list: u64) -> Result<Groceries, String> {
    // There's always at least one list (the classic app makes it when the screen opens).
    if state.lock().unwrap().lib.grocery_lists.is_empty() {
        let _ = mutate(&state, |st| {
            if st.lib.grocery_lists.is_empty() {
                let id = st.lib.new_id();
                st.lib.grocery_lists.push(GroceryList { id, name: "My Grocery List".into(), ..Default::default() });
            }
            Ok(())
        });
    }
    let s = state.lock().unwrap();
    let lib = &s.lib;
    let shown = lib.grocery_lists.iter().find(|l| l.id == list).or(lib.grocery_lists.first());
    let label_of = |recipe: &str| {
        let first = item_recipes(recipe).next()?;
        let r = lib.recipes.iter().find(|r| r.name == first)?;
        model::recipe_label(&lib.label_colors, &r.categories).1
    };
    Ok(Groceries {
        lists: lib.grocery_lists.iter().map(|l| ListInfo { id: l.id, name: l.name.clone(), to_buy: l.items.iter().filter(|i| !i.purchased).count() }).collect(),
        list: shown.map_or(0, |l| l.id),
        items: shown
            .map(|l| {
                l.items
                    .iter()
                    .map(|i| Item {
                        id: i.id,
                        name: i.name.clone(),
                        note: i.note.clone(),
                        quantity: i.quantity.clone(),
                        rank: aisle_rank(&i.aisle).0,
                        aisle: i.aisle.clone(),
                        label: label_of(&i.recipe),
                        recipe: i.recipe.clone(),
                        purchased: i.purchased,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        recipes: shown.map(|l| list_recipe_names(&l.items)).unwrap_or_default(),
        aisles: all_aisles(),
        by_recipe: lib.settings.grocery_by_recipe,
        pantry: lib.pantry.len(),
        today: model::today(),
    })
}

#[derive(Serialize)]
pub struct Preview {
    items: usize,
    in_pantry: Vec<String>,
    recipes: usize,
}

fn preview_of(rep: &groceries::BuildReport) -> Preview {
    Preview { items: rep.items.len(), in_pantry: rep.in_pantry.clone(), recipes: rep.recipes }
}

fn add_items(st: &mut AppState, list: u64, items: Vec<GroceryItem>) -> Result<String, String> {
    let ids: Vec<u64> = items.iter().map(|_| st.lib.new_id()).collect();
    let l = list_mut(&mut st.lib, list)?;
    let n = items.len();
    let mut combined = 0;
    for (mut it, id) in items.into_iter().zip(ids) {
        it.id = id;
        combined += groceries::add_to_list(&mut l.items, it) as usize;
    }
    l.modified = model::now_secs();
    let s = if n == 1 { "" } else { "s" };
    Ok(match combined {
        0 => format!("Added {n} item{s} to {}", l.name),
        c => format!("Added {n} item{s} to {} ({c} combined with what was already on it)", l.name),
    })
}

/// What "Add to Grocery List" would add: the recipe's ingredients × times cooked, combined.
pub fn grocery_preview(state: &Shared, id: u64, times: u32, skip: bool) -> Result<Preview, String> {
    let s = state.lock().unwrap();
    let r = s.lib.recipe(id).ok_or(GONE)?;
    Ok(preview_of(&groceries::build_times(&s.lib, &[(r, times.clamp(1, 50))], skip, model::today())))
}

pub fn add_recipe_to_list(state: &Shared, id: u64, list: u64, times: u32, skip: bool) -> Result<String, String> {
    mutate(&state, |st| {
        let r = st.lib.recipe(id).ok_or(GONE)?;
        let rep = groceries::build_times(&st.lib, &[(r, times.clamp(1, 50))], skip, model::today());
        add_items(st, list, rep.items)
    })
}

pub fn plan_preview(state: &Shared, from: i32, to: i32, skip: bool) -> Result<Preview, String> {
    let s = state.lock().unwrap();
    let recipes = groceries::planned_recipes(&s.lib, from, to);
    Ok(preview_of(&groceries::build(&s.lib, &recipes, skip, model::today())))
}

pub fn add_plan_to_list(state: &Shared, list: u64, from: i32, to: i32, skip: bool) -> Result<String, String> {
    mutate(&state, |st| {
        let recipes = groceries::planned_recipes(&st.lib, from, to);
        let rep = groceries::build(&st.lib, &recipes, skip, model::today());
        let skipped = rep.in_pantry.len();
        let msg = add_items(st, list, rep.items)?;
        Ok(if skipped > 0 { format!("{msg}; {skipped} already in the pantry") } else { msg })
    })
}

/// Quick add: "2 cups milk" becomes milk, 2 cups, filed under its store section.
pub fn add_item(state: &Shared, list: u64, text: String) -> Result<(), String> {
    let (name, quantity) = split_quick_add(&text);
    if name.trim().is_empty() {
        return Ok(());
    }
    mutate(&state, |st| {
        let id = st.lib.new_id();
        let aisle = ingredients::aisle_for(&name).to_string();
        let l = list_mut(&mut st.lib, list)?;
        l.items.push(GroceryItem { id, name, quantity, aisle, recipe: "My Items".into(), ..Default::default() });
        l.modified = model::now_secs();
        Ok(())
    })
}

/// One change to an item: tick it, move it to a section, step its amount, or retype it.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct ItemChange {
    purchased: Option<bool>,
    aisle: Option<String>,
    step: Option<f64>,
    name: Option<String>,
    quantity: Option<String>,
}

pub fn update_item(state: &Shared, list: u64, id: u64, change: ItemChange) -> Result<(), String> {
    mutate(&state, |st| {
        let l = list_mut(&mut st.lib, list)?;
        let it = l.items.iter_mut().find(|i| i.id == id).ok_or("That item isn't on the list any more.")?;
        if let Some(p) = change.purchased {
            it.purchased = p;
        }
        if let Some(a) = change.aisle {
            it.aisle = a;
        }
        if let Some(d) = change.step {
            it.quantity = groceries::step_quantity(&it.quantity, d);
        }
        if let Some(n) = change.name.filter(|n| !n.trim().is_empty()) {
            it.name = n.trim().to_string();
        }
        if let Some(q) = change.quantity {
            it.quantity = q.trim().to_string();
        }
        l.modified = model::now_secs();
        Ok(())
    })
}

/// A removal that can be undone: Version History snapshot first, then the lists and pantry as
/// they were are kept for Undo. Returns the message for the undo bar (None when nothing went).
pub fn undoable(state: &Shared, why: &str, msg: impl Fn(usize) -> String, f: impl FnOnce(&mut Library) -> Result<usize, String>) -> Result<Option<String>, String> {
    undoable_with(state, why, 0, msg, f)
}

/// Removing one item at a time (a busy shop) takes at most one snapshot every few minutes, so
/// it doesn't push older restore points out of Version History; Undo covers each removal.
pub fn undoable_single(state: &Shared, why: &str, msg: impl Fn(usize) -> String, f: impl FnOnce(&mut Library) -> Result<usize, String>) -> Result<Option<String>, String> {
    undoable_with(state, why, SINGLE_SNAPSHOT_GAP, msg, f)
}

const SINGLE_SNAPSHOT_GAP: u64 = 300;

fn undoable_with(state: &Shared, why: &str, gap: u64, msg: impl Fn(usize) -> String, f: impl FnOnce(&mut Library) -> Result<usize, String>) -> Result<Option<String>, String> {
    crate::mutate_keeping_undo(state, |st| {
        let before = (st.lib.grocery_lists.clone(), st.lib.pantry.clone());
        st.store.snapshot_unless_recent(&st.lib, why, gap)?;
        let n = f(&mut st.lib)?;
        if n == 0 {
            return Ok(None);
        }
        st.undo = Some(before);
        Ok(Some(msg(n)))
    })
}

pub fn remove_item(state: &Shared, list: u64, id: u64) -> Result<Option<String>, String> {
    undoable_single(&state, "before removing a grocery item", |_| "Removed 1 item".into(), |lib| {
        let l = list_mut(lib, list)?;
        let before = l.items.len();
        l.items.retain(|i| i.id != id);
        l.modified = model::now_secs();
        Ok(before - l.items.len())
    })
}

pub fn new_list(state: &Shared, name: String) -> Result<u64, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the list a name.".into());
    }
    mutate(&state, |st| {
        let id = st.lib.new_id();
        st.lib.grocery_lists.push(GroceryList { id, name, items: vec![], modified: model::now_secs() });
        Ok(id)
    })
}

pub fn rename_list(state: &Shared, list: u64, name: String) -> Result<(), String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the list a name.".into());
    }
    mutate(&state, |st| {
        let l = list_mut(&mut st.lib, list)?;
        l.name = name;
        l.modified = model::now_secs();
        Ok(())
    })
}

/// There's always at least one list, so the last one can't be deleted.
pub fn delete_list(state: &Shared, list: u64) -> Result<Option<String>, String> {
    undoable(&state, "before deleting a grocery list", |_| "Deleted the list".into(), |lib| {
        if lib.grocery_lists.len() < 2 {
            return Err("This is your only grocery list, so it stays. You can clear its items instead.".into());
        }
        let before = lib.grocery_lists.len();
        lib.grocery_lists.retain(|l| l.id != list);
        Ok(before - lib.grocery_lists.len())
    })
}

/// "Clear all items": this list or every list (all, only bought, or only still to buy), and/or the pantry.
pub fn clear_items(state: &Shared, list: u64, all_lists: bool, pantry: bool, which: u8) -> Result<Option<String>, String> {
    undoable(&state, "before clearing items", |n| format!("Cleared {n} item{}", if n == 1 { "" } else { "s" }), |lib| {
        let t = model::now_secs();
        let mut n = 0;
        for l in lib.grocery_lists.iter_mut().filter(|l| all_lists || l.id == list) {
            let before = l.items.len();
            l.items.retain(|it| !clear_matches(which, it.purchased));
            n += before - l.items.len();
            l.modified = t;
        }
        if pantry {
            n += lib.pantry.len();
            lib.pantry.clear();
        }
        Ok(n)
    })
}

pub fn remove_pantry_items(state: &Shared, list: u64) -> Result<Option<String>, String> {
    undoable(&state, "before removing items already in the pantry", |n| format!("Removed {n} item{} you already have", if n == 1 { "" } else { "s" }), |lib| {
        let have = pantry_names(lib, model::today());
        let l = list_mut(lib, list)?;
        let before = l.items.len();
        l.items.retain(|it| !have.iter().any(|p| ingredients::pantry_covers(p, &it.name)));
        l.modified = model::now_secs();
        Ok(before - l.items.len())
    })
}

pub fn move_bought_to_pantry(state: &Shared, list: u64) -> Result<Option<String>, String> {
    undoable(&state, "before moving bought items to the pantry", |n| format!("Moved {n} bought item{} to the pantry", if n == 1 { "" } else { "s" }), |lib| {
        let today = model::today();
        let bought: Vec<GroceryItem> = list_mut(lib, list)?.items.iter().filter(|i| i.purchased).cloned().collect();
        for b in &bought {
            let id = lib.new_id();
            match lib.pantry.iter_mut().find(|p| ingredients::pantry_covers(&p.name, &b.name)) {
                Some(p) => {
                    p.in_stock = true;
                    p.bought = Some(today);
                    p.modified = model::now_secs();
                }
                None => lib.pantry.push(PantryItem {
                    id,
                    name: b.name.clone(),
                    quantity: b.quantity.clone(),
                    aisle: b.aisle.clone(),
                    bought: Some(today),
                    expires: None,
                    in_stock: true,
                    modified: model::now_secs(),
                }),
            }
        }
        let l = list_mut(lib, list)?;
        l.items.retain(|i| !i.purchased);
        l.modified = model::now_secs();
        Ok(bought.len())
    })
}

pub fn undo_groceries(state: &Shared) -> Result<(), String> {
    crate::mutate_keeping_undo(&state, |st| {
        let (lists, pantry) = st.undo.take().ok_or("Undo isn't available after another change.")?;
        let t = model::now_secs();
        st.lib.grocery_lists = lists;
        st.lib.pantry = pantry;
        st.lib.grocery_lists.iter_mut().for_each(|l| l.modified = t);
        st.lib.pantry.iter_mut().for_each(|p| p.modified = t);
        Ok(())
    })
}

pub fn set_grocery_by_recipe(state: &Shared, on: bool) -> Result<(), String> {
    mutate(&state, |st| {
        st.lib.settings.grocery_by_recipe = on;
        Ok(())
    })
}
