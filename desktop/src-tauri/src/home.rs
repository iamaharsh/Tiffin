//! The Home dashboard command: today at a glance, the macro hit rates, the streak and the two charts, built by
//! `recipe_box::tracker::home`. Reads only.

use crate::Shared;
use recipe_box::tracker::body::BodyRange;
use recipe_box::tracker::home::{self, HomeView};

/// `today` is the page's local day (`YYYY-MM-DD`); `range` sets how far back both charts reach.
pub(crate) fn view_of(shared: &Shared, range: BodyRange, today: &str) -> Result<HomeView, String> {
    let planned = crate::diary::planned_for(shared, today)?;
    let st = &mut *shared.lock().unwrap();
    let recipes = crate::foods::recipe_foods(&st.lib);
    home::home_view(&mut st.journal, today, range, planned, &st.lib.my_foods, &recipes)
}

#[tauri::command]
pub(crate) fn home_view(state: tauri::State<Shared>, range: BodyRange, today: String) -> Result<HomeView, String> {
    view_of(&state, range, &today)
}
