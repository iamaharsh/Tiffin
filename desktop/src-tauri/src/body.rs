//! The Body commands: log a weight or measurement, read the Body view, and take the last change back. Every write goes
//! through `mutate_journal`, so library.json is never touched; the rules live in `recipe_box::tracker::body`.

use crate::diary::NO_UNDO;
use crate::{mutate_journal, Shared};
use recipe_box::model;
use recipe_box::tracker::body::{self, BodyRange, BodyView, DayValue, MeasureKind, UnitIn};
use recipe_box::tracker::diary;
use recipe_box::tracker::targets::{self, Drift};
use serde::Serialize;

/// After a save: `confirm` is the question to ask when a weight looks wrong (nothing was saved; the same call with `confirmed` saves);
/// `text` is what was saved, for the toast; `drift` is the offer to update the target when the trend weight has moved (D-07);
/// `view` is the refreshed Body page when the caller gave a range.
#[derive(Serialize)]
pub(crate) struct BodyLogged {
    pub(crate) confirm: Option<String>,
    pub(crate) text: String,
    pub(crate) drift: Option<Drift>,
    pub(crate) view: Option<BodyView>,
}

pub(crate) fn view_of(shared: &Shared, range: BodyRange, today: &str) -> Result<BodyView, String> {
    body::body_view(&mut shared.lock().unwrap().journal, range, today)
}

/// Log one measurement and keep its undo. A weight is asked about once when it looks wrong, and a saved weight carries the drift offer.
#[allow(clippy::too_many_arguments)]
pub(crate) fn log_in(shared: &Shared, day: &str, today: &str, measure: &str, value: f64, unit: UnitIn, confirmed: bool, range: Option<BodyRange>) -> Result<BodyLogged, String> {
    mutate_journal(shared, |st| {
        let logged = body::log(&mut st.journal, day, today, measure, value, unit, confirmed, model::now_secs())?;
        let saved = logged.undo.is_some();
        if let Some(u) = logged.undo {
            st.journal.set_undo(u);
        }
        let drift = if saved && measure == "weight" { targets::drift(&mut st.journal, today) } else { None };
        let view = match range {
            Some(r) => Some(body::body_view(&mut st.journal, r, today)?),
            None => None,
        };
        Ok(BodyLogged { confirm: logged.confirm, text: logged.text, drift, view })
    })
}

/// Put back the last change (the Diary's undo slot, shared with food and exercise), then read the page.
pub(crate) fn undo_in(shared: &Shared, range: BodyRange, today: &str) -> Result<BodyView, String> {
    mutate_journal(shared, |st| {
        let u = st.journal.take_undo().ok_or(NO_UNDO)?;
        diary::undo(&mut st.journal, &u, model::now_secs())?;
        body::body_view(&mut st.journal, range, today)
    })
}

/// Save or clear several measures of one day in one step with one undo. Weight asks once, as `log_in` does.
pub(crate) fn edit_in(shared: &Shared, day: &str, today: &str, values: Vec<DayValue>, confirmed: bool, range: Option<BodyRange>) -> Result<BodyLogged, String> {
    mutate_journal(shared, |st| {
        let logged = body::set_day(&mut st.journal, day, today, &values, confirmed, model::now_secs())?;
        let saved = logged.undo.is_some();
        if let Some(u) = logged.undo {
            st.journal.set_undo(u);
        }
        let weighed = values.iter().any(|v| v.measure == "weight" && v.value.is_some());
        let drift = if saved && weighed { targets::drift(&mut st.journal, today) } else { None };
        let view = match range {
            Some(r) => Some(body::body_view(&mut st.journal, r, today)?),
            None => None,
        };
        Ok(BodyLogged { confirm: logged.confirm, text: logged.text, drift, view })
    })
}

/// Add one of the person's own measures, then read the page.
pub(crate) fn save_in(shared: &Shared, name: &str, kind: MeasureKind, range: BodyRange, today: &str) -> Result<BodyView, String> {
    mutate_journal(shared, |st| {
        body::save_measure(&mut st.journal, name, kind)?;
        body::body_view(&mut st.journal, range, today)
    })
}

/// Hide one of the person's own measures, or bring it back. Every value stays.
pub(crate) fn archive_in(shared: &Shared, uid: &str, archived: bool, range: BodyRange, today: &str) -> Result<BodyView, String> {
    mutate_journal(shared, |st| {
        body::archive_measure(&mut st.journal, uid, archived)?;
        body::body_view(&mut st.journal, range, today)
    })
}

#[tauri::command]
pub(crate) fn body_view(state: tauri::State<Shared>, range: BodyRange, today: String) -> Result<BodyView, String> {
    view_of(&state, range, &today)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) fn log_measure(state: tauri::State<Shared>, day: String, today: String, measure: String, value: f64, unit: UnitIn, confirmed: bool, range: Option<BodyRange>) -> Result<BodyLogged, String> {
    log_in(&state, &day, &today, &measure, value, unit, confirmed, range)
}

#[tauri::command]
pub(crate) fn undo_body(state: tauri::State<Shared>, range: BodyRange, today: String) -> Result<BodyView, String> {
    undo_in(&state, range, &today)
}

#[tauri::command]
pub(crate) fn edit_measurements(state: tauri::State<Shared>, day: String, today: String, values: Vec<DayValue>, confirmed: bool, range: Option<BodyRange>) -> Result<BodyLogged, String> {
    edit_in(&state, &day, &today, values, confirmed, range)
}

#[tauri::command]
pub(crate) fn save_measure(state: tauri::State<Shared>, name: String, kind: MeasureKind, range: BodyRange, today: String) -> Result<BodyView, String> {
    save_in(&state, &name, kind, range, &today)
}

#[tauri::command]
pub(crate) fn archive_measure(state: tauri::State<Shared>, uid: String, archived: bool, range: BodyRange, today: String) -> Result<BodyView, String> {
    archive_in(&state, &uid, archived, range, &today)
}

#[cfg(test)]
mod tests {
    use super::*;
    use recipe_box::tracker::goals::{Activity, Sex};
    use recipe_box::tracker::targets::{HeightInput, MassInput, TargetsDraft};
    use std::collections::BTreeMap;
    use std::path::Path;
    use std::sync::Mutex;

    const TODAY: &str = "2026-11-17";

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-desktop-body-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn open(d: &Path) -> Shared {
        Mutex::new(crate::load_state(d.to_path_buf()))
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

    fn weigh(shared: &Shared, day: &str, kg: f64, confirmed: bool) -> Result<BodyLogged, String> {
        log_in(shared, day, TODAY, "weight", kg, UnitIn::Kg, confirmed, None)
    }

    #[test]
    fn log_measure_writes_only_the_journal() {
        let d = tdir("only-journal");
        let shared = open(&d);
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        let snaps_before = shared.lock().unwrap().store.snapshots().len();
        let r = log_in(&shared, TODAY, TODAY, "weight", 78.9, UnitIn::Kg, false, Some(BodyRange::M3)).unwrap();
        assert_eq!((r.text.as_str(), r.confirm.clone(), r.drift.is_none()), ("78.9 kg", None, true));
        let v = r.view.unwrap();
        assert_eq!((v.headline.as_deref(), v.rows.len()), (Some("Trend 78.9 kg"), 1));
        assert!(weigh(&shared, TODAY, 79.0, false).unwrap().view.is_none(), "no range, no view");
        assert!(d.join("journal").join("2026-11.json").exists());
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        assert_eq!(shared.lock().unwrap().store.snapshots().len(), snaps_before);
        assert!(weigh(&shared, TODAY, 0.0, false).is_err());
        assert!(weigh(&shared, "2026-11-18", 80.0, false).is_err(), "a future day");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn implausible_weight_writes_nothing_until_confirmed() {
        let d = tdir("ask");
        let shared = open(&d);
        weigh(&shared, "2026-11-10", 148.9, false).unwrap();
        let before = bytes_of(&d);
        let r = weigh(&shared, TODAY, 78.9, false).unwrap();
        assert_eq!(r.confirm.as_deref(), Some("That's 70 kg less than your last weigh-in. Save anyway?"));
        assert!(r.text.is_empty() && r.drift.is_none());
        assert_eq!(bytes_of(&d), before, "the month file is byte for byte unchanged");
        assert!(shared.lock().unwrap().journal.take_undo().is_some(), "the earlier save's undo is still the pending one");
        let r = weigh(&shared, TODAY, 78.9, true).unwrap();
        assert!(r.confirm.is_none());
        assert_ne!(bytes_of(&d), before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn log_then_undo_body() {
        let d = tdir("undo");
        let shared = open(&d);
        let lib_before = std::fs::read(d.join("library.json")).unwrap();
        weigh(&shared, TODAY, 78.9, false).unwrap();
        let v = undo_in(&shared, BodyRange::M3, TODAY).unwrap();
        assert!(v.rows.is_empty() && v.empty, "the weigh-in is gone");
        assert_eq!(undo_in(&shared, BodyRange::M3, TODAY).unwrap_err(), "Undo isn't available after another change.");
        // a replace is undone to the value it replaced
        weigh(&shared, TODAY, 80.0, false).unwrap();
        weigh(&shared, TODAY, 79.0, false).unwrap();
        let v = undo_in(&shared, BodyRange::M3, TODAY).unwrap();
        assert_eq!(v.rows[0].cells[0].as_deref(), Some("80.0 kg"));
        assert_eq!(std::fs::read(d.join("library.json")).unwrap(), lib_before);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn body_view_has_chart_and_forecast() {
        let d = tdir("chart");
        let shared = open(&d);
        crate::mutate_journal(&shared, |st| {
            for day in 1..=17 {
                body::set_value(&mut st.journal, &format!("2026-11-{day:02}"), TODAY, "weight", 80.0 - 0.1 * f64::from(day), 1)?;
            }
            st.journal.profile_mut()?.goal_weight_kg = Some(78.0);
            Ok(())
        })
        .unwrap();
        let v = view_of(&shared, BodyRange::M1, TODAY).unwrap();
        let c = v.chart.unwrap();
        assert_eq!((c.points.len(), c.unit_text, c.goal), (17, "kg", Some(78.0)));
        assert!(c.y_min < 78.0 && c.y_max > 79.9);
        assert!(v.forecast.unwrap().starts_with("At this pace you reach 78.0 kg around "));
        assert!(v.headline.unwrap().starts_with("Trend 79."));
        assert!(v.rate_text.unwrap().ends_with(" kg/week"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn edit_measurements_is_one_undo() {
        let d = tdir("edit");
        let shared = open(&d);
        let day = "2026-11-15";
        log_in(&shared, day, TODAY, "weight", 80.0, UnitIn::Kg, true, None).unwrap();
        log_in(&shared, day, TODAY, "waist", 90.0, UnitIn::Cm, true, None).unwrap();
        let set = vec![DayValue { measure: "weight".into(), value: Some(79.5), unit: UnitIn::Kg }, DayValue { measure: "waist".into(), value: None, unit: UnitIn::Cm }, DayValue { measure: "chest".into(), value: Some(98.0), unit: UnitIn::Cm }];
        let r = edit_in(&shared, day, TODAY, set, false, Some(BodyRange::M3)).unwrap();
        assert_eq!((r.text.as_str(), r.confirm.is_none()), ("Nov 15", true));
        let v = r.view.unwrap();
        assert_eq!(v.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Weight", "Chest"], "the cleared waist is gone, the chest has a value");
        assert_eq!(v.rows[0].cells, [Some("79.5 kg".to_string()), Some("98.0 cm".to_string())]);
        let v = undo_in(&shared, BodyRange::M3, TODAY).unwrap();
        assert_eq!(v.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Weight", "Waist"]);
        assert_eq!(v.rows[0].cells, [Some("80.0 kg".to_string()), Some("90.0 cm".to_string())]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn custom_measure_archive_keeps_values() {
        let d = tdir("archive");
        let shared = open(&d);
        let v = save_in(&shared, "Calf", MeasureKind::Length, BodyRange::M3, TODAY).unwrap();
        let uid = v.measures.iter().find(|m| m.name == "Calf").unwrap().id.clone();
        log_in(&shared, TODAY, TODAY, &uid, 38.0, UnitIn::Cm, false, None).unwrap();
        let v = archive_in(&shared, &uid, true, BodyRange::M3, TODAY).unwrap();
        assert_eq!((v.columns.len(), v.archived.len()), (1, 1));
        assert!(shared.lock().unwrap().journal.month("2026-11").unwrap().body.iter().any(|e| e.measure == uid && e.deleted_at.is_none()), "the value is still in the month file");
        let v = archive_in(&shared, &uid, false, BodyRange::M3, TODAY).unwrap();
        assert_eq!((v.columns.len(), v.archived.len()), (2, 0));
        assert!(save_in(&shared, "calf", MeasureKind::Number, BodyRange::M3, TODAY).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn weight_save_reports_drift() {
        let d = tdir("drift");
        let shared = open(&d);
        // a target built on 82 kg on Nov 1, then a trend two weeks near 78
        let draft = TargetsDraft {
            sex: Some(Sex::Male),
            birth_date: Some("2000-05-01".into()),
            height: HeightInput { cm: Some(180.0), ..Default::default() },
            weight: MassInput { value: Some(82.0), ..Default::default() },
            weight_edited: true,
            activity: Some(Activity::Light),
            ..Default::default()
        };
        crate::goals::save(&shared, &draft, "2026-11-01", false).unwrap();
        let mut last = None;
        for day in 2..=15 {
            last = Some(log_in(&shared, &format!("2026-11-{day:02}"), TODAY, "weight", 78.0, UnitIn::Kg, true, None).unwrap());
        }
        // the last save is a weigh-in like any other and carries the offer
        let offer = last.unwrap().drift.expect("the trend moved about 4 kg from the target's weight");
        assert!(offer.text.contains("lower"), "{}", offer.text);
        // a measurement that is not a weight never carries one
        let w = log_in(&shared, TODAY, TODAY, "waist", 84.0, UnitIn::Cm, false, None).unwrap();
        assert!(w.drift.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
