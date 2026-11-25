//! Exercise logged into the journal month files, estimated from MET values as net calories and frozen at log time (D-17b).
//! A custom activity lives in profile.json; nothing here ever writes the cited activity table.

use crate::tracker::body;
use crate::tracker::day::Day;
use crate::tracker::diary::{self, new_uid, NOT_THERE};
use crate::tracker::goals::{self, kcal_text};
use crate::tracker::journal::{CustomActivity, DiaryUndo, ExerciseBasis, ExerciseEntry, ExerciseSource, Journal};
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

pub const BAD_MINUTES: &str = "Enter a duration above 0 minutes.";
pub const TOO_LONG: &str = "A day has 1,440 minutes. Enter a shorter duration.";
pub const BAD_KCAL: &str = "Enter calories from 0 to 10,000.";
pub const NO_ACTIVITY: &str = "That activity isn't in the list.";
pub const NO_CUSTOM: &str = "That activity isn't in your list any more.";
pub const NO_WEIGHT_NOTE: &str = "Add a weight to estimate calories, or enter your own number below.";
const MAX_MINUTES: f64 = 1440.0;
const MAX_KCAL: f64 = 10_000.0;
const MAX_MET: f64 = 30.0;
const MAX_KCAL_PER_HOUR: f64 = 3_000.0;
const MAX_ACTIVITIES: usize = 200;
const NAME_CHARS: usize = 60;

/// The energy above resting, in kcal: (MET - 1) x kg x hours, never below 0. Resting is left out because
/// the daily target already covers it (the carried-forward net rule).
pub fn net_kcal(met: f64, kg: f64, minutes: f64) -> f64 {
    (met - 1.0).max(0.0) * kg * minutes / 60.0
}

/// An activity the page picked: a row of the bundled table by id, or one of the person's own by uid.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityRef {
    Compendium(String),
    Custom(String),
}

/// A number to two places with no trailing zeros: 4.8 is "4.8", 45.0 is "45".
pub(crate) fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A MET value as people read it, always with a decimal: "7.5", "14.0".
pub(crate) fn met_str(met: f64) -> String {
    let s = trim(met);
    if s.contains('.') {
        s
    } else {
        format!("{s}.0")
    }
}

/// "45 min", "37.5 min".
pub(crate) fn minutes_text(minutes: f64) -> String {
    format!("{} min", trim(minutes))
}

fn check_minutes(minutes: f64) -> Result<f64, String> {
    if !minutes.is_finite() || minutes <= 0.0 {
        Err(BAD_MINUTES.into())
    } else if minutes > MAX_MINUTES {
        Err(TOO_LONG.into())
    } else {
        Ok(minutes)
    }
}

fn check_kcal(kcal: Option<f64>) -> Result<Option<f64>, String> {
    match kcal {
        Some(k) if !k.is_finite() || !(0.0..=MAX_KCAL).contains(&k) => Err(BAD_KCAL.into()),
        k => Ok(k),
    }
}

/// The calories an activity gives for `minutes` at `weight_kg`; None for a MET activity with no weight (unknown, never 0).
fn estimate_kcal(basis: &ExerciseBasis, weight_kg: Option<f64>, minutes: f64) -> Option<f64> {
    match basis {
        ExerciseBasis::Met(met) => weight_kg.map(|kg| net_kcal(*met, kg, minutes)),
        ExerciseBasis::KcalPerHour(rate) => Some(rate * minutes / 60.0),
    }
}

fn resolve(table: &[Met], j: &mut Journal, r: &ActivityRef) -> Result<(String, ExerciseSource, ExerciseBasis), String> {
    match r {
        ActivityRef::Custom(uid) => {
            let a = j.profile()?.activities.iter().find(|a| a.uid == *uid).ok_or(NO_CUSTOM)?;
            Ok((a.name.clone(), ExerciseSource::Custom(uid.clone()), a.basis.clone()))
        }
        ActivityRef::Compendium(id) => compendium(table, id),
    }
}

/// What an activity would give for a duration, with the working shown. Nothing is written.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Estimate {
    pub kcal: Option<i32>,
    /// "288", or "–" when the weight isn't known.
    pub kcal_text: String,
    /// "5.8 MET − 1 resting = 4.8, 80.0 kg, 45 min"
    pub working: Option<String>,
    /// "Using your latest weigh-in, 80.0 kg (Nov 17)."
    pub weight_text: Option<String>,
    pub note: Option<&'static str>,
}

pub fn estimate(j: &mut Journal, day: &str, r: &ActivityRef, minutes: f64) -> Result<Estimate, String> {
    let d = Day::parse(day)?;
    check_minutes(minutes)?;
    let (_, _, basis) = resolve(table(), j, r)?;
    let unit = j.profile().map(|p| p.units.weight).unwrap_or_default();
    let mins = trim(minutes);
    Ok(match basis {
        ExerciseBasis::KcalPerHour(rate) => {
            let k = (rate * minutes / 60.0).round() as i32;
            Estimate { kcal: Some(k), kcal_text: kcal_text(k), working: Some(format!("{} kcal an hour, {mins} min. Counted as net.", trim(rate))), weight_text: None, note: None }
        }
        ExerciseBasis::Met(met) => match body::weight_basis(j, d.as_str()) {
            None => Estimate { kcal: None, kcal_text: "–".into(), working: None, weight_text: None, note: Some(NO_WEIGHT_NOTE) },
            Some(w) => {
                let k = net_kcal(met, w.kg, minutes).round() as i32;
                let kg = goals::mass_text(w.kg, unit);
                let weight_text = if w.trend { format!("Using your trend weight, {kg}.") } else { format!("Using your latest weigh-in, {kg} ({}).", goals::date_text(&w.day)) };
                Estimate { kcal: Some(k), kcal_text: kcal_text(k), working: Some(format!("{} MET \u{2212} 1 resting = {}, {kg}, {mins} min", met_str(met), trim((met - 1.0).max(0.0)))), weight_text: Some(weight_text), note: None }
            }
        },
    })
}

/// Log `minutes` of an activity on `day`. The name, source, MET, weight and estimate are frozen into the entry (D-17b); a MET entry with
/// no weight on record keeps an unknown estimate, which a typed `kcal_override` can still give calories. Returns the activity's name.
pub fn log(j: &mut Journal, day: &str, r: &ActivityRef, minutes: f64, kcal_override: Option<f64>, now: u64) -> Result<String, String> {
    log_in(table(), j, day, r, minutes, kcal_override, now)
}

/// [`log`] against a given table, so the public stand-in (no rows) is testable beside the full one.
fn log_in(table: &[Met], j: &mut Journal, day: &str, r: &ActivityRef, minutes: f64, kcal_override: Option<f64>, now: u64) -> Result<String, String> {
    let d = Day::parse(day)?;
    check_minutes(minutes)?;
    let kcal_override = check_kcal(kcal_override)?;
    let (name, source, basis) = resolve(table, j, r)?;
    let weight_kg = match basis {
        ExerciseBasis::Met(_) => body::weight_basis(j, d.as_str()).map(|w| w.kg),
        ExerciseBasis::KcalPerHour(_) => None,
    };
    let est_kcal = estimate_kcal(&basis, weight_kg, minutes);
    let e = ExerciseEntry { uid: new_uid(), day: d.as_str().into(), logged_at: now, modified: now, deleted_at: None, name: name.clone(), source, basis, minutes, weight_kg, est_kcal, kcal_override };
    j.month_mut(d.month())?.exercise.push(e);
    Ok(name)
}

/// Save one of the person's own activities in profile.json, never in the activity table (D-15). A missing uid makes a new one; a known uid is replaced.
/// Returns the uid.
pub fn save_activity(j: &mut Journal, mut a: CustomActivity) -> Result<String, String> {
    a.name = a.name.trim().chars().take(NAME_CHARS).collect::<String>().trim().to_string();
    if a.name.is_empty() {
        return Err("Give the activity a name.".into());
    }
    match a.basis {
        ExerciseBasis::Met(v) if !v.is_finite() || v <= 0.0 || v > MAX_MET => return Err("Enter a MET value above 0 and up to 30.".into()),
        ExerciseBasis::KcalPerHour(v) if !v.is_finite() || v <= 0.0 || v > MAX_KCAL_PER_HOUR => return Err("Enter calories per hour above 0 and up to 3,000.".into()),
        _ => {}
    }
    let list = &mut j.profile_mut()?.activities;
    if a.uid.is_empty() {
        a.uid = new_uid();
    }
    let uid = a.uid.clone();
    match list.iter().position(|x| x.uid == uid) {
        Some(i) => list[i] = a,
        None if list.len() >= MAX_ACTIVITIES => return Err(format!("You can keep up to {MAX_ACTIVITIES} activities. Delete one to add another.")),
        None => list.push(a),
    }
    Ok(uid)
}

/// A live (not deleted) workout of `day` by uid, for changing. A miss after `month_mut` is harmless: the shell's `mutate_journal` discards unsaved changes on any error.
fn live_exercise<'a>(j: &'a mut Journal, d: &Day, uid: &str) -> Result<&'a mut ExerciseEntry, String> {
    j.month_mut(d.month())?.exercise.iter_mut().find(|e| e.uid == uid && e.deleted_at.is_none() && e.day == d.as_str()).ok_or_else(|| NOT_THERE.to_string())
}

/// Change a workout's duration and calories. The estimate is worked again from the entry's own frozen MET and weight, never from today's
/// weight; the person's own number (`kcal`) sticks until `use_estimate` clears it (D-17a, D-17b). Returns the name and the undo.
pub fn edit(j: &mut Journal, day: &str, uid: &str, minutes: f64, kcal: Option<f64>, use_estimate: bool, now: u64) -> Result<(String, DiaryUndo), String> {
    let d = Day::parse(day)?;
    check_minutes(minutes)?;
    let kcal = check_kcal(kcal)?;
    let e = live_exercise(j, &d, uid)?;
    let before = e.clone();
    e.minutes = minutes;
    e.est_kcal = estimate_kcal(&e.basis, e.weight_kg, minutes);
    if use_estimate {
        e.kcal_override = None;
    } else if kcal.is_some() {
        e.kcal_override = kcal;
    }
    e.modified = now;
    Ok((before.name.clone(), DiaryUndo { day: d.as_str().into(), before_exercise: vec![before], ..Default::default() }))
}

/// Delete a workout the soft way: it stays in the month file with `deleted_at` set. Returns its name and the undo.
pub fn delete(j: &mut Journal, day: &str, uid: &str, now: u64) -> Result<(String, DiaryUndo), String> {
    let d = Day::parse(day)?;
    let e = live_exercise(j, &d, uid)?;
    let before = e.clone();
    e.deleted_at = Some(now);
    e.modified = now;
    Ok((before.name.clone(), DiaryUndo { day: d.as_str().into(), before_exercise: vec![before], ..Default::default() }))
}

/// Remove one of the person's own activities. Workouts already logged with it are not touched. Returns its name.
pub fn delete_activity(j: &mut Journal, uid: &str) -> Result<String, String> {
    let list = &mut j.profile_mut()?.activities;
    let i = list.iter().position(|a| a.uid == uid).ok_or(NO_CUSTOM)?;
    Ok(list.remove(i).name)
}

// ---- the compiled-in activity table ----

/// The 2024 Adult Compendium table 04-09 builds (`app/tools/build_mets.py`).
const MET_TABLE: &str = include_str!("../../data/mets.tsv");
/// The first words of the line a header-only stand-in carries. The public release snapshot compiles the stand-in until the
/// maintainers allow the rows to ship (RESEARCH Open Question 1), so an empty table is a normal case, not an error.
/// The About screen's credits and the tests tell the two apart.
const STAND_IN: &str = "# Stand-in:";

#[derive(Clone, Copy, PartialEq, Debug)]
enum Shown {
    /// Offered before anything is typed.
    Common,
    /// Found only by typing.
    Search,
    /// Never listed (the Inactivity rows).
    Hidden,
}

/// One row of the table. `met_text` is the value as printed in the file, so "8.00" stays "8.00".
#[derive(Clone, Debug)]
struct Met {
    id: String,
    heading: String,
    met: f64,
    met_text: String,
    description: String,
    shown: Shown,
}

/// One table row; None for a `#` line, a blank, a short row or the column-heading line.
fn parse_row(line: &str) -> Option<Met> {
    if line.starts_with('#') {
        return None;
    }
    let f: Vec<&str> = line.trim_end_matches(['\r', '\n']).split('\t').collect();
    if f.len() < 6 || f[0].trim().is_empty() {
        return None;
    }
    // the column-heading line fails here: "met" is not a number
    let met: f64 = f[3].trim().parse().ok().filter(|m: &f64| m.is_finite() && *m > 0.0)?;
    let shown = match f[5].trim() {
        "common" => Shown::Common,
        "search" => Shown::Search,
        "hidden" => Shown::Hidden,
        _ => return None,
    };
    Some(Met { id: f[0].trim().into(), heading: f[2].trim().into(), met, met_text: f[3].trim().into(), description: f[4].trim().into(), shown })
}

fn parse_table(text: &str) -> Vec<Met> {
    text.lines().filter_map(parse_row).collect()
}

fn table() -> &'static [Met] {
    static TABLE: OnceLock<Vec<Met>> = OnceLock::new();
    TABLE.get_or_init(|| parse_table(MET_TABLE))
}

/// True when the compiled table holds the Compendium rows rather than the stand-in.
pub fn full_table() -> bool {
    !MET_TABLE.lines().any(|l| l.starts_with(STAND_IN))
}

/// The table row `id` names, for an activity a person may log (a hidden row is not one).
fn compendium(table: &[Met], id: &str) -> Result<(String, ExerciseSource, ExerciseBasis), String> {
    let m = table.iter().find(|m| m.id == id && m.shown != Shown::Hidden).ok_or(NO_ACTIVITY)?;
    Ok((m.description.clone(), ExerciseSource::Compendium(m.id.clone()), ExerciseBasis::Met(m.met)))
}

/// One activity in a picker list.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ActivityRow {
    pub key: ActivityRef,
    pub text: String,
    pub heading: String,
    /// "MET 14.0" as the table prints it, or "600 kcal an hour".
    pub met_text: String,
    pub tag: &'static str,
    pub custom: bool,
    /// A custom activity's own value, so Edit Activity can open it; None for a table row.
    pub basis: Option<ExerciseBasis>,
}

/// The common rows of one Compendium heading.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CommonGroup {
    pub heading: String,
    pub rows: Vec<ActivityRow>,
}

/// What the picker lists. Before anything is typed: `recent`, `mine` and `common`. Typing: `rows` alone, ranked.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Activities {
    pub recent: Vec<ActivityRow>,
    pub mine: Vec<ActivityRow>,
    pub common: Vec<CommonGroup>,
    pub rows: Vec<ActivityRow>,
}

pub fn activities(j: &mut Journal, q: &str, today: &str) -> Result<Activities, String> {
    activities_in(table(), j, q, today)
}

/// Distinct recent activities listed before typing.
const RECENT_ROWS: usize = 8;
/// Rows a typed search returns, so one letter doesn't send the whole table.
const SEARCH_ROWS: usize = 100;

fn met_row(m: &Met) -> ActivityRow {
    ActivityRow { key: ActivityRef::Compendium(m.id.clone()), text: m.description.clone(), heading: m.heading.clone(), met_text: format!("MET {}", m.met_text), tag: "Compendium", custom: false, basis: None }
}

fn custom_row(a: &CustomActivity) -> ActivityRow {
    let met_text = match a.basis {
        ExerciseBasis::Met(m) => format!("MET {}", met_str(m)),
        ExerciseBasis::KcalPerHour(k) => format!("{} kcal an hour", trim(k)),
    };
    ActivityRow { key: ActivityRef::Custom(a.uid.clone()), text: a.name.clone(), heading: "My Activities".into(), met_text, tag: "Custom", custom: true, basis: Some(a.basis.clone()) }
}

/// Every distinct activity logged in the last 60 days up to `today`, newest first, as rows. An activity that no longer resolves
/// (a custom one deleted, a table without its row) is left out. A month that won't open is skipped.
fn recent_rows(table: &[Met], j: &mut Journal, own: &[CustomActivity], today: &str) -> Result<Vec<ActivityRow>, String> {
    let end = Day::parse(today)?;
    let start = Day::parse(&diary::shift(&end, 1 - diary::HISTORY_DAYS))?;
    let mut months: Vec<String> = vec![];
    for i in 0..diary::HISTORY_DAYS {
        let m = diary::shift(&start, i)[..7].to_string();
        if !months.contains(&m) {
            months.push(m);
        }
    }
    let mut seen: Vec<(u64, usize, ActivityRef)> = vec![];
    for m in &months {
        let Ok(month) = j.month(m) else { continue };
        for e in month.exercise.iter().filter(|e| e.deleted_at.is_none() && e.day.as_str() >= start.as_str() && e.day.as_str() <= end.as_str()) {
            let key = match &e.source {
                ExerciseSource::Compendium(id) => ActivityRef::Compendium(id.clone()),
                ExerciseSource::Custom(uid) => ActivityRef::Custom(uid.clone()),
                ExerciseSource::Workout(_) => continue,
            };
            seen.push((e.logged_at, seen.len(), key));
        }
    }
    seen.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    let mut out: Vec<ActivityRow> = vec![];
    for (_, _, key) in seen {
        if out.iter().any(|r| r.key == key) {
            continue;
        }
        let row = match &key {
            ActivityRef::Compendium(id) => table.iter().find(|m| m.id == *id && m.shown != Shown::Hidden).map(met_row),
            ActivityRef::Custom(uid) => own.iter().find(|a| a.uid == *uid).map(custom_row),
        };
        out.extend(row);
    }
    Ok(out)
}

/// The picker's rows. With nothing typed: Recent, My Activities (alphabetical) and the common rows grouped by heading in file order.
/// With words typed: one list of every non-hidden row and custom activity holding every word in its text or heading, a description
/// that starts with the words first, then a recent activity, then the person's own, then table order.
fn activities_in(table: &[Met], j: &mut Journal, q: &str, today: &str) -> Result<Activities, String> {
    let own: Vec<CustomActivity> = j.profile()?.activities.clone();
    let recent = recent_rows(table, j, &own, today)?;
    let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        let mut mine: Vec<&CustomActivity> = own.iter().collect();
        mine.sort_by_key(|a| a.name.to_lowercase());
        let mut common: Vec<CommonGroup> = vec![];
        for m in table.iter().filter(|m| m.shown == Shown::Common) {
            match common.iter_mut().find(|g| g.heading == m.heading) {
                Some(g) => g.rows.push(met_row(m)),
                None => common.push(CommonGroup { heading: m.heading.clone(), rows: vec![met_row(m)] }),
            }
        }
        return Ok(Activities { recent: recent.into_iter().take(RECENT_ROWS).collect(), mine: mine.into_iter().map(custom_row).collect(), common, rows: vec![] });
    }
    let typed = words.join(" ");
    // the person's own activities come ahead of the table in the base order
    let candidates = own.iter().map(custom_row).chain(table.iter().filter(|m| m.shown != Shown::Hidden).map(met_row));
    let mut found: Vec<(bool, usize, ActivityRow)> = vec![];
    for row in candidates {
        let (text, heading) = (row.text.to_lowercase(), if row.custom { String::new() } else { row.heading.to_lowercase() });
        if words.iter().all(|w| text.contains(w.as_str()) || heading.contains(w.as_str())) {
            let rank = recent.iter().position(|r| r.key == row.key).unwrap_or(usize::MAX);
            found.push((!text.starts_with(&typed), rank, row));
        }
    }
    found.sort_by_key(|f| (f.0, f.1)); // stable: base order breaks the rest
    Ok(Activities { rows: found.into_iter().take(SEARCH_ROWS).map(|f| f.2).collect(), ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::journal::BodyEntry;
    use std::fs;
    use std::path::PathBuf;

    pub(crate) fn tdir(name: &str) -> PathBuf {
        let d = crate::scratch_dir().join(format!("rb-exercise-{}-{}", name, crate::test_pid()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    pub(crate) fn weigh(j: &mut Journal, day: &str, kg: f64) {
        j.month_mut(&day[..7]).unwrap().body.push(BodyEntry { uid: new_uid(), day: day.into(), logged_at: 1, modified: 1, measure: "weight".into(), value: kg, ..Default::default() });
    }

    pub(crate) fn mine(j: &mut Journal, name: &str, basis: ExerciseBasis) -> ActivityRef {
        ActivityRef::Custom(save_activity(j, CustomActivity { uid: String::new(), name: name.into(), basis }).unwrap())
    }

    fn entries(j: &mut Journal, month: &str) -> Vec<ExerciseEntry> {
        j.month(month).unwrap().exercise.clone()
    }

    #[test]
    fn net_formula() {
        assert!((net_kcal(7.5, 82.4, 45.0) - 401.7).abs() < 0.01);
        assert_eq!(net_kcal(1.0, 80.0, 60.0), 0.0);
        assert_eq!(net_kcal(0.9, 80.0, 60.0), 0.0);
    }

    #[test]
    fn log_freezes_weight_and_estimate() {
        let d = tdir("freeze");
        let mut j = Journal::open(d.clone());
        let r = mine(&mut j, "Bouldering", ExerciseBasis::Met(5.8));
        weigh(&mut j, "2026-11-17", 80.0);
        let name = log(&mut j, "2026-11-17", &r, 45.0, None, 100).unwrap();
        assert_eq!(name, "Bouldering");
        weigh(&mut j, "2026-11-18", 70.0);
        let e = entries(&mut j, "2026-11").remove(0);
        assert_eq!((e.weight_kg, e.kcal_override, e.minutes), (Some(80.0), None, 45.0));
        assert!((e.est_kcal.unwrap() - 288.0).abs() < 0.001, "{:?}", e.est_kcal);
        assert_eq!(e.basis, ExerciseBasis::Met(5.8));
        assert!(matches!(e.source, ExerciseSource::Custom(ref u) if !u.is_empty()));
        // a later weigh-in and a later day never move it
        assert!((entries(&mut j, "2026-11")[0].est_kcal.unwrap() - 288.0).abs() < 0.001);
        let est = estimate(&mut j, "2026-11-17", &r, 45.0).unwrap();
        assert_eq!((est.kcal, est.kcal_text.as_str()), (Some(288), "288"));
        assert_eq!(est.working.as_deref(), Some("5.8 MET \u{2212} 1 resting = 4.8, 80.0 kg, 45 min"));
        assert_eq!(est.weight_text.as_deref(), Some("Using your latest weigh-in, 80.0 kg (Nov 17)."));
        assert_eq!(entries(&mut j, "2026-11").len(), 1, "estimate writes nothing");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unknown_weight_is_not_zero() {
        let d = tdir("noweight");
        let mut j = Journal::open(d.clone());
        let met = mine(&mut j, "Bouldering", ExerciseBasis::Met(5.8));
        let rate = mine(&mut j, "Rowing machine", ExerciseBasis::KcalPerHour(600.0));
        log(&mut j, "2026-11-17", &met, 45.0, None, 1).unwrap();
        log(&mut j, "2026-11-17", &rate, 30.0, None, 2).unwrap();
        log(&mut j, "2026-11-17", &met, 45.0, Some(250.0), 3).unwrap();
        let es = entries(&mut j, "2026-11");
        assert_eq!((es[0].weight_kg, es[0].est_kcal), (None, None));
        assert_eq!(es[1].est_kcal, Some(300.0));
        assert_eq!((es[2].est_kcal, es[2].kcal_override), (None, Some(250.0)));
        let est = estimate(&mut j, "2026-11-17", &met, 45.0).unwrap();
        assert_eq!((est.kcal, est.kcal_text.as_str(), est.note), (None, "–", Some(NO_WEIGHT_NOTE)));
        assert_eq!(estimate(&mut j, "2026-11-17", &rate, 30.0).unwrap().kcal, Some(300));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn custom_activity_rules() {
        let d = tdir("custom");
        let mut j = Journal::open(d.clone());
        let a = |name: &str, basis| CustomActivity { uid: String::new(), name: name.into(), basis };
        assert_eq!(save_activity(&mut j, a("  ", ExerciseBasis::Met(5.0))).unwrap_err(), "Give the activity a name.");
        for bad in [0.0, -1.0, f64::NAN, 30.5] {
            assert!(save_activity(&mut j, a("X", ExerciseBasis::Met(bad))).is_err(), "MET {bad}");
        }
        assert!(save_activity(&mut j, a("X", ExerciseBasis::KcalPerHour(3001.0))).is_err());
        assert!(save_activity(&mut j, a("X", ExerciseBasis::KcalPerHour(0.0))).is_err());
        let uid = save_activity(&mut j, a("Bouldering", ExerciseBasis::Met(5.8))).unwrap();
        assert_eq!(j.profile().unwrap().activities.len(), 1);
        // the same uid replaces, and a long name is cut
        let again = save_activity(&mut j, CustomActivity { uid: uid.clone(), name: "y".repeat(80), basis: ExerciseBasis::Met(6.0) }).unwrap();
        assert_eq!(again, uid);
        let p = j.profile().unwrap();
        assert_eq!((p.activities.len(), p.activities[0].name.chars().count(), p.activities[0].basis.clone()), (1, 60, ExerciseBasis::Met(6.0)));
        for i in 1..200 {
            save_activity(&mut j, a(&format!("A{i}"), ExerciseBasis::Met(3.0))).unwrap();
        }
        assert!(save_activity(&mut j, a("one too many", ExerciseBasis::Met(3.0))).is_err());
        // log refuses what a person can't have done
        let r = ActivityRef::Custom(uid);
        for bad in [0.0, -5.0, f64::NAN, 1441.0] {
            assert!(log(&mut j, "2026-11-17", &r, bad, None, 1).is_err(), "{bad} minutes");
        }
        assert!(log(&mut j, "2026-11-17", &r, 30.0, Some(-1.0), 1).is_err());
        assert!(log(&mut j, "2026-11-17", &r, 30.0, Some(f64::NAN), 1).is_err());
        assert!(log(&mut j, "2026-02-30", &r, 30.0, None, 1).unwrap_err().contains("isn't a valid date"));
        assert!(log(&mut j, "2026-11-17", &ActivityRef::Custom("nope".into()), 30.0, None, 1).is_err());
        assert!(log(&mut j, "2031-01-05", &r, 30.0, None, 1).is_ok(), "planning ahead is allowed");
        let _ = fs::remove_dir_all(&d);
    }

    // ---- the compiled-in table, the picker and the public stand-in ----

    /// A few rows in the file's own shape: a repeated heading, a value printed "8.00", and a hidden row.
    const SAMPLE: &str = "# 2024 Adult Compendium sample\n# Rows: 6\nid\tcode\theading\tmet\tdescription\tshown\n\
01009\t01009\tBicycling\t8.5\tBicycling, mountain, general\tcommon\n\
01003\t01003\tBicycling\t14.0\tBicycling, mountain, uphill, vigorous\tsearch\n\
12020\t12020\tRunning\t7.5\tJogging, general, self-selected pace\tcommon\n\
12150\t12150\tRunning\t8.00\tRunning, 5 mph (12 min/mile)\tcommon\n\
12040\t12040\tRunning\t6.0\tJogging in place\tsearch\n\
07010\t07010\tInactivity\t1.0\tLying quietly and watching television\thidden\n";

    /// The header-only file the public release snapshot compiles (build_mets.py --stand-in).
    const STAND_IN_TEXT: &str = "# 2024 Adult Compendium of Physical Activities, Herrmann SD et al.\n# MET values unaltered; activities are not combined.\n# Permission to embed: asked of the maintainers, reply pending.\n# Stand-in: the cited Compendium rows are left out of public builds until the maintainers reply. Custom activities still work.\nid\tcode\theading\tmet\tdescription\tshown\n";

    fn ids(rows: &[ActivityRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match &r.key {
                ActivityRef::Compendium(id) => id.clone(),
                ActivityRef::Custom(_) => format!("custom:{}", r.text),
            })
            .collect()
    }

    fn group_ids(a: &Activities) -> Vec<(String, Vec<String>)> {
        a.common.iter().map(|g| (g.heading.clone(), ids(&g.rows))).collect()
    }

    fn put(j: &mut Journal, t: &[Met], r: &ActivityRef, at: u64) {
        log_in(t, j, "2026-11-17", r, 30.0, Some(100.0), at).unwrap();
    }

    #[test]
    fn table_and_spot_checks() {
        let t = parse_table(SAMPLE);
        assert_eq!(t.len(), 6);
        let run = t.iter().find(|m| m.id == "12150").unwrap();
        assert_eq!((run.met, run.met_text.as_str(), run.heading.as_str(), run.shown), (8.0, "8.00", "Running", Shown::Common));
        assert_eq!(t.iter().filter(|m| m.shown == Shown::Hidden).count(), 1);
        // short, blank, comment, column-heading and unreadable rows are skipped, never a panic
        assert!(parse_table("# c\n\nid\tcode\theading\tmet\tdescription\tshown\nx\ty\n9\t9\tH\tabc\tD\tcommon\n9\t9\tH\t5.0\tD\tsometimes\n\t9\tH\t5.0\tD\tcommon\n").is_empty());

        // the file compiled into this build
        let t = table();
        if !full_table() {
            assert!(t.is_empty(), "the public stand-in compiles with no rows (RESEARCH Open Question 1)");
            return;
        }
        assert_eq!(t.len(), 1111);
        let mut seen: Vec<&str> = t.iter().map(|m| m.id.as_str()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), t.len(), "ids are unique");
        let by = |id: &str| t.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("no row {id}"));
        let (a, b, c) = (by("01003"), by("02050"), by("12020"));
        assert_eq!((a.heading.as_str(), a.met, a.description.as_str()), ("Bicycling", 14.0, "Bicycling, mountain, uphill, vigorous"));
        assert_eq!((b.met, c.met), (6.0, 7.5));
        let common = t.iter().filter(|m| m.shown == Shown::Common).count();
        assert!((100..=140).contains(&common), "{common} common rows");
        let hidden: Vec<&Met> = t.iter().filter(|m| m.shown == Shown::Hidden).collect();
        assert_eq!(hidden.len(), 17);
        assert!(hidden.iter().all(|m| m.heading == "Inactivity"));
        assert_eq!((by("12355").met, by("12355-2").met), (16.0, 9.0), "a repeated code is two rows");
    }

    #[test]
    fn stand_in_table_is_empty_and_refuses_compendium() {
        let t = parse_table(STAND_IN_TEXT);
        assert!(t.is_empty());
        assert_eq!(compendium(&t, "12020").unwrap_err(), "That activity isn't in the list.");
        assert_eq!(compendium(&t, "").unwrap_err(), NO_ACTIVITY);

        // the picker and the logging still work on custom activities
        let d = tdir("stand-in");
        let mut j = Journal::open(d.clone());
        let r = mine(&mut j, "Bouldering", ExerciseBasis::Met(5.8));
        weigh(&mut j, "2026-11-17", 80.0);
        assert_eq!(log_in(&t, &mut j, "2026-11-17", &ActivityRef::Compendium("12020".into()), 30.0, None, 1).unwrap_err(), NO_ACTIVITY);
        assert_eq!(log_in(&t, &mut j, "2026-11-17", &r, 30.0, None, 2).unwrap(), "Bouldering");
        let a = activities_in(&t, &mut j, "", "2026-11-17").unwrap();
        assert_eq!((ids(&a.recent), ids(&a.mine), a.common.len(), a.rows.len()), (vec!["custom:Bouldering".to_string()], vec!["custom:Bouldering".to_string()], 0, 0));
        assert_eq!(ids(&activities_in(&t, &mut j, "boulder", "2026-11-17").unwrap().rows), ["custom:Bouldering"]);
        assert!(activities_in(&t, &mut j, "jog", "2026-11-17").unwrap().rows.is_empty());
        // an entry logged earlier from the full table keeps its name and calories, and just drops out of Recent
        let mut old = entries(&mut j, "2026-11").remove(0);
        old.uid = "old".into();
        old.logged_at = 99;
        old.name = "Jogging, general, self-selected pace".into();
        old.source = ExerciseSource::Compendium("12020".into());
        old.basis = ExerciseBasis::Met(7.5);
        j.month_mut("2026-11").unwrap().exercise.push(old);
        let a = activities_in(&t, &mut j, "", "2026-11-17").unwrap();
        assert_eq!(ids(&a.recent), ["custom:Bouldering"]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hidden_rows_never_listed() {
        let t = parse_table(SAMPLE);
        let d = tdir("hidden");
        let mut j = Journal::open(d.clone());
        for q in ["", "lying", "television", "inactivity", "a", "e"] {
            let a = activities_in(&t, &mut j, q, "2026-11-17").unwrap();
            let every: Vec<String> = ids(&a.rows).into_iter().chain(group_ids(&a).into_iter().flat_map(|g| g.1)).collect();
            assert!(!every.contains(&"07010".to_string()), "query {q:?}");
        }
        assert_eq!(compendium(&t, "07010").unwrap_err(), NO_ACTIVITY, "a hidden row can't be logged");
        if full_table() {
            let hidden: Vec<&str> = table().iter().filter(|m| m.shown == Shown::Hidden).map(|m| m.id.as_str()).collect();
            for q in ["inactivity", "sleeping", "lying", "sitting", "quiet", "a"] {
                let a = activities(&mut j, q, "2026-11-17").unwrap();
                assert!(ids(&a.rows).iter().all(|id| !hidden.contains(&id.as_str())), "query {q:?}");
            }
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn activities_before_typing() {
        let t = parse_table(SAMPLE);
        let d = tdir("before-typing");
        let mut j = Journal::open(d.clone());
        let a = activities_in(&t, &mut j, "  ", "2026-11-17").unwrap();
        assert!(a.recent.is_empty() && a.mine.is_empty() && a.rows.is_empty());
        assert_eq!(group_ids(&a), [("Bicycling".to_string(), vec!["01009".to_string()]), ("Running".to_string(), vec!["12020".to_string(), "12150".to_string()])]);
        let r = &a.common[1].rows[1];
        assert_eq!((r.text.as_str(), r.heading.as_str(), r.met_text.as_str(), r.tag, r.custom), ("Running, 5 mph (12 min/mile)", "Running", "MET 8.00", "Compendium", false));

        let zumba = mine(&mut j, "Zumba", ExerciseBasis::KcalPerHour(500.0));
        mine(&mut j, "aerobics", ExerciseBasis::Met(6.0));
        put(&mut j, &t, &ActivityRef::Compendium("12020".into()), 10);
        put(&mut j, &t, &ActivityRef::Compendium("01009".into()), 20);
        put(&mut j, &t, &zumba, 30);
        put(&mut j, &t, &ActivityRef::Compendium("12020".into()), 40);
        let a = activities_in(&t, &mut j, "", "2026-11-17").unwrap();
        assert_eq!(ids(&a.recent), ["12020", "custom:Zumba", "01009"], "newest first, each activity once");
        assert_eq!(ids(&a.mine), ["custom:aerobics", "custom:Zumba"], "alphabetical, ignoring case");
        assert_eq!((a.mine[0].met_text.as_str(), a.mine[1].met_text.as_str(), a.mine[0].tag, a.mine[0].custom), ("MET 6.0", "500 kcal an hour", "Custom", true));
        assert_eq!((a.mine[0].basis.clone(), a.mine[1].basis.clone(), a.common[0].rows[0].basis.clone()), (Some(ExerciseBasis::Met(6.0)), Some(ExerciseBasis::KcalPerHour(500.0)), None), "a custom row carries its value so it can be edited");

        // at most 8, and only the last 60 days up to today
        for i in 0..10 {
            let r = mine(&mut j, &format!("Own {i}"), ExerciseBasis::Met(3.0));
            put(&mut j, &t, &r, 100 + i);
        }
        assert_eq!(activities_in(&t, &mut j, "", "2026-11-17").unwrap().recent.len(), 8);
        assert!(activities_in(&t, &mut j, "", "2027-03-01").unwrap().recent.is_empty(), "nothing in the 60 days before 2027-03-01");
        assert!(activities_in(&t, &mut j, "", "2026-11-10").unwrap().recent.is_empty(), "a day after today is not recent");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn search_ranks_recent_first() {
        let t = parse_table(SAMPLE);
        let d = tdir("search");
        let mut j = Journal::open(d.clone());
        let q = |j: &mut Journal, q: &str| ids(&activities_in(&t, j, q, "2026-11-17").unwrap().rows);
        assert_eq!(q(&mut j, "jog"), ["12020", "12040"], "equal matches keep table order");
        assert_eq!(q(&mut j, "JOG"), ["12020", "12040"], "case doesn't matter");
        put(&mut j, &t, &ActivityRef::Compendium("12040".into()), 5);
        assert_eq!(q(&mut j, "jog"), ["12040", "12020"], "a recent activity wins among equal matches");
        // a description that starts with the words beats one that only has them in the heading
        assert_eq!(q(&mut j, "running"), ["12150", "12040", "12020"]);
        // every typed word has to match, in the description or the heading
        assert_eq!(q(&mut j, "mountain general"), ["01009"]);
        assert_eq!(q(&mut j, "bicycling vigorous"), ["01003"]);
        assert!(q(&mut j, "zzz").is_empty());
        // the person's own activities are searched too
        mine(&mut j, "Jogging with the dog", ExerciseBasis::Met(5.0));
        let rows = q(&mut j, "jog");
        assert_eq!((rows.len(), rows[0].as_str()), (3, "12040"));
        assert!(rows.contains(&"custom:Jogging with the dog".to_string()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn compendium_log_freezes_the_row() {
        let t = parse_table(SAMPLE);
        let d = tdir("compendium-log");
        let mut j = Journal::open(d.clone());
        weigh(&mut j, "2026-11-17", 80.0);
        let name = log_in(&t, &mut j, "2026-11-17", &ActivityRef::Compendium("12020".into()), 30.0, None, 7).unwrap();
        assert_eq!(name, "Jogging, general, self-selected pace");
        let e = entries(&mut j, "2026-11").remove(0);
        assert_eq!((e.name.as_str(), e.source.clone(), e.basis.clone(), e.weight_kg), ("Jogging, general, self-selected pace", ExerciseSource::Compendium("12020".into()), ExerciseBasis::Met(7.5), Some(80.0)));
        assert!((e.est_kcal.unwrap() - 260.0).abs() < 0.001);
        assert_eq!(log_in(&t, &mut j, "2026-11-17", &ActivityRef::Compendium("99999".into()), 30.0, None, 8).unwrap_err(), NO_ACTIVITY);
        assert_eq!(entries(&mut j, "2026-11").len(), 1);
        let r = ActivityRef::Compendium("12020".into());
        if full_table() {
            // the real table through the public functions
            assert_eq!(log(&mut j, "2026-11-17", &r, 30.0, None, 9).unwrap(), "Jogging, general, self-selected pace");
            assert_eq!(estimate(&mut j, "2026-11-17", &r, 30.0).unwrap().kcal, Some(260));
        } else {
            assert_eq!(log(&mut j, "2026-11-17", &r, 30.0, None, 9).unwrap_err(), NO_ACTIVITY);
        }
        let _ = fs::remove_dir_all(&d);
    }

    // ---- edit, delete and custom activities ----

    /// A logged 45-minute "Hill walk" (MET 7.5) at 80 kg on 2026-11-17: an estimate of 6.5 x 80 x 0.75 = 390.
    fn hill_walk(j: &mut Journal) -> (ActivityRef, String) {
        let r = mine(j, "Hill walk", ExerciseBasis::Met(7.5));
        weigh(j, "2026-11-17", 80.0);
        log(j, "2026-11-17", &r, 45.0, None, 100).unwrap();
        let uid = entries(j, "2026-11")[0].uid.clone();
        (r, uid)
    }

    fn the(j: &mut Journal, uid: &str) -> ExerciseEntry {
        entries(j, "2026-11").into_iter().find(|e| e.uid == uid).unwrap()
    }

    fn est(e: &ExerciseEntry) -> f64 {
        e.est_kcal.unwrap()
    }

    #[test]
    fn override_sticks_and_use_estimate_clears() {
        let d = tdir("override");
        let mut j = Journal::open(d.clone());
        let (_, uid) = hill_walk(&mut j);
        assert!((est(&the(&mut j, &uid)) - 390.0).abs() < 0.001);

        let (name, u) = edit(&mut j, "2026-11-17", &uid, 45.0, Some(400.0), false, 200).unwrap();
        assert_eq!(name, "Hill walk");
        let e = the(&mut j, &uid);
        assert_eq!((e.kcal_override, e.modified), (Some(400.0), 200));
        assert!((est(&e) - 390.0).abs() < 0.001, "the estimate is still the estimate");
        assert_eq!((u.before_exercise.len(), u.before_exercise[0].kcal_override), (1, None));
        // a new duration changes only the estimate; the person's number stays
        edit(&mut j, "2026-11-17", &uid, 30.0, None, false, 300).unwrap();
        let e = the(&mut j, &uid);
        assert_eq!((e.minutes, e.kcal_override), (30.0, Some(400.0)));
        assert!((est(&e) - 260.0).abs() < 0.001, "{:?}", e.est_kcal);
        // Use estimate clears it
        edit(&mut j, "2026-11-17", &uid, 30.0, Some(999.0), true, 400).unwrap();
        assert_eq!(the(&mut j, &uid).kcal_override, None);
        // one undo puts back the old minutes and the old override
        let (_, u) = edit(&mut j, "2026-11-17", &uid, 20.0, Some(50.0), false, 500).unwrap();
        diary::undo(&mut j, &u, 600).unwrap();
        let e = the(&mut j, &uid);
        assert_eq!((e.minutes, e.kcal_override, e.modified), (30.0, None, 600));

        for bad in [0.0, -1.0, f64::NAN, 1441.0] {
            assert!(edit(&mut j, "2026-11-17", &uid, bad, None, false, 1).is_err(), "{bad} minutes");
        }
        for bad in [-1.0, f64::NAN, 10_001.0] {
            assert!(edit(&mut j, "2026-11-17", &uid, 30.0, Some(bad), false, 1).is_err(), "{bad} kcal");
        }
        assert_eq!(edit(&mut j, "2026-11-17", "nope", 30.0, None, false, 1).unwrap_err(), "That entry isn't in the diary any more.");
        assert_eq!(edit(&mut j, "2026-11-18", &uid, 30.0, None, false, 1).unwrap_err(), "That entry isn't in the diary any more.", "the day has to match");
        assert_eq!(the(&mut j, &uid).minutes, 30.0, "a refused edit changed nothing");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn edit_uses_the_frozen_weight() {
        let d = tdir("frozen-edit");
        let mut j = Journal::open(d.clone());
        let (r, uid) = hill_walk(&mut j);
        // the weight on the day changes afterwards
        body::set_value(&mut j, "2026-11-17", "2026-11-17", "weight", 70.0, 5).unwrap();
        weigh(&mut j, "2026-11-18", 60.0);
        edit(&mut j, "2026-11-17", &uid, 60.0, None, false, 9).unwrap();
        let e = the(&mut j, &uid);
        assert_eq!(e.weight_kg, Some(80.0));
        assert!((est(&e) - 520.0).abs() < 0.001, "6.5 x 80 x 1 h, not today's weight: {:?}", e.est_kcal);
        // an entry logged with no weight stays unknown when it is edited after a weigh-in
        log(&mut j, "2026-11-10", &r, 30.0, None, 50).unwrap();
        let early = entries(&mut j, "2026-11").into_iter().find(|e| e.day == "2026-11-10").unwrap();
        edit(&mut j, "2026-11-10", &early.uid, 40.0, None, false, 60).unwrap();
        let e = the(&mut j, &early.uid);
        assert_eq!((e.weight_kg, e.est_kcal), (None, None));
        // a kcal-per-hour entry follows its minutes
        let rate = mine(&mut j, "Rowing", ExerciseBasis::KcalPerHour(600.0));
        log(&mut j, "2026-11-17", &rate, 30.0, None, 70).unwrap();
        let rowing = entries(&mut j, "2026-11").into_iter().find(|e| e.name == "Rowing").unwrap();
        edit(&mut j, "2026-11-17", &rowing.uid, 45.0, None, false, 80).unwrap();
        assert_eq!(the(&mut j, &rowing.uid).est_kcal, Some(450.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn exercise_delete_and_undo() {
        let d = tdir("delete");
        let mut j = Journal::open(d.clone());
        let (_, uid) = hill_walk(&mut j);
        let before = the(&mut j, &uid);
        let (name, u) = delete(&mut j, "2026-11-17", &uid, 300).unwrap();
        assert_eq!(name, "Hill walk");
        let gone = the(&mut j, &uid);
        assert_eq!((gone.deleted_at, gone.modified), (Some(300), 300), "soft delete: still in the file");
        assert_eq!(delete(&mut j, "2026-11-17", &uid, 301).unwrap_err(), "That entry isn't in the diary any more.");
        assert_eq!(delete(&mut j, "2026-11-17", "nope", 301).unwrap_err(), "That entry isn't in the diary any more.");
        diary::undo(&mut j, &u, 400).unwrap();
        let back = the(&mut j, &uid);
        assert_eq!(back, ExerciseEntry { modified: 400, ..before });
        assert_eq!(entries(&mut j, "2026-11").len(), 1, "the same entry, not a copy");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_activity_keeps_logged_entries() {
        let d = tdir("delete-activity");
        let mut j = Journal::open(d.clone());
        let (r, uid) = hill_walk(&mut j);
        let ActivityRef::Custom(act) = &r else { unreachable!() };
        let before = the(&mut j, &uid);
        assert_eq!(delete_activity(&mut j, act).unwrap(), "Hill walk");
        assert!(j.profile().unwrap().activities.is_empty());
        assert_eq!(the(&mut j, &uid), before, "what was logged keeps its name, MET and calories");
        assert_eq!(delete_activity(&mut j, act).unwrap_err(), NO_CUSTOM);
        assert_eq!(log(&mut j, "2026-11-17", &r, 30.0, None, 1).unwrap_err(), NO_CUSTOM, "it can't be logged again");
        let _ = fs::remove_dir_all(&d);
    }
}
