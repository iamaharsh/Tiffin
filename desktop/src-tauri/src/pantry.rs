//! The pantry: what's in the cupboard, when it was bought and when it runs out. Same actions as
//! the classic Pantry screen.

use crate::grocery::{undoable, undoable_single};
use crate::{mutate, Shared};
use recipe_box::groceries::{aisle_rank, all_aisles, split_quick_add};
use recipe_box::ingredients;
use recipe_box::model::{self, Expiry, GroceryItem, GroceryList, PantryItem};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub(crate) struct Item {
    id: u64,
    name: String,
    quantity: String,
    aisle: String,
    rank: usize,
    bought: Option<i32>,
    expires: Option<i32>,
    in_stock: bool,
    /// expired | today | week | month | later (none without a date)
    expiry: Option<&'static str>,
}

#[derive(Serialize)]
pub(crate) struct Pantry {
    items: Vec<Item>,
    /// 0 section, 1 date added, 2 expiration date, 3 in stock (the classic settings value)
    sort: u8,
    aisles: Vec<&'static str>,
    lists: Vec<(u64, String)>,
    today: i32,
}

fn expiry_word(e: i32, today: i32) -> &'static str {
    match model::expiry_status(e, today) {
        Expiry::Expired => "expired",
        Expiry::Today => "today",
        Expiry::ThisWeek => "week",
        Expiry::ThisMonth => "month",
        Expiry::Later => "later",
    }
}

#[tauri::command]
pub(crate) fn pantry(state: tauri::State<Shared>) -> Pantry {
    let s = state.lock().unwrap();
    let today = model::today();
    Pantry {
        items: s
            .lib
            .pantry
            .iter()
            .map(|p| Item {
                id: p.id,
                name: p.name.clone(),
                quantity: p.quantity.clone(),
                rank: aisle_rank(&p.aisle).0,
                aisle: p.aisle.clone(),
                bought: p.bought,
                expires: p.expires,
                in_stock: p.in_stock,
                expiry: p.expires.map(|e| expiry_word(e, today)),
            })
            .collect(),
        sort: s.lib.settings.pantry_sort,
        aisles: all_aisles(),
        lists: s.lib.grocery_lists.iter().map(|l| (l.id, l.name.clone())).collect(),
        today,
    }
}

/// Quick add: "2 lbs flour", bought today, in stock, filed under its store section.
#[tauri::command]
pub(crate) fn add_pantry(state: tauri::State<Shared>, text: String) -> Result<(), String> {
    let (name, quantity) = split_quick_add(&text);
    if name.trim().is_empty() {
        return Ok(());
    }
    mutate(&state, |st| {
        let id = st.lib.new_id();
        st.lib.pantry.push(PantryItem {
            id,
            aisle: ingredients::aisle_for(&name).into(),
            name,
            quantity,
            bought: Some(model::today()),
            expires: None,
            in_stock: true,
            modified: model::now_secs(),
        });
        Ok(())
    })
}

/// The item as edited (every field, so a date can be cleared).
#[derive(Deserialize)]
pub(crate) struct PantryEdit {
    id: u64,
    name: String,
    quantity: String,
    aisle: String,
    bought: Option<i32>,
    expires: Option<i32>,
    in_stock: bool,
}

#[tauri::command]
pub(crate) fn update_pantry(state: tauri::State<Shared>, item: PantryEdit) -> Result<(), String> {
    if item.name.trim().is_empty() {
        return Err("Give the item a name.".into());
    }
    mutate(&state, |st| {
        let p = st.lib.pantry.iter_mut().find(|p| p.id == item.id).ok_or("That item isn't in the pantry any more.")?;
        p.name = item.name.trim().to_string();
        p.quantity = item.quantity.trim().to_string();
        p.aisle = item.aisle;
        p.bought = item.bought;
        p.expires = item.expires;
        p.in_stock = item.in_stock;
        p.modified = model::now_secs();
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn delete_pantry(state: tauri::State<Shared>, id: u64) -> Result<Option<String>, String> {
    undoable_single(&state, "before deleting a pantry item", |_| "Deleted 1 item".into(), |lib| {
        let before = lib.pantry.len();
        lib.pantry.retain(|p| p.id != id);
        Ok(before - lib.pantry.len())
    })
}

#[tauri::command]
pub(crate) fn remove_expired(state: tauri::State<Shared>) -> Result<Option<String>, String> {
    let today = model::today();
    undoable(&state, "before removing expired items", |n| format!("Removed {n} expired item{}", if n == 1 { "" } else { "s" }), |lib| {
        let before = lib.pantry.len();
        lib.pantry.retain(|p| p.expires.is_none_or(|e| e >= today));
        Ok(before - lib.pantry.len())
    })
}

#[tauri::command]
pub(crate) fn remove_out_of_stock(state: tauri::State<Shared>) -> Result<Option<String>, String> {
    undoable(&state, "before removing out of stock items", |n| format!("Removed {n} out of stock item{}", if n == 1 { "" } else { "s" }), |lib| {
        let before = lib.pantry.len();
        lib.pantry.retain(|p| p.in_stock);
        Ok(before - lib.pantry.len())
    })
}

/// Put chosen pantry items on a grocery list, skipping ones already there and not yet bought.
#[tauri::command]
pub(crate) fn pantry_to_list(state: tauri::State<Shared>, ids: Vec<u64>, list: u64) -> Result<String, String> {
    mutate(&state, |st| {
        let lib = &mut st.lib;
        if lib.grocery_lists.is_empty() {
            let id = lib.new_id();
            lib.grocery_lists.push(GroceryList { id, name: "My Grocery List".into(), ..Default::default() });
        }
        let li = lib.grocery_lists.iter().position(|l| l.id == list).unwrap_or(0);
        let wanted: Vec<PantryItem> = lib.pantry.iter().filter(|p| ids.contains(&p.id)).cloned().collect();
        let mut n = 0;
        for p in wanted {
            if lib.grocery_lists[li].items.iter().any(|g| !g.purchased && ingredients::pantry_covers(&p.name, &g.name)) {
                continue;
            }
            let id = lib.new_id();
            let aisle = if p.aisle.is_empty() { ingredients::aisle_for(&p.name).into() } else { p.aisle };
            lib.grocery_lists[li].items.push(GroceryItem { id, name: p.name, quantity: p.quantity, aisle, recipe: "Pantry".into(), ..Default::default() });
            n += 1;
        }
        let l = &mut lib.grocery_lists[li];
        if n > 0 {
            l.modified = model::now_secs();
        }
        Ok(if n == 0 { format!("They're already on {}", l.name) } else { format!("Added {n} item{} to {}", if n == 1 { "" } else { "s" }, l.name) })
    })
}

#[tauri::command]
pub(crate) fn set_pantry_sort(state: tauri::State<Shared>, sort: u8) -> Result<(), String> {
    mutate(&state, |st| {
        st.lib.settings.pantry_sort = sort.min(3);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn expiry_words_follow_the_calendar() {
        // 2026-11-13 is a Friday (day 20405)
        let today = recipe_box::model::days_from_civil(2026, 11, 13);
        assert_eq!(super::expiry_word(today - 1, today), "expired");
        assert_eq!(super::expiry_word(today, today), "today");
        assert_eq!(super::expiry_word(today + 1, today), "week");
        assert_eq!(super::expiry_word(today + 10, today), "month");
        assert_eq!(super::expiry_word(today + 40, today), "later");
    }
}
