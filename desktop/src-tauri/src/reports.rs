//! The Reports command: averages over finished days, built by `recipe_box::tracker::reports`. Reads only.

use crate::Shared;
use recipe_box::tracker::body::BodyRange;
use recipe_box::tracker::reports::{self, Report};

/// `today` is the page's local day (`YYYY-MM-DD`); the range ends the day before it.
#[tauri::command]
pub(crate) fn report_view(state: tauri::State<Shared>, range: BodyRange, today: String) -> Result<Report, String> {
    reports::report(&mut state.lock().unwrap().journal, range, &today)
}
