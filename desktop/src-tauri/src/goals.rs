//! The Targets page commands: read the page, preview a draft, save it, and take an offered target update. Every
//! write goes through `mutate_journal`, so library.json is never touched; the rules live in
//! `recipe_box::tracker::targets`.

use crate::{mutate_journal, Shared};
use recipe_box::model;
use recipe_box::tracker::targets::{self, TargetsDraft, TargetsResult, TargetsView};
use serde::Serialize;

/// What the Targets page opens with. `today` is the page's local day (`YYYY-MM-DD`).
pub(crate) fn view_of(shared: &Shared, today: &str) -> TargetsView {
    targets::targets_view(&mut shared.lock().unwrap().journal, today)
}

/// What a draft works out to, exactly as Save Targets would store it. Writes nothing.
pub(crate) fn preview_of(shared: &Shared, draft: &TargetsDraft, today: &str) -> TargetsResult {
    targets::preview(&mut shared.lock().unwrap().journal, draft, today)
}

/// The page after Save Targets. `confirm` is the sentence to ask when a typed weight is far from the last weigh-in:
/// nothing was saved, and saving again with `confirmed` true does it.
#[derive(Serialize)]
pub(crate) struct SavedTargets {
    pub(crate) view: TargetsView,
    pub(crate) confirm: Option<String>,
}

/// Save a draft as the target in force from `today`, then read the page back.
pub(crate) fn save(shared: &Shared, draft: &TargetsDraft, today: &str, confirmed: bool) -> Result<SavedTargets, String> {
    mutate_journal(shared, |st| {
        let saved = targets::save(&mut st.journal, draft, today, model::now_secs(), confirmed)?;
        Ok(SavedTargets { view: targets::targets_view(&mut st.journal, today), confirm: saved.confirm })
    })
}

/// Take the offered target update: a new target effective `today`. Nothing else changes the target when the weight drifts.
pub(crate) fn accept(shared: &Shared, today: &str) -> Result<TargetsView, String> {
    mutate_journal(shared, |st| {
        targets::accept_update(&mut st.journal, today)?;
        Ok(targets::targets_view(&mut st.journal, today))
    })
}

#[tauri::command]
pub(crate) fn targets_view(state: tauri::State<Shared>, today: String) -> TargetsView {
    view_of(&state, &today)
}

#[tauri::command]
pub(crate) fn targets_preview(state: tauri::State<Shared>, draft: TargetsDraft, today: String) -> TargetsResult {
    preview_of(&state, &draft, &today)
}

#[tauri::command]
pub(crate) fn save_targets(state: tauri::State<Shared>, draft: TargetsDraft, today: String, confirmed: bool) -> Result<SavedTargets, String> {
    save(&state, &draft, &today, confirmed)
}

#[tauri::command]
pub(crate) fn accept_target_update(state: tauri::State<Shared>, today: String) -> Result<TargetsView, String> {
    accept(&state, &today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_box::model::MealType;
    use recipe_box::tracker::diary::BudgetState;
    use recipe_box::tracker::goals::{Activity, Sex};
    use recipe_box::tracker::targets::{HeightInput, MassInput};
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Mutex;

    const TODAY: &str = "2026-11-17";

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-desktop-goals-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Male, born 2000-05-01, 180 cm, 80 kg typed, light movement: 2,475 kcal.
    pub(crate) fn draft() -> TargetsDraft {
        TargetsDraft {
            sex: Some(Sex::Male),
            birth_date: Some("2000-05-01".into()),
            height: HeightInput { cm: Some(180.0), ..Default::default() },
            weight: MassInput { value: Some(80.0), ..Default::default() },
            weight_edited: true,
            activity: Some(Activity::Light),
            ..Default::default()
        }
    }

    /// Every file under `dir` with its bytes.
    fn bytes_of(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.insert(p.strip_prefix(base).unwrap().display().to_string(), std::fs::read(&p).unwrap());
                }
            }
        }
        let mut out = BTreeMap::new();
        walk(dir, dir, &mut out);
        out
    }

    #[test]
    fn saving_targets_writes_only_the_profile_and_dates_it() {
        let d = tdir("save");
        let shared = Mutex::new(crate::load_state(d.clone()));
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();

        let v = save(&shared, &draft(), TODAY, false).unwrap();
        assert!(v.view.has_target && v.confirm.is_none());
        assert_eq!(v.view.result.kcal, Some(2475));
        let p: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join("journal").join("profile.json")).unwrap()).unwrap();
        assert_eq!(p["format"], 2);
        assert_eq!(p["targets"][0]["effective_from"], TODAY);
        assert_eq!(p["targets"][0]["kcal"], 2475);
        assert!(d.join("journal").join("2026-11.json").exists(), "the typed weight is today's weigh-in");

        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        // a second save the same day replaces that day's record
        let mut lose = draft();
        lose.goal = recipe_box::tracker::goals::GoalKind::Lose;
        save(&shared, &lose, TODAY, false).unwrap();
        let mut st = shared.lock().unwrap();
        assert_eq!(st.journal.profile().unwrap().targets.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn preview_writes_nothing() {
        let d = tdir("preview");
        let shared = Mutex::new(crate::load_state(d.clone()));
        save(&shared, &draft(), TODAY, false).unwrap();
        let before = bytes_of(&d);
        let mut other = draft();
        other.weight.value = Some(70.0);
        let r = preview_of(&shared, &other, TODAY);
        assert!(r.can_save);
        assert_ne!(r.kcal, Some(2475));
        let _ = view_of(&shared, TODAY);
        assert_eq!(bytes_of(&d), before, "no file changed");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn diary_day_shows_remaining_for_the_target_in_force() {
        let d = tdir("diary");
        let shared = Mutex::new(crate::load_state(d.clone()));
        let before = crate::diary::view(&mut shared.lock().unwrap(), TODAY, TODAY).unwrap();
        assert_eq!(before.budget_state, BudgetState::Never);
        save(&shared, &draft(), TODAY, false).unwrap();
        crate::diary::quick(&shared, TODAY, TODAY, MealType::Lunch, "Lunch", Some(500.0), None, None, None).unwrap();
        let v = crate::diary::view(&mut shared.lock().unwrap(), TODAY, TODAY).unwrap();
        let b = v.budget.unwrap();
        assert_eq!((v.budget_state, b.remaining, b.formula.as_str()), (BudgetState::Set, Some(1975), "Target 2,475 + Exercise 0 \u{2212} Food 500"));
        let yesterday = crate::diary::view(&mut shared.lock().unwrap(), "2026-11-16", TODAY).unwrap();
        assert_eq!((yesterday.budget_state, yesterday.budget_note.as_deref()), (BudgetState::Before, Some("No target was set for this day.")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn implausible_weight_asks_before_writing() {
        let d = tdir("ask");
        let shared = Mutex::new(crate::load_state(d.clone()));
        let mut first = draft();
        first.weight.value = Some(150.0);
        save(&shared, &first, "2026-11-10", false).unwrap();
        let before = bytes_of(&d);
        let r = save(&shared, &draft(), TODAY, false).unwrap(); // 80 kg typed, 70 kg below the last weigh-in
        assert_eq!(r.confirm.as_deref(), Some("That's 70 kg less than your last weigh-in. Save anyway?"));
        assert_eq!(bytes_of(&d), before, "profile.json and the month file are byte for byte unchanged");
        let r = save(&shared, &draft(), TODAY, true).unwrap();
        assert!(r.confirm.is_none() && r.view.has_target);
        assert_ne!(bytes_of(&d), before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn accepting_a_drift_dates_a_new_target() {
        let d = tdir("drift");
        let shared = Mutex::new(crate::load_state(d.clone()));
        let mut first = draft();
        first.weight.value = Some(82.0);
        save(&shared, &first, "2026-11-01", false).unwrap();
        crate::mutate_journal(&shared, |st| {
            for day in 2..=15 {
                recipe_box::tracker::body::set_value(&mut st.journal, &format!("2026-11-{day:02}"), TODAY, "weight", 78.0, 1)?;
            }
            Ok(())
        })
        .unwrap();
        let v = view_of(&shared, "2026-11-15");
        let offer = v.drift.expect("a drift of about 3 kg is offered");
        assert_eq!(v.earlier.len(), 0, "viewing the offer wrote nothing");
        assert_eq!(shared.lock().unwrap().journal.profile().unwrap().targets.iter().map(|t| t.kcal).collect::<Vec<_>>(), [2503], "the target in force is still the old one");

        let v = accept(&shared, "2026-11-15").unwrap();
        assert!(v.has_target && v.drift.is_none());
        assert_eq!(v.result.kcal, Some(offer.kcal), "the new target is in force from that day");
        assert_eq!(v.earlier.iter().map(|e| e.date_text.as_str()).collect::<Vec<_>>(), ["Nov 1"], "the first target stays as it was");
        assert!(accept(&shared, "2026-11-15").is_err(), "nothing more to accept");
        let _ = std::fs::remove_dir_all(&d);
    }
}
