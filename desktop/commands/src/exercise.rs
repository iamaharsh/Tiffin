//! The Exercise commands: estimate, log, edit and delete a workout, and keep the person's own activities. Every
//! write goes through `mutate_journal`, so library.json is never touched; the rules live in
//! `recipe_box::tracker::exercise`. Each write answers with `crate::diary::view`, so the page gets the whole day back.

use crate::diary::{Changed, Logged};
use crate::{mutate_journal, Shared};
use recipe_box::model;
use recipe_box::tracker::exercise::{self, Activities, ActivityRef, Estimate};
use recipe_box::tracker::journal::CustomActivity;

/// What an activity gives for a duration on `day`, with the working. Writes nothing.
pub fn estimate_in(shared: &Shared, day: &str, activity: &ActivityRef, minutes: f64) -> Result<Estimate, String> {
    exercise::estimate(&mut shared.lock().unwrap().journal, day, activity, minutes)
}

/// Log a workout into `day`. A single add ends the pending undo, as a single food add does (Phase 3 D-12).
pub fn log_in(shared: &Shared, day: &str, today: &str, activity: &ActivityRef, minutes: f64, kcal: Option<f64>) -> Result<Logged, String> {
    mutate_journal(shared, |st| {
        st.journal.take_undo();
        let name = exercise::log(&mut st.journal, day, activity, minutes, kcal, model::now_secs())?;
        Ok(Logged { view: crate::diary::view(st, day, today)?, name })
    })
}

/// What the Add Exercise dialog lists: Recent, My Activities and the common rows for an empty box, ranked rows for what was typed. Writes nothing.
pub fn activities_in(shared: &Shared, q: &str, today: &str) -> Result<Activities, String> {
    exercise::activities(&mut shared.lock().unwrap().journal, q, today)
}

/// Save one of the person's own activities (a new one, or a known uid replaced) and return its uid.
pub fn save_in(shared: &Shared, activity: CustomActivity) -> Result<String, String> {
    mutate_journal(shared, |st| exercise::save_activity(&mut st.journal, activity))
}

/// Change a workout's duration and calories against its frozen basis, and keep the undo.
pub fn edit_in(shared: &Shared, day: &str, today: &str, uid: &str, minutes: f64, kcal: Option<f64>, use_estimate: bool) -> Result<Changed, String> {
    mutate_journal(shared, |st| {
        let (name, u) = exercise::edit(&mut st.journal, day, uid, minutes, kcal, use_estimate, model::now_secs())?;
        st.journal.set_undo(u);
        Ok(Changed { view: crate::diary::view(st, day, today)?, name })
    })
}

/// Delete a workout (it stays in the month file) and keep the undo.
pub fn delete_in(shared: &Shared, day: &str, today: &str, uid: &str) -> Result<Changed, String> {
    mutate_journal(shared, |st| {
        let (name, u) = exercise::delete(&mut st.journal, day, uid, model::now_secs())?;
        st.journal.set_undo(u);
        Ok(Changed { view: crate::diary::view(st, day, today)?, name })
    })
}

/// Remove one of the person's own activities. What was logged with it stays as logged.
pub fn delete_activity_in(shared: &Shared, uid: &str) -> Result<String, String> {
    mutate_journal(shared, |st| exercise::delete_activity(&mut st.journal, uid))
}

pub fn exercise_activities(state: &Shared, q: String, today: String) -> Result<Activities, String> {
    activities_in(&state, &q, &today)
}

pub fn exercise_estimate(state: &Shared, day: String, activity: ActivityRef, minutes: f64) -> Result<Estimate, String> {
    estimate_in(&state, &day, &activity, minutes)
}

pub fn log_exercise(state: &Shared, day: String, today: String, activity: ActivityRef, minutes: f64, kcal: Option<f64>) -> Result<Logged, String> {
    log_in(&state, &day, &today, &activity, minutes, kcal)
}

pub fn edit_exercise(state: &Shared, day: String, today: String, uid: String, minutes: f64, kcal: Option<f64>, use_estimate: bool) -> Result<Changed, String> {
    edit_in(&state, &day, &today, &uid, minutes, kcal, use_estimate)
}

pub fn delete_exercise(state: &Shared, day: String, today: String, uid: String) -> Result<Changed, String> {
    delete_in(&state, &day, &today, &uid)
}

pub fn delete_activity(state: &Shared, uid: String) -> Result<String, String> {
    delete_activity_in(&state, &uid)
}

pub fn save_activity(state: &Shared, activity: CustomActivity) -> Result<String, String> {
    save_in(&state, activity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_box::model::MealType;
    use recipe_box::tracker::goals::{self, TargetSet};
    use recipe_box::tracker::journal::ExerciseBasis;
    use recipe_box::tracker::body;
    use std::sync::Mutex;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = crate::scratch_dir().join(format!("rb-desktop-exercise-{}-{}", name, crate::test_pid()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn open(d: &std::path::Path) -> Shared {
        Mutex::new(crate::load_state(d.to_path_buf()))
    }

    fn bouldering(shared: &Shared) -> ActivityRef {
        ActivityRef::Custom(save_in(shared, CustomActivity { uid: String::new(), name: "Bouldering".into(), basis: ExerciseBasis::Met(5.8) }).unwrap())
    }

    #[test]
    fn logging_exercise_writes_only_the_journal() {
        let d = tdir("only-journal");
        let shared = open(&d);
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();
        {
            let mut st = shared.lock().unwrap();
            body::set_value(&mut st.journal, "2026-11-17", "2026-11-17", "weight", 80.0, 1).unwrap();
        }
        let r = bouldering(&shared);
        // a pending undo ends with a single add
        crate::diary::quick(&shared, "2026-11-17", "2026-11-17", MealType::Lunch, "Lunch", Some(500.0), None, None, None).unwrap();
        let uid = shared.lock().unwrap().journal.month("2026-11").unwrap().entries[0].uid.clone();
        crate::diary::remove(&shared, "2026-11-17", "2026-11-17", &uid).unwrap();
        let a = log_in(&shared, "2026-11-17", "2026-11-17", &r, 45.0, None).unwrap();
        assert_eq!(a.name, "Bouldering");
        assert_eq!(a.view.exercise.entries.len(), 1);
        assert_eq!(a.view.exercise.entries[0].kcal, Some(288));
        assert_eq!(a.view.planned_count, 0);
        assert!(shared.lock().unwrap().journal.take_undo().is_none(), "the add ended the pending undo");
        assert!(d.join("journal").join("2026-11.json").exists());
        assert!(d.join("journal").join("profile.json").exists(), "the activity is in the profile");
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        assert!(log_in(&shared, "2026-11-17", "2026-11-17", &r, 0.0, None).is_err());
        assert!(estimate_in(&shared, "2026-11-17", &r, 45.0).unwrap().kcal == Some(288));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn exercise_moves_remaining_not_the_target() {
        let d = tdir("remaining");
        let shared = open(&d);
        {
            let mut st = shared.lock().unwrap();
            goals::put_target(st.journal.profile_mut().unwrap(), TargetSet { effective_from: "2026-11-01".into(), kcal: 2100, floor_kcal: 1200, ..Default::default() });
            body::set_value(&mut st.journal, "2026-11-17", "2026-11-17", "weight", 80.0, 1).unwrap();
        }
        let food = crate::diary::quick(&shared, "2026-11-17", "2026-11-17", MealType::Lunch, "Lunch", Some(1700.0), None, None, None).unwrap();
        let b = food.view.budget.unwrap();
        assert_eq!((b.target, b.remaining), (2100, Some(400)));
        let r = bouldering(&shared);
        let b = log_in(&shared, "2026-11-17", "2026-11-17", &r, 45.0, None).unwrap().view.budget.unwrap();
        assert_eq!((b.target, b.remaining), (2100, Some(688)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn activity_lists_read_without_writing() {
        let d = tdir("lists");
        let shared = open(&d);
        let a = activities_in(&shared, "", "2026-11-17").unwrap();
        assert!(a.recent.is_empty() && a.mine.is_empty() && a.rows.is_empty());
        let r = bouldering(&shared);
        log_in(&shared, "2026-11-17", "2026-11-17", &r, 30.0, Some(120.0)).unwrap();
        let a = activities_in(&shared, "", "2026-11-17").unwrap();
        assert_eq!((a.recent.len(), a.mine.len()), (1, 1));
        assert!(activities_in(&shared, "bould", "2026-11-17").unwrap().rows[0].custom, "the one logged ranks first");
        assert!(activities_in(&shared, "", "not a day").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    const DAY: &str = "2026-11-17";

    /// Weighs 80 kg on `day` and logs 45 minutes of a custom MET 7.5 activity: an estimate of 390.
    fn walked(shared: &Shared, day: &str) -> (ActivityRef, String) {
        body::set_value(&mut shared.lock().unwrap().journal, day, day, "weight", 80.0, 1).unwrap();
        let r = ActivityRef::Custom(save_in(shared, CustomActivity { uid: String::new(), name: "Hill walk".into(), basis: ExerciseBasis::Met(7.5) }).unwrap());
        let uid = log_in(shared, day, day, &r, 45.0, None).unwrap().view.exercise.entries[0].uid.clone();
        (r, uid)
    }

    #[test]
    fn edit_then_undo_keeps_the_override_rules() {
        let d = tdir("edit");
        let shared = open(&d);
        let (_, uid) = walked(&shared, DAY);
        let c = edit_in(&shared, DAY, DAY, &uid, 45.0, Some(400.0), false).unwrap();
        assert_eq!(c.name, "Hill walk");
        let x = &c.view.exercise.entries[0];
        assert_eq!((x.kcal, x.edited, x.kcal_text.as_deref()), (Some(400), true, Some("\u{2212}400")));
        // a new duration keeps the person's number; Use estimate clears it
        let c = edit_in(&shared, DAY, DAY, &uid, 30.0, None, false).unwrap();
        assert_eq!((c.view.exercise.entries[0].kcal, c.view.exercise.entries[0].minutes_text.as_str()), (Some(400), "30 min"));
        let c = edit_in(&shared, DAY, DAY, &uid, 30.0, None, true).unwrap();
        assert_eq!((c.view.exercise.entries[0].kcal, c.view.exercise.entries[0].edited), (Some(260), false));
        // the undo puts back the step just before it
        let u = crate::diary::undo_last(&shared, DAY, DAY).unwrap();
        assert_eq!(u.day, DAY);
        let x = &u.view.exercise.entries[0];
        assert_eq!((x.kcal, x.edited, x.minutes_text.as_str()), (Some(400), true, "30 min"));
        assert_eq!(crate::diary::undo_last(&shared, DAY, DAY).err().as_deref(), Some("Undo isn't available after another change."));
        assert!(edit_in(&shared, DAY, DAY, &uid, 0.0, None, false).is_err());
        assert!(edit_in(&shared, DAY, DAY, "nope", 30.0, None, false).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_exercise_then_undo_puts_it_back() {
        let d = tdir("delete");
        let shared = open(&d);
        let (_, uid) = walked(&shared, DAY);
        let c = delete_in(&shared, DAY, DAY, &uid).unwrap();
        assert_eq!((c.name.as_str(), c.view.exercise.entries.len(), c.view.exercise.total_text.clone()), ("Hill walk", 0, None));
        let u = crate::diary::undo_last(&shared, DAY, DAY).unwrap();
        let x = &u.view.exercise.entries[0];
        assert_eq!((x.uid.as_str(), x.kcal), (uid.as_str(), Some(390)), "the same entry comes back");
        // a custom activity can be deleted without touching what was logged
        let act = match &walked_activity(&shared) {
            ActivityRef::Custom(a) => a.clone(),
            _ => unreachable!(),
        };
        assert_eq!(delete_activity_in(&shared, &act).unwrap(), "Hill walk");
        assert_eq!(crate::diary::view(&mut shared.lock().unwrap(), DAY, DAY).unwrap().exercise.entries.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    fn walked_activity(shared: &Shared) -> ActivityRef {
        let mut st = shared.lock().unwrap();
        let uid = st.journal.profile().unwrap().activities[0].uid.clone();
        ActivityRef::Custom(uid)
    }

    #[test]
    fn copy_yesterday_copies_exercise_and_undoes_with_food() {
        let d = tdir("copy");
        let shared = open(&d);
        let (yesterday, today) = ("2026-11-16", "2026-11-17");
        walked(&shared, yesterday);
        crate::diary::quick(&shared, yesterday, today, MealType::Lunch, "Lunch", Some(500.0), None, None, None).unwrap();
        let v = crate::diary::view(&mut shared.lock().unwrap(), today, today).unwrap();
        assert_eq!(v.previous_day_count, 2, "the food and the workout");
        let c = crate::diary::copy_in(&shared, yesterday, today, today, None).unwrap();
        assert_eq!(c.count, 2);
        let foods = |v: &recipe_box::tracker::diary::DayView| v.slots.iter().map(|s| s.entries.len()).sum::<usize>();
        assert_eq!((foods(&c.view), c.view.exercise.entries.len(), c.view.exercise.entries[0].kcal), (1, 1, Some(390)));
        let u = crate::diary::undo_last(&shared, today, today).unwrap();
        assert_eq!((foods(&u.view), u.view.exercise.entries.len()), (0, 0), "one undo takes both copies away");
        let y = crate::diary::view(&mut shared.lock().unwrap(), yesterday, today).unwrap();
        assert_eq!((foods(&y), y.exercise.entries.len()), (1, 1), "yesterday is untouched");
        let _ = std::fs::remove_dir_all(&d);
    }
}
