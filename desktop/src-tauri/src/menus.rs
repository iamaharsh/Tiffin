//! Menus: a reusable set of meals over a few days (Day 1, Day 2, …) that can be put onto the
//! meal plan from any start date.

use crate::{mutate, Shared};
use recipe_box::model::{self, Meal, Menu, MenuEntry};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct EntryOut {
    #[serde(flatten)]
    entry: MenuEntry,
    label: Option<usize>,
    photo: String,
    seed: u32,
}

#[derive(Serialize)]
pub(crate) struct MenuOut {
    id: u64,
    name: String,
    days: u32,
    entries: Vec<EntryOut>,
}

#[tauri::command]
pub(crate) fn menus(state: tauri::State<Shared>) -> Vec<MenuOut> {
    let s = state.lock().unwrap();
    let lib = &s.lib;
    lib.menus
        .iter()
        .map(|m| MenuOut {
            id: m.id,
            name: m.name.clone(),
            days: m.days,
            entries: m
                .entries
                .iter()
                .map(|e| {
                    let r = e.recipe_id.and_then(|id| lib.recipe(id));
                    EntryOut {
                        label: r.and_then(|r| model::recipe_label(&lib.label_colors, &r.categories).1),
                        photo: r.map(|r| r.photo.clone()).unwrap_or_default(),
                        seed: r.map_or(1, |r| r.photo_seed),
                        entry: e.clone(),
                    }
                })
                .collect(),
        })
        .collect()
}

#[tauri::command]
pub(crate) fn new_menu(state: tauri::State<Shared>) -> Result<u64, String> {
    mutate(&state, |st| {
        let id = st.lib.new_id();
        st.lib.menus.push(Menu { id, name: "New Menu".into(), days: 7, entries: vec![], modified: model::now_secs() });
        Ok(id)
    })
}

/// Save the menu as edited (name, number of days, its recipes). 1 to 28 days, like the classic app.
#[tauri::command]
pub(crate) fn save_menu(state: tauri::State<Shared>, id: u64, name: String, days: u32, entries: Vec<MenuEntry>) -> Result<(), String> {
    mutate(&state, |st| {
        let m = st.lib.menus.iter_mut().find(|m| m.id == id).ok_or("That menu isn't there any more.")?;
        let name = name.trim();
        m.name = if name.is_empty() { "Untitled Menu".into() } else { name.to_string() };
        m.days = days.clamp(1, 28);
        m.entries = entries.into_iter().filter(|e| e.day >= 1).collect();
        m.modified = model::now_secs();
        Ok(())
    })
}

/// Delete a menu (a Version History snapshot first); returns it so Undo can put it back.
#[tauri::command]
pub(crate) fn delete_menu(state: tauri::State<Shared>, id: u64) -> Result<Menu, String> {
    mutate(&state, |st| {
        let i = st.lib.menus.iter().position(|m| m.id == id).ok_or("That menu isn't there any more.")?;
        st.store.snapshot(&st.lib, "before deleting a menu")?;
        Ok(st.lib.menus.remove(i))
    })
}

#[tauri::command]
pub(crate) fn restore_menu(state: tauri::State<Shared>, mut menu: Menu) -> Result<(), String> {
    mutate(&state, |st| {
        if st.lib.menus.iter().any(|m| m.id == menu.id) {
            menu.id = st.lib.new_id();
        }
        menu.modified = model::now_secs();
        st.lib.menus.push(menu);
        Ok(())
    })
}

/// Day 1 lands on `start`. Recipes on days past the menu's length are left out (they're hidden
/// on the page too). Returns how many meals were added.
#[tauri::command]
pub(crate) fn apply_menu(state: tauri::State<Shared>, id: u64, start: i32) -> Result<usize, String> {
    mutate(&state, |st| {
        let m = st.lib.menus.iter().find(|m| m.id == id).ok_or("That menu isn't there any more.")?;
        let entries: Vec<MenuEntry> = m.entries.iter().filter(|e| e.day >= 1 && e.day <= m.days).cloned().collect();
        let t = model::now_secs();
        for e in &entries {
            let id = st.lib.new_id();
            st.lib.meals.push(Meal { id, day: start + e.day as i32 - 1, kind: e.kind, recipe_id: e.recipe_id, name: e.name.clone(), modified: t });
        }
        Ok(entries.len())
    })
}
