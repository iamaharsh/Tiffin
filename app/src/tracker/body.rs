//! Body measurements kept in the journal month files: one live value per day and measure, stored canonically
//! (kg, cm, percent or a plain number) and converted only at the edge.

use crate::model::{civil_from_days, weekday};
use crate::tracker::day::Day;
use crate::tracker::diary;
use crate::tracker::goals::{self, HeightUnit, MassUnit, UnitChoice};
use crate::tracker::journal::{BodyEntry, DiaryUndo, Journal};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What a measure's number means, which decides the values it can hold.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MeasureKind {
    /// Kilograms.
    Mass,
    /// Centimetres.
    Length,
    /// Percent of the body.
    Percent,
    /// A plain number.
    #[default]
    Number,
}

/// A measure the user added to the built-in list (D-19). It lives in the profile; `archived` hides it from new
/// entries while keeping the ones already logged.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct CustomMeasure {
    pub uid: String,
    pub name: String,
    pub kind: MeasureKind,
    pub archived: bool,
}

/// The measures every install has: id, name and kind.
pub const BUILT_IN: [(&str, &str, MeasureKind); 8] = [
    ("weight", "Weight", MeasureKind::Mass),
    ("body_fat", "Body fat", MeasureKind::Percent),
    ("waist", "Waist", MeasureKind::Length),
    ("hips", "Hips", MeasureKind::Length),
    ("chest", "Chest", MeasureKind::Length),
    ("neck", "Neck", MeasureKind::Length),
    ("arms", "Arms", MeasureKind::Length),
    ("thighs", "Thighs", MeasureKind::Length),
];

/// The most a body weight can be, in kg.
pub const MAX_MASS_KG: f64 = 500.0;
/// The most a length can be, in cm.
pub const MAX_LENGTH_CM: f64 = 400.0;

pub const BAD_MASS: &str = "That weight isn't possible. Enter a number above 0 and up to 500 kg.";
pub const BAD_LENGTH: &str = "That measurement isn't possible. Enter a number above 0 and up to 400 cm.";
pub const BAD_PERCENT: &str = "That percentage isn't possible. Enter a number above 0 and below 100.";
pub const BAD_NUMBER: &str = "That number isn't possible. Enter a number above 0.";
pub const FUTURE_DAY: &str = "A body measurement can't be logged for a future day.";
pub const NO_MEASURE: &str = "That measure isn't in your list any more.";

/// Check one value against its kind; Err is the sentence to show. NaN and infinity are never a value.
pub fn check_value(kind: MeasureKind, v: f64) -> Result<(), &'static str> {
    let ok = v.is_finite()
        && v > 0.0
        && match kind {
            MeasureKind::Mass => v <= MAX_MASS_KG,
            MeasureKind::Length => v <= MAX_LENGTH_CM,
            MeasureKind::Percent => v < 100.0,
            MeasureKind::Number => true,
        };
    if ok {
        return Ok(());
    }
    Err(match kind {
        MeasureKind::Mass => BAD_MASS,
        MeasureKind::Length => BAD_LENGTH,
        MeasureKind::Percent => BAD_PERCENT,
        MeasureKind::Number => BAD_NUMBER,
    })
}

/// What `set_value` did: the uid of the entry now holding the value and, when it replaced one, the entry as it was.
#[derive(Clone, Debug, PartialEq)]
pub struct SetBody {
    pub uid: String,
    pub before: Option<BodyEntry>,
}

/// Log one measurement for a day. The same day and measure replace the live entry in place (same uid), so
/// there is never two for one day. A future day, an unknown measure and an impossible value are refused.
pub fn set_value(j: &mut Journal, day: &str, today: &str, measure: &str, value: f64, now: u64) -> Result<SetBody, String> {
    let d = check_day(day, today)?;
    let kind = measure_kind(j, measure, false)?;
    check_value(kind, value)?;
    let body = &mut j.month_mut(d.month())?.body;
    if let Some(e) = body.iter_mut().find(|e| e.deleted_at.is_none() && e.day == day && e.measure == measure) {
        let before = e.clone();
        e.value = value;
        e.modified = now;
        return Ok(SetBody { uid: e.uid.clone(), before: Some(before) });
    }
    let uid = diary::new_uid();
    body.push(BodyEntry { uid: uid.clone(), day: day.into(), logged_at: now, modified: now, deleted_at: None, measure: measure.into(), value });
    Ok(SetBody { uid, before: None })
}

/// The weight a target is worked out from, and where it came from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WeightBasis {
    pub kg: f64,
    /// True when the number is the smoothed trend rather than one weigh-in.
    pub trend: bool,
    /// The day of the latest weigh-in on or before the day judged.
    pub day: String,
}

/// Weigh-ins it takes in the last [`TREND_WINDOW_DAYS`] days before the formula uses the trend (D-06; RESEARCH A8).
pub const TREND_MIN_WEIGH_INS: usize = 5;
/// The window those weigh-ins must fall in, ending on the day judged.
pub const TREND_WINDOW_DAYS: i32 = 28;
/// The weight the trend moves toward a new reading each day: the Hacker's Diet 10% a day.
const TREND_ALPHA_PER_DAY: f64 = 0.1;
/// A typed weight this far from the last weigh-in, or this share of it (the larger), is asked about once (D-21; RESEARCH A12).
const PLAUSIBLE_MIN_KG: f64 = 2.0;
const PLAUSIBLE_SHARE: f64 = 0.05;

/// Every live weigh-in on or before `upto` as (day number, kg, day), ascending and one per day (a second one on
/// a day can't happen through `set_value`, but a merged or restored file might hold it: the later logged wins).
/// A month that won't open is skipped.
pub(crate) fn live_weights(j: &mut Journal, upto: &str) -> Vec<(i32, f64, String)> {
    let Ok(u) = Day::parse(upto) else { return vec![] };
    let months: Vec<String> = j.months_with_data().into_iter().filter(|m| m.as_str() <= u.month()).collect();
    let mut by_day: BTreeMap<i32, (u64, f64, String)> = BTreeMap::new();
    for m in &months {
        let Ok(month) = j.month(m) else { continue };
        for e in month.body.iter().filter(|e| e.deleted_at.is_none() && e.measure == "weight" && e.day.as_str() <= u.as_str()) {
            let Ok(n) = diary::day_number(&e.day) else { continue };
            if by_day.get(&n).is_none_or(|(at, _, _)| e.logged_at > *at) {
                by_day.insert(n, (e.logged_at, e.value, e.day.clone()));
            }
        }
    }
    by_day.into_iter().map(|(n, (_, kg, day))| (n, kg, day)).collect()
}

/// Every live weigh-in on or before `upto`: (day number, kg), ascending, one per day.
pub fn weight_points(j: &mut Journal, upto: &str) -> Vec<(i32, f64)> {
    live_weights(j, upto).into_iter().map(|(n, kg, _)| (n, kg)).collect()
}

/// The smoothed weight at each point: an exponentially weighted average that moves toward each reading by 10% a
/// day (the Hacker's Diet smoothing, cited). A gap of `gap` days moves it by `1 - 0.9^gap`, so a missed week
/// counts for more than one reading (RESEARCH A8). The first value is the first weigh-in. Points come in ascending.
pub fn trend(points: &[(i32, f64)]) -> Vec<(i32, f64)> {
    let mut out: Vec<(i32, f64)> = Vec::with_capacity(points.len());
    for &(day, kg) in points {
        let t = match out.last() {
            None => kg,
            Some(&(prev_day, prev)) => prev + (1.0 - (1.0 - TREND_ALPHA_PER_DAY).powi((day - prev_day).max(1))) * (kg - prev),
        };
        out.push((day, t));
    }
    out
}

/// The weight to work from on `day`, from the body log only (D-06): the trend at the latest weigh-in once there are
/// [`TREND_MIN_WEIGH_INS`] in the 28 days ending on `day`, else the latest weigh-in itself. A weigh-in after `day`
/// never counts, so a past day keeps the weight it had.
pub fn weight_basis(j: &mut Journal, day: &str) -> Option<WeightBasis> {
    let d = Day::parse(day).ok()?;
    let points = live_weights(j, d.as_str());
    basis_from(&points, diary::day_number(d.as_str()).ok()?)
}

/// [`weight_basis`] from weigh-ins already read (as [`live_weights`] gives them), for many days at once without
/// reading the journal again. Points after `today` are ignored.
pub(crate) fn basis_from(points: &[(i32, f64, String)], today: i32) -> Option<WeightBasis> {
    let points = &points[..points.partition_point(|p| p.0 <= today)];
    let (_, latest, latest_day) = points.last()?.clone();
    let recent = points.iter().filter(|p| p.0 > today - TREND_WINDOW_DAYS).count();
    if recent >= TREND_MIN_WEIGH_INS {
        let series: Vec<(i32, f64)> = points.iter().map(|p| (p.0, p.1)).collect();
        let smoothed = trend(&series).last()?.1;
        return Some(WeightBasis { kg: smoothed, trend: true, day: latest_day });
    }
    Some(WeightBasis { kg: latest, trend: false, day: latest_day })
}

/// What the body log says about a typed weight.
#[derive(Clone, Debug, PartialEq)]
pub enum WeightCheck {
    Ok,
    /// Far from the last weigh-in: ask once, with this sentence.
    Confirm(String),
    /// Not a weight at all.
    Refused(String),
}

/// Judge a typed weight against the last weigh-in. Outside 0 to 500 kg (or not a number) is refused; a change
/// strictly greater than max(2 kg, 5% of the last weigh-in) is asked about once; anything else, including a
/// change exactly at that line, goes through. The one boundary rule for every weight the person types.
pub fn check_weight(previous_kg: Option<f64>, new_kg: f64, unit: goals::MassUnit) -> WeightCheck {
    if check_value(MeasureKind::Mass, new_kg).is_err() {
        return WeightCheck::Refused(BAD_MASS.into());
    }
    let Some(prev) = previous_kg else { return WeightCheck::Ok };
    let change = new_kg - prev;
    if change.abs() > PLAUSIBLE_MIN_KG.max(PLAUSIBLE_SHARE * prev) {
        let way = if change < 0.0 { "less" } else { "more" };
        return WeightCheck::Confirm(format!("That's {} {way} than your last weigh-in. Save anyway?", goals::mass_change_text(change, unit)));
    }
    WeightCheck::Ok
}

/// The latest live weigh-in before `day`. An entry on `day` itself is the one being replaced, so it doesn't count.
pub fn previous_weight(j: &mut Journal, day: &str) -> Option<f64> {
    let n = diary::day_number(day).ok()?;
    live_weights(j, day).into_iter().rev().find(|p| p.0 < n).map(|p| p.1)
}

// ---------- logging: units in, one undo, ask once (BODY-01, D-20, D-21) ----------

pub const BAD_UNIT: &str = "That unit doesn't fit this measure.";
/// Said under the Body headline whatever the numbers do (D-24).
pub const SWINGS_NOTE: &str = "Daily swings of a kilo or two are normal. The trend smooths them out.";
const FEW_FOR_RATE: &str = "Log a few more to see a rate.";

/// The unit a typed number is in. It must fit the measure's kind: kg or lb for a mass, cm or in for a length, none for the rest.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnitIn {
    Kg,
    Lb,
    Cm,
    In,
    None,
}

/// A typed number as the canonical value (kg, cm, percent or a plain number).
fn canonical(kind: MeasureKind, value: f64, unit: UnitIn) -> Result<f64, String> {
    match (kind, unit) {
        (MeasureKind::Mass, UnitIn::Kg) | (MeasureKind::Length, UnitIn::Cm) | (MeasureKind::Percent | MeasureKind::Number, UnitIn::None) => Ok(value),
        (MeasureKind::Mass, UnitIn::Lb) => Ok(goals::kg_from(value, MassUnit::Lb)),
        (MeasureKind::Length, UnitIn::In) => Ok(value * goals::CM_PER_IN),
        _ => Err(BAD_UNIT.into()),
    }
}

/// A valid `day` that is not after `today`.
fn check_day(day: &str, today: &str) -> Result<Day, String> {
    let d = Day::parse(day)?;
    if d.as_str() > Day::parse(today)?.as_str() {
        return Err(FUTURE_DAY.into());
    }
    Ok(d)
}

/// What a measure id means: a built-in, or a custom measure (archived ones only when `allow_archived`).
fn measure_kind(j: &mut Journal, id: &str, allow_archived: bool) -> Result<MeasureKind, String> {
    if let Some((_, _, k)) = BUILT_IN.iter().find(|(b, _, _)| *b == id) {
        return Ok(*k);
    }
    j.profile()?.measures.iter().find(|m| m.uid == id && (allow_archived || !m.archived)).map(|m| m.kind).ok_or_else(|| NO_MEASURE.to_string())
}

fn units_of(j: &mut Journal) -> UnitChoice {
    j.profile().map(|p| p.units).unwrap_or_default()
}

pub(crate) fn mass_word(u: MassUnit) -> &'static str {
    match u {
        MassUnit::Kg => "kg",
        MassUnit::Lb => "lb",
    }
}

fn unit_text(kind: MeasureKind, u: &UnitChoice) -> &'static str {
    match kind {
        MeasureKind::Mass => mass_word(u.weight),
        MeasureKind::Length if u.height == HeightUnit::FtIn => "in",
        MeasureKind::Length => "cm",
        MeasureKind::Percent => "%",
        MeasureKind::Number => "",
    }
}

/// One decimal, no trailing zero: "8000", "2.5".
pub(crate) fn trimmed1(v: f64) -> String {
    format!("{v:.1}").trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A stored value as the person reads it: rounded to 0.1 in their unit, with the unit ("78.9 kg", "84.0 cm", "22.5%").
fn value_text(kind: MeasureKind, v: f64, u: &UnitChoice) -> String {
    match kind {
        MeasureKind::Mass => goals::mass_text(v, u.weight),
        MeasureKind::Length if u.height == HeightUnit::FtIn => format!("{:.1} in", v / goals::CM_PER_IN),
        MeasureKind::Length => format!("{v:.1} cm"),
        MeasureKind::Percent => format!("{v:.1}%"),
        MeasureKind::Number => trimmed1(v),
    }
}

/// What a log or an edit did. `confirm` is the question to ask when a weight looks wrong (nothing was written);
/// `text` is what was saved, for the toast; `undo` puts it back.
#[derive(Clone, Debug, PartialEq)]
pub struct Logged {
    pub confirm: Option<String>,
    pub text: String,
    pub undo: Option<DiaryUndo>,
}

fn undo_of(day: &str, set: SetBody) -> DiaryUndo {
    let added_body = if set.before.is_none() { vec![set.uid] } else { vec![] };
    DiaryUndo { day: day.into(), before_body: set.before.into_iter().collect(), added_body, ..Default::default() }
}

/// The ask-once check for a typed weight. Err is a refusal; Some is the question to ask (when not yet `confirmed`).
fn weight_gate(j: &mut Journal, day: &str, kg: f64, confirmed: bool) -> Result<Option<String>, String> {
    let unit = units_of(j).weight;
    match check_weight(previous_weight(j, day), kg, unit) {
        WeightCheck::Ok => Ok(None),
        WeightCheck::Refused(s) => Err(s),
        WeightCheck::Confirm(s) => Ok((!confirmed).then_some(s)),
    }
}

/// Log one measurement typed in `unit` (D-20, D-21). The same day and measure replace in place. A weight far from the last
/// weigh-in comes back as `confirm` with nothing written; the same call with `confirmed` saves it. Zero, negative, above
/// the limit, not a number, a future day and a unit that doesn't fit are refused.
#[allow(clippy::too_many_arguments)]
pub fn log(j: &mut Journal, day: &str, today: &str, measure: &str, value: f64, unit: UnitIn, confirmed: bool, now: u64) -> Result<Logged, String> {
    check_day(day, today)?;
    let kind = measure_kind(j, measure, false)?;
    let v = canonical(kind, value, unit)?;
    if measure == "weight" {
        if let Some(ask) = weight_gate(j, day, v, confirmed)? {
            return Ok(Logged { confirm: Some(ask), text: String::new(), undo: None });
        }
    }
    let set = set_value(j, day, today, measure, v, now)?;
    let text = value_text(kind, v, &units_of(j));
    Ok(Logged { confirm: None, text, undo: Some(undo_of(day, set)) })
}

// ---------- editing a day, clearing a value, the person's own measures (D-19, D-20) ----------

pub const NO_NAME: &str = "Give the measure a name.";
pub const NAME_TAKEN: &str = "You already have a measure with that name.";
pub const TOO_MANY: &str = "You can keep up to 30 measures of your own.";
pub const NOTHING_TO_REMOVE: &str = "There's no value to remove for that day.";
pub const MAX_CUSTOM_MEASURES: usize = 30;
pub const MAX_MEASURE_NAME: usize = 40;

/// One box of the Edit Measurements dialog: a value typed in `unit`, or None for a box cleared.
#[derive(Clone, Deserialize, Debug, PartialEq)]
pub struct DayValue {
    pub measure: String,
    pub value: Option<f64>,
    pub unit: UnitIn,
}

fn measure_name(j: &mut Journal, id: &str) -> String {
    match BUILT_IN.iter().find(|(b, _, _)| *b == id) {
        Some((_, name, _)) => name.to_string(),
        None => j.profile().ok().and_then(|p| p.measures.iter().find(|m| m.uid == id).map(|m| m.name.clone())).unwrap_or_default(),
    }
}

/// Soft-delete the live value of `measure` on `day` and give back what it was. None (and nothing marked changed) when there isn't one.
fn clear_live(j: &mut Journal, day: &str, measure: &str, now: u64) -> Result<Option<BodyEntry>, String> {
    let d = Day::parse(day)?;
    let live = |e: &&mut BodyEntry| e.deleted_at.is_none() && e.day == day && e.measure == measure;
    if !j.month(d.month())?.body.iter().any(|e| e.deleted_at.is_none() && e.day == day && e.measure == measure) {
        return Ok(None);
    }
    Ok(j.month_mut(d.month())?.body.iter_mut().find(live).map(|e| {
        let before = e.clone();
        e.deleted_at = Some(now);
        e.modified = now;
        before
    }))
}

/// Save or clear several measures of one day in one step with one undo (D-20). Everything is checked before anything is written;
/// a weight in the set gets the same ask-once check as `log`. `text` is the day, for the toast ("Nov 15"). A set that changes
/// nothing (clearing boxes that were empty) keeps no undo.
pub fn set_day(j: &mut Journal, day: &str, today: &str, values: &[DayValue], confirmed: bool, now: u64) -> Result<Logged, String> {
    check_day(day, today)?;
    let mut plan: Vec<(&str, Option<f64>)> = vec![];
    for v in values {
        if plan.iter().any(|(m, _)| *m == v.measure) {
            return Err("Each measure can be changed once in one save.".into());
        }
        let canon = match v.value {
            None => {
                measure_kind(j, &v.measure, true)?;
                None
            }
            Some(x) => {
                let kind = measure_kind(j, &v.measure, false)?;
                let c = canonical(kind, x, v.unit)?;
                check_value(kind, c)?;
                Some(c)
            }
        };
        plan.push((&v.measure, canon));
    }
    if let Some((_, Some(kg))) = plan.iter().find(|(m, _)| *m == "weight") {
        if let Some(ask) = weight_gate(j, day, *kg, confirmed)? {
            return Ok(Logged { confirm: Some(ask), text: String::new(), undo: None });
        }
    }
    let mut undo = DiaryUndo { day: day.into(), ..Default::default() };
    for (measure, value) in plan {
        match value {
            Some(v) => {
                let set = set_value(j, day, today, measure, v, now)?;
                match set.before {
                    Some(b) => undo.before_body.push(b),
                    None => undo.added_body.push(set.uid),
                }
            }
            None => undo.before_body.extend(clear_live(j, day, measure, now)?),
        }
    }
    let changed = !undo.before_body.is_empty() || !undo.added_body.is_empty();
    Ok(Logged { confirm: None, text: goals::date_text(day), undo: changed.then_some(undo) })
}

/// Remove one value (it stays in the month file, marked deleted). Returns the measure's name and the undo.
pub fn delete_value(j: &mut Journal, day: &str, measure: &str, now: u64) -> Result<(String, DiaryUndo), String> {
    let name = measure_name(j, measure);
    let before = clear_live(j, day, measure, now)?.ok_or(NOTHING_TO_REMOVE)?;
    Ok((name, DiaryUndo { day: day.into(), before_body: vec![before], ..Default::default() }))
}

/// Add one of the person's own measures and return its uid. The name is trimmed and cut to 40 characters; it can't repeat a
/// built-in or one of theirs (archived too, so a restore never makes two of a name), and there are at most 30.
pub fn save_measure(j: &mut Journal, name: &str, kind: MeasureKind) -> Result<String, String> {
    let name: String = name.trim().chars().take(MAX_MEASURE_NAME).collect::<String>().trim().to_string();
    if name.is_empty() {
        return Err(NO_NAME.into());
    }
    let same = |other: &str| other.to_lowercase() == name.to_lowercase();
    let p = j.profile()?;
    if BUILT_IN.iter().any(|(_, b, _)| same(b)) || p.measures.iter().any(|m| same(&m.name)) {
        return Err(NAME_TAKEN.into());
    }
    if p.measures.len() >= MAX_CUSTOM_MEASURES {
        return Err(TOO_MANY.into());
    }
    let uid = diary::new_uid();
    j.profile_mut()?.measures.push(CustomMeasure { uid: uid.clone(), name, kind, archived: false });
    Ok(uid)
}

/// Hide a custom measure from new entries and the table, or bring it back. Every value stays. There is no delete: a measure with
/// values is archived, never removed. Returns its name.
pub fn archive_measure(j: &mut Journal, uid: &str, archived: bool) -> Result<String, String> {
    let m = j.profile_mut()?.measures.iter_mut().find(|m| m.uid == uid).ok_or_else(|| NO_MEASURE.to_string())?;
    m.archived = archived;
    Ok(m.name.clone())
}

// ---------- the Body view ----------

/// How far back the chart and table reach, ending today (both ends included).
#[derive(Clone, Copy, Deserialize, Debug, PartialEq, Eq)]
pub enum BodyRange {
    #[serde(rename = "2w")]
    W2,
    #[serde(rename = "1m")]
    M1,
    #[serde(rename = "3m")]
    M3,
    #[serde(rename = "6m")]
    M6,
    #[serde(rename = "1y")]
    Y1,
    #[serde(rename = "all")]
    All,
}

impl BodyRange {
    /// Days in the range; None for All (from the first record).
    pub(crate) fn days(self) -> Option<i32> {
        match self {
            BodyRange::W2 => Some(14),
            BodyRange::M1 => Some(30),
            BodyRange::M3 => Some(91),
            BodyRange::M6 => Some(182),
            BodyRange::Y1 => Some(365),
            BodyRange::All => None,
        }
    }
}

/// A measure the person can log: built-in or their own.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct MeasureOption {
    pub id: String,
    pub name: String,
    pub kind: MeasureKind,
    pub unit_text: &'static str,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Column {
    pub id: String,
    pub name: String,
    pub unit_text: &'static str,
}

/// A stored value as a number in the person's unit at full precision, for a dialog to prefill from. The cell text is rounded to 0.1;
/// editing from that text would round what was logged, so edits start from this number and its unit instead.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct CellValue {
    pub value: f64,
    pub unit: UnitIn,
}

/// One day of the measurement table: a cell per column, None for a measure not logged that day. `values` lines up with `cells`.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Row {
    pub day: String,
    pub date_text: String,
    pub cells: Vec<Option<String>>,
    pub values: Vec<Option<CellValue>>,
}

/// A stored value in the person's unit, with the `UnitIn` that sends it back unchanged.
fn cell_value(kind: MeasureKind, v: f64, u: &UnitChoice) -> CellValue {
    match kind {
        MeasureKind::Mass => CellValue { value: goals::in_unit(v, u.weight), unit: if u.weight == MassUnit::Lb { UnitIn::Lb } else { UnitIn::Kg } },
        MeasureKind::Length if u.height == HeightUnit::FtIn => CellValue { value: v / goals::CM_PER_IN, unit: UnitIn::In },
        MeasureKind::Length => CellValue { value: v, unit: UnitIn::Cm },
        MeasureKind::Percent | MeasureKind::Number => CellValue { value: v, unit: UnitIn::None },
    }
}

/// One weigh-in day on the chart, in the person's unit. Days without a weigh-in have no point (nothing is interpolated).
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Point {
    pub day: i32,
    pub raw: Option<f64>,
    pub trend: Option<f64>,
}

/// The chart's Show as Table row.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct ChartRow {
    pub date_text: String,
    pub raw_text: String,
    pub trend_text: String,
}

/// Everything the weight chart draws: numbers in the person's unit and the sentences around them. The page does no maths beyond mapping these to pixels.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Chart {
    pub points: Vec<Point>,
    pub x_min: i32,
    pub x_max: i32,
    pub y_min: f64,
    pub y_max: f64,
    pub goal: Option<f64>,
    pub goal_text: Option<String>,
    pub unit_text: &'static str,
    pub label: String,
    pub empty_text: Option<&'static str>,
    pub table: Vec<ChartRow>,
}

/// What the Body page opens with. Trend first (D-24): the headline is the smoothed weight, never the day's scale number.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct BodyView {
    /// No weigh-in or measurement logged at all.
    pub empty: bool,
    pub headline: Option<String>,
    pub trend_text: Option<String>,
    pub rate_text: Option<String>,
    pub note: &'static str,
    pub forecast: Option<String>,
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub measures: Vec<MeasureOption>,
    pub archived: Vec<MeasureOption>,
    pub chart: Option<Chart>,
    /// The refusal sentence when the profile or this month can't be changed (damaged or from a newer build).
    pub read_only: Option<String>,
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn day_key(n: i32) -> String {
    let (y, m, d) = civil_from_days(n);
    format!("{y:04}-{m:02}-{d:02}")
}

/// "Nov 17", with the year when it isn't `this_year`, built from the day's parts.
pub(crate) fn date_short(n: i32, this_year: i32) -> String {
    let base = goals::date_text(&day_key(n));
    let y = civil_from_days(n).0;
    if y == this_year {
        base
    } else {
        format!("{base}, {y}")
    }
}

/// "Tue, Nov 17", with the year when it isn't `this_year`.
pub(crate) fn date_long(n: i32, this_year: i32) -> String {
    format!("{}, {}", WEEKDAYS[weekday(n) as usize], date_short(n, this_year))
}

/// Every live record in every month, in no particular order. A month that won't open is skipped.
fn live_entries(j: &mut Journal) -> Vec<BodyEntry> {
    let mut out = vec![];
    for m in j.months_with_data() {
        if let Ok(month) = j.month(&m) {
            out.extend(month.body.iter().filter(|e| e.deleted_at.is_none()).cloned());
        }
    }
    out
}

pub(crate) fn range_start(j: &mut Journal, range: BodyRange, today_n: i32) -> i32 {
    match range.days() {
        Some(n) => today_n - (n - 1),
        None => live_entries(j).iter().filter_map(|e| diary::day_number(&e.day).ok()).min().unwrap_or(today_n),
    }
}

// ---------- trend, rate, chart and forecast (BODY-02, BODY-03, D-22 to D-24) ----------

/// Weigh-ins it takes in the last 28 days, spanning this many days, before a rate or a forecast is said: one idea of "enough" (RESEARCH A8, A11).
pub const FORECAST_MIN_WEIGH_INS: usize = TREND_MIN_WEIGH_INS;
pub const FORECAST_MIN_SPAN_DAYS: i32 = 14;
pub const FORECAST_WINDOW_DAYS: i32 = TREND_WINDOW_DAYS;
/// A trend that moves less than this many kg a week is flat.
pub const FLAT_KG_PER_WEEK: f64 = 0.05;
/// A goal further off than this many days at the current pace is "more than 2 years".
pub const FORECAST_MAX_DAYS: f64 = 730.0;
/// A trend this close to the goal, in kg, is at it.
const AT_GOAL_KG: f64 = 0.05;

pub const NOT_ENOUGH: &str = "Not enough data yet.";
const FLAT: &str = "Your trend is flat.";
const AWAY: &str = "Your trend isn't heading toward your goal yet.";
const AT_GOAL: &str = "Your trend is at your goal weight.";
const TOO_FAR: &str = "More than 2 years at this pace.";
const NO_POINTS: &str = "No weigh-ins in this range. Try a longer range.";

/// Least-squares slope of (day, value) points, per day. None for fewer than two points or no spread in days.
pub fn slope(points: &[(i32, f64)]) -> Option<f64> {
    if points.len() < 2 {
        return None;
    }
    let n = points.len() as f64;
    let (mx, my) = (points.iter().map(|p| f64::from(p.0)).sum::<f64>() / n, points.iter().map(|p| p.1).sum::<f64>() / n);
    let sxx: f64 = points.iter().map(|p| (f64::from(p.0) - mx).powi(2)).sum();
    if sxx == 0.0 {
        return None;
    }
    Some(points.iter().map(|p| (f64::from(p.0) - mx) * (p.1 - my)).sum::<f64>() / sxx)
}

/// The points of an ascending series that fall in the 28 days ending `today_n`.
fn recent(series: &[(i32, f64)], today_n: i32) -> &[(i32, f64)] {
    &series[series.partition_point(|p| p.0 <= today_n - FORECAST_WINDOW_DAYS)..]
}

fn enough(window: &[(i32, f64)]) -> bool {
    window.len() >= FORECAST_MIN_WEIGH_INS && window.last().zip(window.first()).is_some_and(|(l, f)| l.0 - f.0 >= FORECAST_MIN_SPAN_DAYS)
}

/// The trend's weekly change in kg, once there are enough weigh-ins.
fn week_rate(series: &[(i32, f64)], today_n: i32) -> Option<f64> {
    let w = recent(series, today_n);
    enough(w).then(|| slope(w)).flatten().map(|per_day| per_day * 7.0)
}

/// "−0.3 kg/week" with a true minus, "+0.3 kg/week", or "0.0 kg/week" when it rounds to nothing.
fn rate_text(kg_week: f64, unit: MassUnit) -> String {
    let shown = goals::in_unit(kg_week.abs(), unit);
    let sign = if (shown * 10.0).round() == 0.0 {
        ""
    } else if kg_week < 0.0 {
        "\u{2212}"
    } else {
        "+"
    };
    format!("{sign}{shown:.1} {}/week", mass_word(unit))
}

/// The trend as of the latest weigh-in, and its weekly rate once there are enough weigh-ins. Never the day's scale number (D-24).
fn headline_parts(j: &mut Journal, today: &str, unit: MassUnit) -> Option<(String, Option<String>)> {
    let series = trend(&weight_points(j, today));
    let latest = series.last()?.1;
    let rate = week_rate(&series, diary::day_number(today).ok()?).map(|r| rate_text(r, unit));
    Some((goals::mass_text(latest, unit), rate))
}

pub(crate) fn range_name(range: BodyRange) -> &'static str {
    match range {
        BodyRange::W2 => "last 2 weeks",
        BodyRange::M1 => "last month",
        BodyRange::M3 => "last 3 months",
        BodyRange::M6 => "last 6 months",
        BodyRange::Y1 => "last year",
        BodyRange::All => "all time",
    }
}

/// The y range of a chart of `data` with a goal line: (y_min, y_max, where the goal line goes, an arrow for its label).
/// A far goal would squash the data into a flat line, so it stays in the scale only while the data fill a fifth of the
/// height; past that the line sits on the edge the goal lies beyond, clear of the data, and its label points the way.
/// `min_span` is 1 kg in the chart's unit, so kilograms and pounds draw the same picture. No data: the goal alone, padded, or 0 to 1.
pub(crate) fn goal_scale(data: &[f64], goal: Option<f64>, min_span: f64) -> (f64, f64, Option<f64>, &'static str) {
    let span = |vs: &mut dyn Iterator<Item = f64>| vs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
    if data.is_empty() {
        return goal.map_or((0.0, 1.0, None, ""), |g| (g - 0.5, g + 0.5, Some(g), ""));
    }
    let (dlo, dhi) = span(&mut data.iter().copied());
    let far = goal.filter(|g| (dhi.max(*g) - dlo.min(*g)) > 5.0 * (dhi - dlo).max(min_span));
    let (lo, hi) = if far.is_some() { (dlo, dhi) } else { span(&mut data.iter().copied().chain(goal)) };
    let pad = 0.5_f64.max(0.05 * (hi - lo));
    let gap = pad.max(0.15 * (hi - lo));
    let line = goal.map(|g| if far.is_some() { if g < lo { lo - gap } else { hi + gap } } else { g });
    let arrow = far.map_or("", |g| if g < dlo { " \u{2193}" } else { " \u{2191}" });
    (line.map_or(lo - pad, |g| g.min(lo - pad)), line.map_or(hi + pad, |g| g.max(hi + pad)), line, arrow)
}

/// The weight chart's data for `range`: one point per weigh-in day, the raw value and its trend (computed over all history,
/// then sliced), nothing for the days between. Everything is in the person's unit. The goal line is on the chart, at its
/// value or, when that would flatten the weigh-ins, on the nearest edge (see [`goal_scale`]).
pub fn weight_chart(j: &mut Journal, range: BodyRange, today: &str) -> Chart {
    let unit = units_of(j).weight;
    let goal_kg = j.profile().ok().and_then(|p| p.goal_weight_kg);
    let today_n = diary::day_number(today).unwrap_or(0);
    let this_year = civil_from_days(today_n).0;
    let start = range_start(j, range, today_n);
    let weights = weight_points(j, today);
    let series = trend(&weights);
    // (day, raw kg, trend kg) inside the range, both ends included
    let inside: Vec<(i32, f64, f64)> = weights.iter().zip(&series).filter(|(w, _)| (start..=today_n).contains(&w.0)).map(|(w, t)| (w.0, w.1, t.1)).collect();
    let conv = |kg: f64| goals::in_unit(kg, unit);
    let points: Vec<Point> = inside.iter().map(|&(day, raw, t)| Point { day, raw: Some(conv(raw)), trend: Some(conv(t)) }).collect();
    let table = inside.iter().map(|&(day, raw, t)| ChartRow { date_text: date_long(day, this_year), raw_text: goals::mass_text(raw, unit), trend_text: goals::mass_text(t, unit) }).collect();
    let data: Vec<f64> = points.iter().flat_map(|p| [p.raw, p.trend]).flatten().collect();
    let (y_min, y_max, goal, arrow) = goal_scale(&data, goal_kg.map(conv), conv(1.0));
    let word = mass_word(unit);
    let name = range_name(range);
    let label = match inside.as_slice() {
        [] => format!("Weight, {name}: no weigh-ins."),
        [only] => format!("Weight, {name}: trend {}.", goals::mass_text(only.2, unit)),
        [first, .., last] => {
            let rate = week_rate(&series, today_n).map(|r| format!(", {}", rate_text(r, unit))).unwrap_or_default();
            format!("Weight, {name}: trend from {:.1} to {}{rate}.", conv(first.2), goals::mass_text(last.2, unit))
        }
    };
    Chart {
        empty_text: points.is_empty().then_some(NO_POINTS),
        points,
        x_min: start,
        x_max: today_n,
        y_min,
        y_max,
        goal,
        goal_text: goal_kg.map(|g| format!("Goal {} {word}{arrow}", trimmed1(conv(g)))),
        unit_text: word,
        label,
        table,
    }
}

/// When the trend reaches the goal weight, said plainly (D-23). A date shows only with enough weigh-ins, a goal weight, and a trend
/// moving toward it at 0.05 kg a week or more; "toward" is the way the goal kind asks for, or with none chosen the side the
/// trend started on. None with no goal weight.
pub fn forecast(j: &mut Journal, today: &str) -> Option<String> {
    let (goal, kind, unit) = {
        let p = j.profile().ok()?;
        (p.goal_weight_kg?, p.goal, p.units.weight)
    };
    let today_n = diary::day_number(today).ok()?;
    let series = trend(&weight_points(j, today));
    let w = recent(&series, today_n);
    if !enough(w) {
        return Some(NOT_ENOUGH.into());
    }
    let per_day = slope(w)?;
    let (first, now) = (w.first()?.1, w.last()?.1);
    let want_down = match kind {
        goals::GoalKind::Lose => true,
        goals::GoalKind::Gain => false,
        goals::GoalKind::Maintain => first >= goal,
    };
    if (goal - now).abs() < AT_GOAL_KG || (want_down && now <= goal) || (!want_down && now >= goal) {
        return Some(AT_GOAL.into());
    }
    if per_day.abs() * 7.0 < FLAT_KG_PER_WEEK {
        return Some(FLAT.into());
    }
    if (per_day < 0.0) != want_down {
        return Some(AWAY.into()); // moving away from the goal: no date, and not "not enough data" (D-23)
    }
    let days = (goal - now) / per_day;
    if days > FORECAST_MAX_DAYS {
        return Some(TOO_FAR.into());
    }
    let on = date_short(today_n + days.ceil() as i32, civil_from_days(today_n).0);
    Some(format!("At this pace you reach {} around {on}.", goals::mass_text(goal, unit)))
}

/// The Body page: the headline, the measurement table for `range`, and the lists of measures. Reads only.
pub fn body_view(j: &mut Journal, range: BodyRange, today: &str) -> Result<BodyView, String> {
    let t = Day::parse(today)?;
    let today_n = diary::day_number(t.as_str())?;
    let read_only = j.profile().err().or_else(|| j.month(t.month()).err());
    let profile = j.profile().map(|p| p.clone()).unwrap_or_default();
    let units = profile.units;
    let live: Vec<BodyEntry> = live_entries(j).into_iter().filter(|e| e.day.as_str() <= today).collect();

    let option = |id: &str, name: &str, kind: MeasureKind| MeasureOption { id: id.into(), name: name.into(), kind, unit_text: unit_text(kind, &units) };
    let mut measures: Vec<MeasureOption> = BUILT_IN.iter().map(|(id, name, kind)| option(id, name, *kind)).collect();
    measures.extend(profile.measures.iter().filter(|m| !m.archived).map(|m| option(&m.uid, &m.name, m.kind)));
    let archived: Vec<MeasureOption> = profile.measures.iter().filter(|m| m.archived).map(|m| option(&m.uid, &m.name, m.kind)).collect();

    // Weight always, then every other measure that has a live value
    let shown: Vec<&MeasureOption> = measures.iter().filter(|m| m.id == "weight" || live.iter().any(|e| e.measure == m.id)).collect();
    let columns = shown.iter().map(|m| Column { id: m.id.clone(), name: m.name.clone(), unit_text: m.unit_text }).collect();

    let start = range_start(j, range, today_n);
    let mut by_day: BTreeMap<i32, BTreeMap<&str, (u64, f64)>> = BTreeMap::new();
    for e in &live {
        let Ok(n) = diary::day_number(&e.day) else { continue };
        if n < start || n > today_n {
            continue;
        }
        let slot = by_day.entry(n).or_default().entry(e.measure.as_str()).or_insert((e.logged_at, e.value));
        if e.logged_at > slot.0 {
            *slot = (e.logged_at, e.value); // a merged file may hold two for a day: the later logged wins
        }
    }
    let this_year = civil_from_days(today_n).0;
    let rows = by_day
        .iter()
        .rev()
        .filter_map(|(n, vals)| {
            let cells: Vec<Option<String>> = shown.iter().map(|m| vals.get(m.id.as_str()).map(|(_, v)| value_text(m.kind, *v, &units))).collect();
            let values = shown.iter().map(|m| vals.get(m.id.as_str()).map(|(_, v)| cell_value(m.kind, *v, &units))).collect();
            cells.iter().any(Option::is_some).then(|| Row { day: day_key(*n), date_text: date_long(*n, this_year), cells, values })
        })
        .collect();

    let (trend_text, rate_text) = match headline_parts(j, today, units.weight) {
        Some((t, r)) => (Some(t), Some(r)),
        None => (None, None),
    };
    let headline = trend_text.as_ref().map(|t| match rate_text.as_ref().and_then(Option::as_ref) {
        Some(r) => format!("Trend {t}, {r}"),
        None => format!("Trend {t}"),
    });
    let rate_text = rate_text.map(|r| r.unwrap_or_else(|| FEW_FOR_RATE.to_string()));
    let forecast = forecast(j, today);
    let chart = Some(weight_chart(j, range, today));
    Ok(BodyView { empty: live.is_empty(), headline, trend_text, rate_text, note: SWINGS_NOTE, forecast, columns, rows, measures, archived, chart, read_only })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = crate::scratch_dir().join(format!("rb-body-{}-{}", name, crate::test_pid()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    const TODAY: &str = "2026-11-17";

    #[test]
    fn a_weigh_in_is_replaced_in_place_and_read_as_of_a_day() {
        let d = tdir("replace");
        let mut j = Journal::open(d.clone());
        let first = set_value(&mut j, TODAY, TODAY, "weight", 80.0, 100).unwrap();
        assert_eq!(first.before, None);
        let second = set_value(&mut j, TODAY, TODAY, "weight", 79.5, 200).unwrap();
        assert_eq!(second.uid, first.uid);
        let before = second.before.unwrap();
        assert_eq!((before.value, before.modified), (80.0, 100));
        let m = j.month("2026-11").unwrap();
        assert_eq!(m.body.len(), 1);
        assert_eq!((m.body[0].value, m.body[0].modified, m.body[0].logged_at), (79.5, 200, 100));
        let b = weight_basis(&mut j, "2026-11-30").unwrap();
        assert_eq!((b.kg, b.trend, b.day.as_str()), (79.5, false, TODAY));
        assert_eq!(weight_basis(&mut j, "2026-11-16"), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn weight_basis_looks_back_across_months_and_skips_deleted() {
        let d = tdir("months");
        let mut j = Journal::open(d.clone());
        set_value(&mut j, "2026-09-10", TODAY, "weight", 82.0, 1).unwrap();
        set_value(&mut j, "2026-11-05", TODAY, "weight", 81.0, 2).unwrap();
        set_value(&mut j, "2026-11-05", TODAY, "waist", 90.0, 3).unwrap();
        j.save_dirty().unwrap();
        let mut fresh = Journal::open(d.clone());
        assert_eq!(weight_basis(&mut fresh, "2026-10-20").map(|b| b.kg), Some(82.0), "a later weigh-in never enters an earlier day");
        assert_eq!(weight_basis(&mut fresh, "2026-12-01").map(|b| b.kg), Some(81.0));
        for e in &mut fresh.month_mut("2026-11").unwrap().body {
            e.deleted_at = Some(9);
        }
        assert_eq!(weight_basis(&mut fresh, "2026-12-01").map(|b| b.kg), Some(82.0), "a deleted entry is not a weigh-in");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn set_value_refuses_what_cannot_be_a_measurement() {
        let d = tdir("refuse");
        let mut j = Journal::open(d.clone());
        let mut put = |day: &str, m: &str, v: f64| set_value(&mut j, day, TODAY, m, v, 1).map(|_| ());
        assert_eq!(put("2026-11-18", "weight", 80.0), Err(FUTURE_DAY.into()));
        for bad in [f64::NAN, f64::INFINITY, 0.0, -1.0, 500.1] {
            assert_eq!(put(TODAY, "weight", bad), Err(BAD_MASS.into()), "{bad}");
        }
        assert_eq!(put(TODAY, "weight", 500.0), Ok(()));
        assert_eq!(put(TODAY, "waist", 400.1), Err(BAD_LENGTH.into()));
        assert_eq!(put(TODAY, "body_fat", 100.0), Err(BAD_PERCENT.into()));
        assert_eq!(put(TODAY, "body_fat", 99.9), Ok(()));
        assert_eq!(put(TODAY, "nope", 1.0), Err(NO_MEASURE.into()));
        assert!(put("2026-02-30", "weight", 80.0).unwrap_err().contains("isn't a valid date"));
        let _ = fs::remove_dir_all(&d);
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn trend_ewma() {
        assert_eq!(trend(&[(0, 80.0)]), vec![(0, 80.0)]);
        assert_eq!(trend(&[]), vec![]);
        let t = trend(&[(0, 80.0), (1, 81.0)]);
        assert!(near(t[1].1, 80.1), "{t:?}");
        // a gap of three days weighs the new reading by 1 - 0.9^3 = 0.271
        let t = trend(&[(0, 80.0), (3, 81.0)]);
        assert!(near(t[1].1, 80.271), "{t:?}");
        assert_eq!(t.iter().map(|p| p.0).collect::<Vec<_>>(), [0, 3]);
    }

    #[test]
    fn weight_basis_uses_trend_once_there_are_enough() {
        let d = tdir("trend");
        let mut j = Journal::open(d.clone());
        // logged out of order, across the 4th and the 1st
        for (day, kg) in [("2026-11-03", 81.0), ("2026-11-01", 80.0), ("2026-11-04", 81.5), ("2026-11-02", 80.5)] {
            set_value(&mut j, day, "2026-11-30", "weight", kg, 1).unwrap();
        }
        assert_eq!(weight_points(&mut j, "2026-11-30").iter().map(|p| p.1).collect::<Vec<_>>(), [80.0, 80.5, 81.0, 81.5], "ascending, one per day");
        let b = weight_basis(&mut j, "2026-11-04").unwrap();
        assert_eq!((b.kg, b.trend, b.day.as_str()), (81.5, false, "2026-11-04"), "four weigh-ins: the latest one");
        set_value(&mut j, "2026-11-05", "2026-11-30", "weight", 82.0, 1).unwrap();
        let b = weight_basis(&mut j, "2026-11-05").unwrap();
        assert!(b.trend && near(b.kg, 80.45245), "{b:?}");
        assert_eq!(b.day, "2026-11-05");
        // weigh-ins after the day are ignored: on the 4th there were only four
        let b = weight_basis(&mut j, "2026-11-04").unwrap();
        assert_eq!((b.kg, b.trend), (81.5, false));
        // five weigh-ins spread over more than 28 days are not a trend
        let far = tdir("trend-far");
        let mut k = Journal::open(far.clone());
        for (day, kg) in [("2026-09-01", 90.0), ("2026-09-20", 89.0), ("2026-10-10", 88.0), ("2026-10-30", 87.0), ("2026-11-17", 86.0)] {
            set_value(&mut k, day, "2026-11-30", "weight", kg, 1).unwrap();
        }
        let b = weight_basis(&mut k, "2026-11-17").unwrap();
        assert_eq!((b.kg, b.trend), (86.0, false));
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(&far);
    }

    #[test]
    fn check_weight_asks_once() {
        use goals::MassUnit::{Kg, Lb};
        let ask = |prev: Option<f64>, new: f64| check_weight(prev, new, Kg);
        assert_eq!(ask(Some(80.0), 84.1), WeightCheck::Confirm("That's 4.1 kg more than your last weigh-in. Save anyway?".into()));
        assert_eq!(ask(Some(80.0), 84.0), WeightCheck::Ok, "exactly at max(2 kg, 5%) does not ask");
        assert_eq!(ask(Some(80.0), 75.9), WeightCheck::Confirm("That's 4.1 kg less than your last weigh-in. Save anyway?".into()));
        assert_eq!(ask(Some(80.0), 82.0), WeightCheck::Ok);
        assert_eq!(ask(Some(150.0), 157.0), WeightCheck::Ok, "5% of 150 is 7.5");
        assert_eq!(ask(Some(150.0), 157.6), WeightCheck::Confirm("That's 7.6 kg more than your last weigh-in. Save anyway?".into()));
        assert_eq!(ask(None, 70.0), WeightCheck::Ok);
        for bad in [0.0, -3.0, 500.1, f64::NAN, f64::INFINITY] {
            assert_eq!(ask(Some(80.0), bad), WeightCheck::Refused(BAD_MASS.into()), "{bad}");
        }
        assert_eq!(ask(None, 500.0), WeightCheck::Ok);
        // the sentence is in the person's unit
        assert_eq!(check_weight(Some(80.0), 90.0, Lb), WeightCheck::Confirm("That's 22 lb more than your last weigh-in. Save anyway?".into()));
    }

    #[test]
    fn the_previous_weigh_in_is_the_one_before_the_day() {
        let d = tdir("previous");
        let mut j = Journal::open(d.clone());
        assert_eq!(previous_weight(&mut j, TODAY), None);
        set_value(&mut j, "2026-11-10", TODAY, "weight", 80.0, 1).unwrap();
        set_value(&mut j, TODAY, TODAY, "weight", 79.0, 2).unwrap();
        assert_eq!(previous_weight(&mut j, TODAY), Some(80.0), "the entry on the day itself is the one being replaced");
        assert_eq!(previous_weight(&mut j, "2026-11-10"), None);
        let _ = fs::remove_dir_all(&d);
    }

    fn put(j: &mut Journal, day: &str, kg: f64) {
        log(j, day, TODAY, "weight", kg, UnitIn::Kg, true, 1).unwrap();
    }

    #[test]
    fn log_converts_and_replaces() {
        let d = tdir("log");
        let mut j = Journal::open(d.clone());
        let a = log(&mut j, TODAY, TODAY, "weight", 78.9, UnitIn::Kg, false, 10).unwrap();
        assert_eq!((a.text.as_str(), a.confirm.clone()), ("78.9 kg", None));
        let au = a.undo.clone().unwrap();
        assert_eq!((au.day.as_str(), au.before_body.len(), au.added_body.len()), (TODAY, 0, 1));
        // a pound value is stored as kg and reads back as the pound value typed
        j.profile_mut().unwrap().units.weight = MassUnit::Lb;
        let b = log(&mut j, TODAY, TODAY, "weight", 174.0, UnitIn::Lb, false, 20).unwrap();
        assert_eq!(b.text, "174.0 lb");
        let bu = b.undo.clone().unwrap();
        assert_eq!((bu.before_body.len(), bu.added_body.len()), (1, 0), "a replace adds nothing new");
        assert_eq!(bu.before_body[0].uid, au.added_body[0], "same uid");
        assert_eq!(bu.before_body[0].value, 78.9);
        let m = j.month("2026-11").unwrap();
        assert_eq!(m.body.len(), 1);
        assert!((m.body[0].value - 78.92507238).abs() < 1e-6, "{}", m.body[0].value);
        assert_eq!(m.body[0].modified, 20);
        // the undo puts the first value back
        diary::undo(&mut j, &bu, 30).unwrap();
        assert_eq!(j.month("2026-11").unwrap().body[0].value, 78.9);
        // the wrong unit for a measure is refused; inches are converted
        assert_eq!(log(&mut j, TODAY, TODAY, "weight", 1.0, UnitIn::Cm, false, 1).unwrap_err(), BAD_UNIT);
        assert_eq!(log(&mut j, TODAY, TODAY, "body_fat", 20.0, UnitIn::Kg, false, 1).unwrap_err(), BAD_UNIT);
        let w = log(&mut j, TODAY, TODAY, "waist", 33.0, UnitIn::In, false, 1).unwrap();
        assert_eq!(w.text, "83.8 cm");
        assert_eq!(log(&mut j, TODAY, TODAY, "body_fat", 22.5, UnitIn::None, false, 1).unwrap().text, "22.5%");
        // the same value twice leaves one entry with the same uid
        let (x, y) = (log(&mut j, TODAY, TODAY, "waist", 80.0, UnitIn::Cm, false, 2).unwrap(), log(&mut j, TODAY, TODAY, "waist", 80.0, UnitIn::Cm, false, 3).unwrap());
        assert_eq!(y.undo.unwrap().before_body[0].uid, x.undo.unwrap().before_body[0].uid);
        assert_eq!(j.month("2026-11").unwrap().body.iter().filter(|e| e.measure == "waist").count(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn log_asks_once_for_weight() {
        let d = tdir("ask");
        let mut j = Journal::open(d.clone());
        put(&mut j, "2026-11-10", 148.9);
        let ask = log(&mut j, TODAY, TODAY, "weight", 78.9, UnitIn::Kg, false, 5).unwrap();
        assert_eq!(ask.confirm.as_deref(), Some("That's 70 kg less than your last weigh-in. Save anyway?"));
        assert!(ask.undo.is_none() && ask.text.is_empty());
        assert_eq!(j.month("2026-11").unwrap().body.len(), 1, "nothing was written");
        let yes = log(&mut j, TODAY, TODAY, "weight", 78.9, UnitIn::Kg, true, 6).unwrap();
        assert_eq!((yes.confirm, yes.text.as_str()), (None, "78.9 kg"));
        assert_eq!(j.month("2026-11").unwrap().body.len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn log_refuses_impossible_and_future() {
        let d = tdir("refuse-log");
        let mut j = Journal::open(d.clone());
        for bad in [0.0, -1.0, 500.1, f64::NAN, f64::INFINITY] {
            assert_eq!(log(&mut j, TODAY, TODAY, "weight", bad, UnitIn::Kg, true, 1).unwrap_err(), BAD_MASS, "{bad}");
        }
        assert_eq!(log(&mut j, "2026-11-18", TODAY, "weight", 80.0, UnitIn::Kg, true, 1).unwrap_err(), FUTURE_DAY);
        assert_eq!(log(&mut j, TODAY, TODAY, "nope", 80.0, UnitIn::Kg, true, 1).unwrap_err(), NO_MEASURE);
        assert_eq!(log(&mut j, TODAY, TODAY, "waist", 0.0, UnitIn::Cm, true, 1).unwrap_err(), BAD_LENGTH);
        assert!(log(&mut j, "2026-02-30", TODAY, "weight", 80.0, UnitIn::Kg, true, 1).unwrap_err().contains("isn't a valid date"));
        assert!(j.month("2026-11").unwrap().body.is_empty(), "every refusal wrote nothing");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn body_view_headline_is_the_trend() {
        let d = tdir("view");
        let mut j = Journal::open(d.clone());
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert!(v.empty && v.headline.is_none() && v.rows.is_empty() && v.read_only.is_none());
        assert_eq!((v.columns.len(), v.measures.len(), v.archived.len()), (1, 8, 0), "Weight is always a column");
        put(&mut j, TODAY, 78.9);
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert!(!v.empty);
        assert_eq!(v.headline.as_deref(), Some("Trend 78.9 kg"));
        assert_eq!(v.rate_text.as_deref(), Some("Log a few more to see a rate."));
        assert_eq!(v.note, "Daily swings of a kilo or two are normal. The trend smooths them out.");
        assert_eq!(v.rows.len(), 1);
        assert_eq!((v.rows[0].date_text.as_str(), v.rows[0].cells[0].as_deref()), ("Tue, Nov 17", Some("78.9 kg")));
        // the headline is the smoothed trend, not the day's scale number
        put(&mut j, "2026-11-10", 80.0);
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert_eq!(v.headline.as_deref(), Some("Trend 79.4 kg"));
        assert_eq!(v.rows.iter().map(|r| r.day.as_str()).collect::<Vec<_>>(), ["2026-11-17", "2026-11-10"], "newest first");
        // a measure with a value gets a column, and a day without it shows no cell
        log(&mut j, TODAY, TODAY, "waist", 84.0, UnitIn::Cm, false, 2).unwrap();
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert_eq!(v.columns.iter().map(|c| (c.name.as_str(), c.unit_text)).collect::<Vec<_>>(), [("Weight", "kg"), ("Waist", "cm")]);
        assert_eq!((v.rows[0].cells[1].as_deref(), v.rows[1].cells[1].clone()), (Some("84.0 cm"), None));
        // a range that ends before the older weigh-in leaves it out
        assert_eq!(body_view(&mut j, BodyRange::W2, "2026-12-30").unwrap().rows.len(), 0);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn row_values_carry_the_stored_number_so_an_edit_never_rounds_it() {
        let d = tdir("row-values");
        let mut j = Journal::open(d.clone());
        j.profile_mut().unwrap().units.weight = MassUnit::Lb;
        // 174.04 lb reads "174.0 lb" in the cell, but the number the dialog prefills from keeps its precision
        log(&mut j, TODAY, TODAY, "weight", 174.04, UnitIn::Lb, true, 1).unwrap();
        log(&mut j, TODAY, TODAY, "waist", 84.37, UnitIn::Cm, false, 2).unwrap();
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        let row = &v.rows[0];
        assert_eq!((row.cells[0].as_deref(), row.cells[1].as_deref()), (Some("174.0 lb"), Some("84.4 cm")));
        assert_eq!(row.values.len(), row.cells.len());
        let (w, c) = (row.values[0].as_ref().unwrap(), row.values[1].as_ref().unwrap());
        assert_eq!((w.unit, c.unit), (UnitIn::Lb, UnitIn::Cm));
        assert!((w.value - 174.04).abs() < 1e-9 && (c.value - 84.37).abs() < 1e-9, "{} {}", w.value, c.value);
        // saving the prefilled numbers back leaves the stored values where they were
        let stored = |j: &mut Journal, m: &str| j.month("2026-11").unwrap().body.iter().find(|e| e.measure == m && e.deleted_at.is_none()).unwrap().value;
        let (kg0, cm0) = (stored(&mut j, "weight"), stored(&mut j, "waist"));
        let set = [DayValue { measure: "weight".into(), value: Some(w.value), unit: w.unit }, DayValue { measure: "waist".into(), value: Some(c.value), unit: c.unit }];
        set_day(&mut j, TODAY, TODAY, &set, true, 3).unwrap();
        assert!((stored(&mut j, "weight") - kg0).abs() < 1e-9 && (stored(&mut j, "waist") - cm0).abs() < 1e-9);
        // a day without the measure has no number either
        log(&mut j, "2026-11-10", TODAY, "weight", 180.0, UnitIn::Lb, true, 4).unwrap();
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert!(v.rows[1].cells[1].is_none() && v.rows[1].values[1].is_none());
        let _ = fs::remove_dir_all(&d);
    }

    /// A journal holding daily weigh-ins `kg(i)` for the `n` days ending TODAY (i counts from 0 on the first day).
    fn daily(name: &str, n: i32, kg: impl Fn(i32) -> f64) -> (Journal, std::path::PathBuf) {
        let d = tdir(name);
        let mut j = Journal::open(d.clone());
        let end = diary::day_number(TODAY).unwrap();
        for i in 0..n {
            set_value(&mut j, &day_key(end - (n - 1) + i), TODAY, "weight", kg(i), 1).unwrap();
        }
        (j, d)
    }

    #[test]
    fn slope_least_squares() {
        let falling: Vec<(i32, f64)> = (0..5).map(|i| (i * 7, 80.0 - 0.3 * i as f64)).collect();
        assert!((slope(&falling).unwrap() * 7.0 + 0.3).abs() < 0.01, "{:?}", slope(&falling));
        assert!(slope(&[(3, 80.0)]).is_none() && slope(&[]).is_none());
        assert!(slope(&[(3, 80.0), (3, 81.0)]).is_none(), "no spread in time");
    }

    #[test]
    fn headline_rate() {
        // 20 daily weigh-ins falling 0.07 kg a day: the smoothed trend follows at about 0.3 kg a week
        let (mut j, d) = daily("rate", 20, |i| 80.0 - 0.07 * i as f64);
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        let h = v.headline.unwrap();
        assert!(h.starts_with("Trend 79.") && h.ends_with(", \u{2212}0.3 kg/week"), "{h}");
        assert_eq!(v.rate_text.as_deref(), Some("\u{2212}0.3 kg/week"));
        // the same in pounds
        j.profile_mut().unwrap().units.weight = MassUnit::Lb;
        let h = body_view(&mut j, BodyRange::M3, TODAY).unwrap().headline.unwrap();
        assert!(h.starts_with("Trend 17") && h.ends_with(" lb, \u{2212}0.7 lb/week"), "{h}");
        let _ = fs::remove_dir_all(&d);
        // flat: a rate under 0.05 prints 0.0
        let (mut j, d) = daily("rate-flat", 20, |i| 80.0 + 0.0001 * i as f64);
        assert_eq!(body_view(&mut j, BodyRange::M3, TODAY).unwrap().rate_text.as_deref(), Some("0.0 kg/week"));
        // too few: four weigh-ins, or five that don't span 14 days
        let (mut four, d4) = daily("rate-few", 4, |i| 80.0 - i as f64);
        assert_eq!(body_view(&mut four, BodyRange::M3, TODAY).unwrap().rate_text.as_deref(), Some("Log a few more to see a rate."));
        let (mut ten, d10) = daily("rate-short", 10, |i| 80.0 - i as f64);
        assert_eq!(body_view(&mut ten, BodyRange::M3, TODAY).unwrap().rate_text.as_deref(), Some("Log a few more to see a rate."));
        for p in [d, d4, d10] {
            let _ = fs::remove_dir_all(&p);
        }
    }

    #[test]
    fn chart_points_and_gaps() {
        let d = tdir("chart-gaps");
        let mut j = Journal::open(d.clone());
        // M3 is 91 days: 2026-08-19 to 2026-11-17, both included
        for (day, kg) in [("2026-08-18", 85.0), ("2026-08-19", 81.0), ("2026-08-20", 80.5), ("2026-08-28", 80.0), (TODAY, 79.0)] {
            set_value(&mut j, day, TODAY, "weight", kg, 1).unwrap();
        }
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        let n = |s: &str| diary::day_number(s).unwrap();
        assert_eq!((c.x_min, c.x_max), (n("2026-08-19"), n(TODAY)));
        assert_eq!(c.points.iter().map(|p| p.day).collect::<Vec<_>>(), [n("2026-08-19"), n("2026-08-20"), n("2026-08-28"), n(TODAY)], "ascending, none for the days between, the day before the range left out");
        assert!(c.points.iter().all(|p| p.raw.is_some() && p.trend.is_some()));
        assert_eq!(c.points[0].raw, Some(81.0));
        assert_eq!(c.table.len(), 4);
        assert_eq!((c.table[0].date_text.as_str(), c.table[0].raw_text.as_str()), ("Wed, Aug 19", "81.0 kg"));
        assert!(c.empty_text.is_none() && c.unit_text == "kg");
        // 2 weeks shows only the last two weeks
        assert_eq!(weight_chart(&mut j, BodyRange::W2, TODAY).points.len(), 1);
        assert_eq!(weight_chart(&mut j, BodyRange::All, TODAY).points.len(), 5);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn chart_trend_uses_all_history() {
        let d = tdir("chart-history");
        let mut j = Journal::open(d.clone());
        set_value(&mut j, "2026-08-10", TODAY, "weight", 90.0, 1).unwrap();
        set_value(&mut j, "2026-08-19", TODAY, "weight", 80.0, 1).unwrap();
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        assert_eq!(c.points.len(), 1, "the first weigh-in is before the range");
        // 90 + (1 - 0.9^9) x (80 - 90) = 83.874, not 80 as a restart would give
        assert!((c.points[0].trend.unwrap() - 83.8739).abs() < 1e-3, "{:?}", c.points[0]);
        assert_eq!(c.points[0].raw, Some(80.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn chart_goal_and_empty_states() {
        let d = tdir("chart-goal");
        let mut j = Journal::open(d.clone());
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        assert!(c.points.is_empty() && c.table.is_empty());
        assert_eq!(c.empty_text, Some("No weigh-ins in this range. Try a longer range."));
        assert_eq!((c.goal, c.label.as_str()), (None, "Weight, last 3 months: no weigh-ins."));
        j.profile_mut().unwrap().goal_weight_kg = Some(74.0);
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        assert_eq!((c.goal, c.goal_text.as_deref()), (Some(74.0), Some("Goal 74 kg")));
        assert!(c.y_min < 74.0 && 74.0 < c.y_max, "the goal line is always on the chart");
        // one weigh-in: one point, its trend is itself, no rate
        set_value(&mut j, TODAY, TODAY, "weight", 78.9, 1).unwrap();
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        assert_eq!(c.points, [Point { day: diary::day_number(TODAY).unwrap(), raw: Some(78.9), trend: Some(78.9) }]);
        assert_eq!(c.label, "Weight, last 3 months: trend 78.9 kg.");
        // the domain covers goal and value, padded by max(0.5, 5% of the span)
        assert!((c.y_min - 73.5).abs() < 1e-9 && (c.y_max - 79.4).abs() < 1e-9, "{} {}", c.y_min, c.y_max);
        // pounds: every number is in the person's unit
        j.profile_mut().unwrap().units.weight = MassUnit::Lb;
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        assert!((c.points[0].raw.unwrap() - 78.9 / goals::KG_PER_LB).abs() < 1e-9);
        assert_eq!((c.unit_text, c.goal_text.as_deref()), ("lb", Some("Goal 163.1 lb")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_far_goal_sits_on_the_edge_of_the_weight_chart() {
        let d = tdir("chart-far-goal");
        let mut j = Journal::open(d.clone());
        j.profile_mut().unwrap().goal_weight_kg = Some(65.0);
        set_value(&mut j, TODAY, TODAY, "weight", 80.0, 1).unwrap();
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        // 15 kg below a single weigh-in: the scale fits the weigh-in, the line sits on the bottom edge and says where the goal is
        assert!(c.y_min > 78.0, "the scale fits 80 kg, not the 65 kg goal: {}", c.y_min);
        assert_eq!(c.goal, Some(c.y_min));
        assert_eq!(c.goal_text.as_deref(), Some("Goal 65 kg \u{2193}"));
        // a goal above, as far off: the top edge, pointing up
        j.profile_mut().unwrap().goal_weight_kg = Some(95.0);
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        assert_eq!((c.goal, c.goal_text.as_deref()), (Some(c.y_max), Some("Goal 95 kg \u{2191}")));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn chart_label_names_the_trend_and_rate() {
        let (mut j, d) = daily("chart-label", 20, |i| 80.0 - 0.07 * i as f64);
        let c = weight_chart(&mut j, BodyRange::M3, TODAY);
        assert!(c.label.starts_with("Weight, last 3 months: trend from 80.0 to 79.") && c.label.ends_with(" kg, \u{2212}0.3 kg/week."), "{}", c.label);
        let _ = fs::remove_dir_all(&d);
    }

    fn with_goal(j: &mut Journal, goal: f64, kind: goals::GoalKind) {
        let p = j.profile_mut().unwrap();
        p.goal_weight_kg = Some(goal);
        p.goal = kind;
    }

    #[test]
    fn forecast_rules() {
        use goals::GoalKind::{Lose, Maintain};
        // no goal weight: no line
        let (mut j, d) = daily("fc-none", 20, |i| 80.0 - 0.1 * i as f64);
        assert_eq!(forecast(&mut j, TODAY), None);
        // too few weigh-ins, or too short a span
        with_goal(&mut j, 74.0, Lose);
        let (mut few, d4) = daily("fc-few", 4, |i| 80.0 - i as f64);
        with_goal(&mut few, 74.0, Lose);
        assert_eq!(forecast(&mut few, TODAY).as_deref(), Some("Not enough data yet."));
        let (mut short, d10) = daily("fc-short", 10, |i| 80.0 - i as f64);
        with_goal(&mut short, 74.0, Lose);
        assert_eq!(forecast(&mut short, TODAY).as_deref(), Some("Not enough data yet."));
        // a date, in the same year (falling 0.1 kg a day toward 78: about three weeks), with the year when it is another
        let (mut near, dn) = daily("fc-date", 17, |i| 80.0 - 0.1 * i as f64);
        with_goal(&mut near, 78.0, Lose);
        let said = forecast(&mut near, TODAY).unwrap();
        assert!(said.starts_with("At this pace you reach 78.0 kg around Dec ") && said.ends_with('.') && !said.contains("2027"), "{said}");
        with_goal(&mut near, 70.0, Lose);
        let said = forecast(&mut near, TODAY).unwrap();
        assert!(said.starts_with("At this pace you reach 70.0 kg around ") && said.ends_with(", 2027."), "{said}");
        // flat: a slope far under 0.05 kg a week
        let (mut flat, df) = daily("fc-flat", 17, |i| 80.0 + 0.0005 * i as f64);
        with_goal(&mut flat, 74.0, Lose);
        assert_eq!(forecast(&mut flat, TODAY).as_deref(), Some("Your trend is flat."));
        // already at or past the goal in the direction wanted
        let (mut past, dp) = daily("fc-past", 17, |_| 73.0);
        with_goal(&mut past, 74.0, Lose);
        assert_eq!(forecast(&mut past, TODAY).as_deref(), Some("Your trend is at your goal weight."));
        // with no goal kind chosen, the side the trend started on decides
        with_goal(&mut past, 74.0, Maintain);
        assert_eq!(forecast(&mut past, TODAY).as_deref(), Some("Your trend is flat."), "1 kg below 74 and not moving, started below: not at a goal it hasn't reached");
        // more than two years
        let (mut slow, ds) = daily("fc-slow", 17, |i| 80.0 - 0.12 / 7.0 * i as f64);
        with_goal(&mut slow, 60.0, Lose);
        assert_eq!(forecast(&mut slow, TODAY).as_deref(), Some("More than 2 years at this pace."));
        // moving away from the goal says nothing about a date
        let (mut away, da) = daily("fc-away", 17, |i| 80.0 + 0.2 * i as f64);
        with_goal(&mut away, 74.0, Lose);
        assert_eq!(forecast(&mut away, TODAY).as_deref(), Some("Your trend isn't heading toward your goal yet."));
        for p in [d, d4, d10, dn, df, dp, ds, da] {
            let _ = fs::remove_dir_all(&p);
        }
    }

    #[test]
    fn body_view_carries_the_forecast_and_chart() {
        let (mut j, d) = daily("view-fc", 17, |i| 80.0 - 0.1 * i as f64);
        with_goal(&mut j, 78.0, goals::GoalKind::Lose);
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert!(v.forecast.unwrap().starts_with("At this pace"));
        assert_eq!(v.chart.unwrap().points.len(), 17);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn custom_measure_rules() {
        let d = tdir("custom-rules");
        let mut j = Journal::open(d.clone());
        let uid = save_measure(&mut j, "  Calf ", MeasureKind::Length).unwrap();
        assert_eq!(j.profile().unwrap().measures, [CustomMeasure { uid: uid.clone(), name: "Calf".into(), kind: MeasureKind::Length, archived: false }]);
        assert_eq!(save_measure(&mut j, "calf", MeasureKind::Length).unwrap_err(), "You already have a measure with that name.");
        assert_eq!(save_measure(&mut j, "Waist", MeasureKind::Length).unwrap_err(), "You already have a measure with that name.");
        assert_eq!(save_measure(&mut j, "   ", MeasureKind::Number).unwrap_err(), "Give the measure a name.");
        let long = save_measure(&mut j, &"x".repeat(41), MeasureKind::Number).unwrap();
        assert_eq!(j.profile().unwrap().measures.iter().find(|m| m.uid == long).unwrap().name.chars().count(), 40);
        // a custom measure logs in its kind's unit and gets a column
        let l = log(&mut j, TODAY, TODAY, &uid, 38.0, UnitIn::Cm, false, 1).unwrap();
        assert_eq!(l.text, "38.0 cm");
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert_eq!(v.columns.iter().map(|c| (c.name.as_str(), c.unit_text)).collect::<Vec<_>>(), [("Weight", "kg"), ("Calf", "cm")]);
        assert_eq!(v.measures.len(), 10);
        // at most 30 of their own
        for i in 3..=30 {
            save_measure(&mut j, &format!("Measure {i}"), MeasureKind::Number).unwrap();
        }
        assert_eq!(save_measure(&mut j, "One too many", MeasureKind::Number).unwrap_err(), "You can keep up to 30 measures of your own.");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn archive_hides_and_keeps() {
        let d = tdir("archive");
        let mut j = Journal::open(d.clone());
        let uid = save_measure(&mut j, "Calf", MeasureKind::Length).unwrap();
        log(&mut j, TODAY, TODAY, &uid, 38.0, UnitIn::Cm, false, 1).unwrap();
        assert_eq!(archive_measure(&mut j, &uid, true).unwrap(), "Calf");
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert_eq!(v.columns.len(), 1, "its column is hidden");
        assert!(v.measures.iter().all(|m| m.id != uid) && v.archived.iter().map(|m| m.name.as_str()).collect::<Vec<_>>() == ["Calf"]);
        assert!(!v.empty && v.rows.is_empty(), "the value is kept, just not shown");
        assert_eq!(j.month("2026-11").unwrap().body.iter().filter(|e| e.measure == uid && e.deleted_at.is_none()).count(), 1);
        assert_eq!(log(&mut j, TODAY, TODAY, &uid, 39.0, UnitIn::Cm, false, 2).unwrap_err(), NO_MEASURE);
        // restoring brings back the column, the picker entry and the value
        archive_measure(&mut j, &uid, false).unwrap();
        let v = body_view(&mut j, BodyRange::M3, TODAY).unwrap();
        assert_eq!((v.columns.len(), v.archived.len(), v.rows[0].cells[1].as_deref()), (2, 0, Some("38.0 cm")));
        assert_eq!(archive_measure(&mut j, "nope", true).unwrap_err(), NO_MEASURE);
        let _ = fs::remove_dir_all(&d);
    }

    fn dv(measure: &str, value: Option<f64>, unit: UnitIn) -> DayValue {
        DayValue { measure: measure.into(), value, unit }
    }

    #[test]
    fn set_day_one_undo() {
        let d = tdir("set-day");
        let mut j = Journal::open(d.clone());
        let day = "2026-11-15";
        log(&mut j, day, TODAY, "weight", 80.0, UnitIn::Kg, true, 1).unwrap();
        log(&mut j, day, TODAY, "waist", 90.0, UnitIn::Cm, true, 1).unwrap();
        let r = set_day(&mut j, day, TODAY, &[dv("weight", Some(78.9), UnitIn::Kg), dv("waist", None, UnitIn::Cm), dv("chest", Some(98.0), UnitIn::Cm)], false, 50).unwrap();
        assert_eq!((r.confirm.clone(), r.text.as_str()), (None, "Nov 15"));
        let u = r.undo.unwrap();
        assert_eq!((u.before_body.len(), u.added_body.len()), (2, 1));
        let live = |j: &mut Journal, m: &str| j.month("2026-11").unwrap().body.iter().find(|e| e.measure == m && e.deleted_at.is_none()).map(|e| e.value);
        assert_eq!((live(&mut j, "weight"), live(&mut j, "waist"), live(&mut j, "chest")), (Some(78.9), None, Some(98.0)));
        assert!(j.month("2026-11").unwrap().body.iter().any(|e| e.measure == "waist" && e.deleted_at == Some(50)), "cleared is soft-deleted, not removed");
        // one undo puts all three back
        diary::undo(&mut j, &u, 60).unwrap();
        assert_eq!((live(&mut j, "weight"), live(&mut j, "waist"), live(&mut j, "chest")), (Some(80.0), Some(90.0), None));
        // clearing something that isn't there changes nothing and keeps no undo
        let none = set_day(&mut j, day, TODAY, &[dv("neck", None, UnitIn::Cm)], false, 70).unwrap();
        assert!(none.undo.is_none());
        // one bad value stops the whole save before anything is written
        let before = j.month("2026-11").unwrap().body.clone();
        assert_eq!(set_day(&mut j, day, TODAY, &[dv("hips", Some(95.0), UnitIn::Cm), dv("waist", Some(0.0), UnitIn::Cm)], false, 80).unwrap_err(), BAD_LENGTH);
        assert_eq!(set_day(&mut j, day, TODAY, &[dv("hips", Some(95.0), UnitIn::Cm), dv("hips", Some(96.0), UnitIn::Cm)], false, 80).unwrap_err(), "Each measure can be changed once in one save.");
        assert_eq!(set_day(&mut j, "2026-11-18", TODAY, &[dv("hips", Some(95.0), UnitIn::Cm)], false, 80).unwrap_err(), FUTURE_DAY);
        assert_eq!(j.month("2026-11").unwrap().body, before);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn set_day_asks_once_for_weight() {
        let d = tdir("set-day-ask");
        let mut j = Journal::open(d.clone());
        put(&mut j, "2026-11-10", 148.9);
        let set = [dv("weight", Some(78.9), UnitIn::Kg), dv("waist", Some(80.0), UnitIn::Cm)];
        let ask = set_day(&mut j, TODAY, TODAY, &set, false, 5).unwrap();
        assert_eq!(ask.confirm.as_deref(), Some("That's 70 kg less than your last weigh-in. Save anyway?"));
        assert!(ask.undo.is_none());
        assert_eq!(j.month("2026-11").unwrap().body.len(), 1, "not even the waist was written");
        let yes = set_day(&mut j, TODAY, TODAY, &set, true, 6).unwrap();
        assert!(yes.confirm.is_none());
        assert_eq!(j.month("2026-11").unwrap().body.len(), 3);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn delete_value_and_undo() {
        let d = tdir("delete-value");
        let mut j = Journal::open(d.clone());
        log(&mut j, TODAY, TODAY, "waist", 84.0, UnitIn::Cm, false, 1).unwrap();
        let (name, u) = delete_value(&mut j, TODAY, "waist", 9).unwrap();
        assert_eq!(name, "Waist");
        assert!(body_view(&mut j, BodyRange::M3, TODAY).unwrap().empty);
        assert!(delete_value(&mut j, TODAY, "waist", 10).is_err(), "nothing left to delete");
        diary::undo(&mut j, &u, 11).unwrap();
        assert_eq!(body_view(&mut j, BodyRange::M3, TODAY).unwrap().rows[0].cells.len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_custom_measure_works_until_it_is_archived() {
        let d = tdir("custom");
        let mut j = Journal::open(d.clone());
        j.profile_mut().unwrap().measures.push(CustomMeasure { uid: "c1".into(), name: "Steps".into(), kind: MeasureKind::Number, archived: false });
        assert!(set_value(&mut j, TODAY, TODAY, "c1", 8000.0, 1).is_ok());
        j.profile_mut().unwrap().measures[0].archived = true;
        assert_eq!(set_value(&mut j, TODAY, TODAY, "c1", 9000.0, 2).map(|_| ()), Err(NO_MEASURE.into()));
        let _ = fs::remove_dir_all(&d);
    }
}
