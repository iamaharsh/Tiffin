//! The daily calorie target and macro goals, worked out from the profile and the body log. Every clamp lives
//! here in the crate, never in a UI, so a later web or mobile build shows the same numbers and sentences.

use crate::model;
use crate::tracker::body;
use crate::tracker::day::Day;
use crate::tracker::diary;
use crate::tracker::journal::Profile;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Sex for the resting-energy formula (the offset) and the lowest target.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
    Other,
}

/// How active an ordinary day is, without workouts (they are logged and added on top).
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Sitting,
    Light,
    OnFeet,
    Physical,
}

impl Activity {
    /// The multiplier on resting energy.
    pub fn factor(self) -> f64 {
        match self {
            Activity::Sitting => 1.2,
            Activity::Light => 1.375,
            Activity::OnFeet => 1.55,
            Activity::Physical => 1.725,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Activity::Sitting => "Mostly sitting",
            Activity::Light => "Light movement",
            Activity::OnFeet => "On your feet",
            Activity::Physical => "Physical work",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Activity::Sitting => "Desk, study or home most of the day",
            Activity::Light => "On your feet some of the day",
            Activity::OnFeet => "On your feet most of the day",
            Activity::Physical => "Hard physical work most of the day",
        }
    }
}

/// What the person wants their weight to do.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalKind {
    Lose,
    #[default]
    Maintain,
    Gain,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MassUnit {
    #[default]
    Kg,
    Lb,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HeightUnit {
    #[default]
    Cm,
    FtIn,
}

/// The units the person types and reads in (D-02). Values are stored in kg and cm whatever these say.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct UnitChoice {
    pub height: HeightUnit,
    pub weight: MassUnit,
    pub rate: MassUnit,
    pub goal_weight: MassUnit,
}

/// How one macro is set. Stored as `{"mode": "grams", "value": 150}`; `rest` has no value.
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub enum MacroMode {
    /// A fixed number of grams; the percent follows the calories.
    Grams(f64),
    /// A share of the day's calories; the grams follow the calories.
    Percent(f64),
    /// Grams for each kg of body weight (protein only).
    PerKg(f64),
    /// Whatever calories the other two leave (carbs or fat only, one of them).
    Rest,
}

/// The three macro settings. A new one is the Balanced preset (20/50/30).
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct MacroSpec {
    pub protein: MacroMode,
    pub carbs: MacroMode,
    pub fat: MacroMode,
}

impl Default for MacroSpec {
    fn default() -> Self {
        PRESETS[0].1
    }
}

/// A named kind of day: a calorie offset from the base target and its own macros (D-11).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Template {
    pub id: u32,
    pub name: String,
    pub kcal_offset: i32,
    pub macros: MacroSpec,
}

/// One dated, complete set of targets. The target in force on a day is the last record whose `effective_from`
/// is on or before it, so changing anything later never touches an earlier day (GOAL-06).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct TargetSet {
    /// `YYYY-MM-DD`, checked with `Day::parse` when it is saved.
    pub effective_from: String,
    /// The base target in force, already clamped. Every consumer reads this one number.
    pub kcal: i32,
    /// What the calculation gave, kept for reference when `manual_kcal` is the number in force.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculated_kcal: Option<i32>,
    /// The person's own number (D-05), when they chose one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual_kcal: Option<i32>,
    /// The lowest a day's number can be in this record (a weekday template is raised to it).
    pub floor_kcal: i32,
    /// The weight the calorie target was built on (the D-07 drift check). For Grams per kg it is only the
    /// fallback for a day with no weigh-in on or before it, never a replacement for the day's own weight (D-09).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis_weight_kg: Option<f64>,
    pub macros: MacroSpec,
    pub templates: Vec<Template>,
    /// The template id for each weekday; index 0 is Sunday, as `model::weekday`.
    pub week: [Option<u32>; 7],
    /// How much of a logged workout's energy is added back to the day, in percent (D-16).
    pub addback_pct: u8,
}

impl Default for TargetSet {
    fn default() -> Self {
        TargetSet {
            effective_from: String::new(),
            kcal: 0,
            calculated_kcal: None,
            manual_kcal: None,
            floor_kcal: 0,
            basis_weight_kg: None,
            macros: MacroSpec::default(),
            templates: vec![],
            week: [None; 7],
            addback_pct: 100,
        }
    }
}

/// Resting energy by the Mifflin-St Jeor formula. The Other offset is the midpoint of the two (D-03).
pub fn bmr(kg: f64, cm: f64, age: u32, sex: Sex) -> f64 {
    let s = match sex {
        Sex::Male => 5.0,
        Sex::Female => -161.0,
        Sex::Other => -78.0,
    };
    10.0 * kg + 6.25 * cm - 5.0 * age as f64 + s
}

/// Whole years between a birth date and `today`. Both come from the caller, never the clock; a birth date
/// after today is refused.
pub fn age_on(birth: &str, today: &str) -> Result<u32, String> {
    let (b, t) = (Day::parse(birth)?, Day::parse(today)?);
    if b.as_str() > t.as_str() {
        return Err("The birth date is in the future.".into());
    }
    let (b, t) = (b.as_str(), t.as_str());
    let years = t[..4].parse::<u32>().unwrap_or(0) - b[..4].parse::<u32>().unwrap_or(0);
    // not yet had this year's birthday: month-day text compares like the date
    Ok(if t[5..] < b[5..] { years - 1 } else { years })
}

/// One printed line of the working.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Line {
    pub label: String,
    pub value: String,
}

/// Why a request was changed.
#[derive(Clone, Copy, Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdjustKind {
    Age,
    BmiGuard,
    RateCap,
    Floor,
    Manual,
}

/// One change the engine made to what was asked: the request, what it used, and the reason in a full sentence.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Adjustment {
    pub kind: AdjustKind,
    pub original: String,
    pub adjusted: String,
    pub reason: String,
}

/// A thing wrong with an input, tied to the field it belongs to.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Problem {
    pub field: String,
    pub text: String,
}

/// A worked-out calorie target and how it was reached.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Breakdown {
    /// Always Resting energy, Daily activity, Maintenance, Goal, Your target, in that order.
    pub lines: Vec<Line>,
    /// In the order the clamps ran: age, BMI guard, rate cap, floor.
    pub adjustments: Vec<Adjustment>,
    pub bmr: f64,
    pub maintenance: i32,
    pub kcal: i32,
    pub floor_kcal: i32,
    /// The weekly change actually used, in kg.
    pub rate_kg_week: f64,
}

pub const UNDER_18: &str = "Tiffin can't work out a calorie target for anyone under 18. A doctor or dietitian can help with that.";
pub const BMI_GUARD: &str = "Your BMI is under 18.5, so Tiffin won't set a weight-loss target. Your target is set to keep your weight.";
/// `{why}` is " (about 1% of your weight)", " (the pace for a BMI under 25)" or nothing, whichever rule set the cap.
pub const RATE_CAP: &str = "You asked to lose {req} a week. Tiffin limits loss to {cap} a week{why}, in line with the 0.5 to 1 kg (1 to 2 lb) a week that the CDC and NHS describe as a steady pace. Your target uses {cap} a week.";
pub const GAIN_CAP: &str = "You asked to gain {req} a week. Tiffin limits gain to {cap} a week. Your target uses {cap} a week.";
pub const FLOOR: &str = "That request works out to {raw} kcal a day. Tiffin doesn't set a target below {floor} kcal without a health professional, so your target is {floor} kcal. If you are worried about your eating, a doctor or dietitian can help.";
/// The floor case where the person's own maintenance is under the usual floor, so maintenance is the target.
pub const FLOOR_AT_MAINTENANCE: &str = "That request works out to {raw} kcal a day, under the {floor} kcal Tiffin sets as its lowest without a health professional. Your maintenance is {used} kcal, so your target is {used} kcal to keep your weight. If you are worried about your eating, a doctor or dietitian can help.";
pub const MANUAL_BELOW_FLOOR: &str = "Your own number, {raw} kcal, is below {floor} kcal, so Tiffin uses {floor} kcal.";
pub const OTHER_NOTE: &str = "Other uses the midpoint of the two formulas. The minimum target is the higher one, 1,500 kcal.";
pub const ACTIVITY_LEAD: &str = "This is your ordinary day without workouts. Log workouts in the Diary; they're added on top.";

/// Exactly 0.45359237 kg to the pound and 2.54 cm to the inch; values are stored in kg and cm and converted only at the edge.
pub const KG_PER_LB: f64 = 0.45359237;
pub const CM_PER_IN: f64 = 2.54;

pub fn other_note(sex: Sex) -> Option<&'static str> {
    (sex == Sex::Other).then_some(OTHER_NOTE)
}

/// The lowest target for a person: the sex's own floor (the higher one for Other) or 0.8 x resting energy,
/// whichever is higher.
pub(crate) fn floor_kcal(sex: Sex, bmr: f64) -> i32 {
    let sex_floor: f64 = match sex {
        Sex::Female => 1200.0,
        Sex::Male | Sex::Other => 1500.0,
    };
    sex_floor.max(0.8 * bmr).round() as i32
}

/// A number the person chose themselves (D-05). It is raised to the floor, with no maintenance guard because it
/// is not a deficit request, and the original number is kept in the adjustment. Under 18 the floor still applies
/// and the under-18 sentence comes first. `age` is None when no birth date is known.
pub fn manual_target(kcal: f64, sex: Sex, bmr: f64, age: Option<u32>) -> Result<(i32, Vec<Adjustment>), String> {
    if !kcal.is_finite() || !(1.0..=10_000.0).contains(&kcal) {
        return Err("Enter a number of calories from 1 to 10,000.".into());
    }
    let (raw, floor) = (kcal.round() as i32, floor_kcal(sex, bmr));
    let used = raw.max(floor);
    let adj = |kind, reason: &str| Adjustment { kind, original: format!("{} kcal", kcal_text(raw)), adjusted: format!("{} kcal", kcal_text(used)), reason: reason.into() };
    let mut out = vec![];
    if age.is_some_and(|a| a < 18) {
        out.push(adj(AdjustKind::Age, UNDER_18));
    }
    if raw < floor {
        out.push(adj(AdjustKind::Manual, &MANUAL_BELOW_FLOOR.replace("{raw}", &kcal_text(raw)).replace("{floor}", &kcal_text(floor))));
    }
    Ok((used, out))
}

pub fn kg_from(value: f64, unit: MassUnit) -> f64 {
    match unit {
        MassUnit::Kg => value,
        MassUnit::Lb => value * KG_PER_LB,
    }
}

pub fn in_unit(kg: f64, unit: MassUnit) -> f64 {
    match unit {
        MassUnit::Kg => kg,
        MassUnit::Lb => kg / KG_PER_LB,
    }
}

fn unit_word(unit: MassUnit) -> &'static str {
    match unit {
        MassUnit::Kg => "kg",
        MassUnit::Lb => "lb",
    }
}

/// A number with at most `decimals` places and no trailing zeros: 0.50 is "0.5", 1.00 is "1".
fn trimmed(v: f64, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// A weight with one decimal and its unit, "180.0 lb". Only the text is rounded; stored values are not.
pub fn mass_text(kg: f64, unit: MassUnit) -> String {
    format!("{:.1} {}", in_unit(kg, unit), unit_word(unit))
}

/// A weekly rate, up to two decimals with the unit, "0.75 kg".
pub fn rate_text(kg: f64, unit: MassUnit) -> String {
    format!("{} {}", trimmed(in_unit(kg, unit), 2), unit_word(unit))
}

pub fn cm_from_ft_in(ft: f64, inches: f64) -> f64 {
    (ft * 12.0 + inches) * CM_PER_IN
}

/// Feet and inches, the inches to 0.1; an inch count that rounds up to 12 carries into the feet.
pub fn ft_in_parts(cm: f64) -> (u32, f64) {
    let tenths = (cm / CM_PER_IN * 10.0).round().max(0.0) as u32;
    (tenths / 120, (tenths % 120) as f64 / 10.0)
}

pub fn ft_in_text(cm: f64) -> String {
    let (ft, inches) = ft_in_parts(cm);
    format!("{ft} ft {} in", trimmed(inches, 1))
}

pub fn height_text(cm: f64, unit: HeightUnit) -> String {
    match unit {
        HeightUnit::Cm => format!("{} cm", trimmed(cm, 1)),
        HeightUnit::FtIn => ft_in_text(cm),
    }
}

/// Whole kcal with thousands separators, "2,475".
pub fn kcal_text(kcal: i32) -> String {
    let digits = kcal.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if kcal < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// A signed change, "+550" or "−550" with a true minus, as the Diary prints exercise.
pub fn signed_kcal_text(kcal: i32) -> String {
    if kcal >= 0 {
        format!("+{}", kcal_text(kcal))
    } else {
        format!("\u{2212}{}", kcal_text(kcal).trim_start_matches('-'))
    }
}

/// A height the engine will work with: finite and 50 to 272 cm.
pub fn check_height_cm(cm: f64) -> Result<f64, String> {
    if cm.is_finite() && (50.0..=272.0).contains(&cm) {
        Ok(cm)
    } else {
        Err("That height isn't possible. Enter a height from 50 to 272 cm.".into())
    }
}

pub(crate) fn problem(field: &str, text: String) -> Problem {
    Problem { field: field.into(), text }
}

const LOSE_CEILING_KG: f64 = 1.0;
const GAIN_CEILING_KG: f64 = 0.5;
const KCAL_PER_KG: f64 = 7700.0;

/// The calorie target for a profile and a weight, as of `today`. With anything missing it returns no target and
/// one problem per missing field; nothing is ever worked out from a default. Otherwise the clamps run in this
/// order: age, BMI guard, rate cap, floor, and each one that changes the request is returned as an
/// [`Adjustment`] with what was asked, what was used and why. The maths runs unrounded and each printed kcal is
/// rounded once, so the printed lines add up.
pub fn compute(p: &Profile, weight_kg: Option<f64>, today: &str) -> Result<Breakdown, Vec<Problem>> {
    let mut problems = vec![];
    let mut need = |ok: bool, field: &str| {
        if !ok {
            problems.push(problem(field, format!("Enter your {field}.")));
        }
    };
    need(p.sex.is_some(), "sex");
    need(p.birth_date.is_some(), "birth date");
    need(p.height_cm.is_some(), "height");
    need(p.activity.is_some(), "activity level");
    need(weight_kg.is_some(), "weight");
    if let Some(cm) = p.height_cm {
        if let Err(e) = check_height_cm(cm) {
            problems.push(problem("height", e));
        }
    }
    if let Some(kg) = weight_kg {
        if let Err(e) = body::check_value(body::MeasureKind::Mass, kg) {
            problems.push(problem("weight", e.into()));
        }
    }
    if p.goal != GoalKind::Maintain && !(p.rate_kg_week.is_finite() && (0.0..=2.0).contains(&p.rate_kg_week)) {
        problems.push(problem("rate", "That weekly rate isn't possible. Enter a number from 0 to 2 kg a week.".into()));
    }
    let mut age = None;
    if let Some(birth) = &p.birth_date {
        match age_on(birth, today) {
            Ok(_) if birth.as_str() < "1900-01-01" => problems.push(problem("birth date", "The birth date isn't possible. Enter a date from 1900 on.".into())),
            Ok(a) if a < 18 => problems.push(problem("birth date", UNDER_18.into())),
            Ok(a) => age = Some(a),
            Err(e) => problems.push(problem("birth date", e)),
        }
    }
    let (Some(sex), Some(age), Some(cm), Some(activity), Some(kg), true) = (p.sex, age, p.height_cm, p.activity, weight_kg, problems.is_empty()) else {
        return Err(problems);
    };

    let resting = bmr(kg, cm, age, sex);
    let maintenance = (resting * activity.factor()).round() as i32;
    let floor = floor_kcal(sex, resting);
    let unit = p.units.rate;
    let per_week = |kg: f64| format!("{} a week", rate_text(kg, unit));
    let m = cm / 100.0;
    let bmi = kg / (m * m);
    let mut adjustments = vec![];
    let (mut goal, mut rate) = (p.goal, p.rate_kg_week);

    if goal == GoalKind::Lose && bmi < 18.5 {
        adjustments.push(Adjustment { kind: AdjustKind::BmiGuard, original: format!("Lose {}", per_week(rate)), adjusted: "Keep your weight".into(), reason: BMI_GUARD.into() });
        goal = GoalKind::Maintain;
    }
    match goal {
        GoalKind::Lose => {
            let (pct, tier) = (0.01 * kg, if bmi < 25.0 { 0.5 } else { f64::INFINITY });
            let cap = LOSE_CEILING_KG.min(pct).min(tier);
            if rate > cap + 1e-9 {
                let why = if cap == tier {
                    " (the pace for a BMI under 25)"
                } else if cap == pct {
                    " (about 1% of your weight)"
                } else {
                    ""
                };
                let reason = RATE_CAP.replace("{req}", &rate_text(rate, unit)).replace("{cap}", &rate_text(cap, unit)).replace("{why}", why);
                adjustments.push(Adjustment { kind: AdjustKind::RateCap, original: per_week(rate), adjusted: per_week(cap), reason });
                rate = cap;
            }
        }
        GoalKind::Gain if rate > GAIN_CEILING_KG + 1e-9 => {
            let reason = GAIN_CAP.replace("{req}", &rate_text(rate, unit)).replace("{cap}", &rate_text(GAIN_CEILING_KG, unit));
            adjustments.push(Adjustment { kind: AdjustKind::RateCap, original: per_week(rate), adjusted: per_week(GAIN_CEILING_KG), reason });
            rate = GAIN_CEILING_KG;
        }
        GoalKind::Gain => {}
        GoalKind::Maintain => rate = 0.0,
    }

    let signed = match goal {
        GoalKind::Lose => -rate,
        _ => rate,
    };
    let requested = maintenance + (signed * KCAL_PER_KG / 7.0).round() as i32;
    // a loss is never raised above the person's own maintenance by the floor
    let (kcal, floor_kcal) = if goal == GoalKind::Lose {
        let guard = floor.min(maintenance);
        if requested < guard {
            let raw = kcal_text(requested);
            let reason = if maintenance < floor {
                FLOOR_AT_MAINTENANCE.replace("{raw}", &raw).replace("{floor}", &kcal_text(floor)).replace("{used}", &kcal_text(maintenance))
            } else {
                FLOOR.replace("{raw}", &raw).replace("{floor}", &kcal_text(guard))
            };
            adjustments.push(Adjustment { kind: AdjustKind::Floor, original: format!("{raw} kcal"), adjusted: format!("{} kcal", kcal_text(guard)), reason });
            (guard, guard)
        } else {
            (requested, guard)
        }
    } else {
        // the same guard, so a no-template day is never lifted above a Maintain or Gain target (WR-01)
        (requested, floor.min(maintenance))
    };

    let resting_printed = resting.round() as i32;
    let goal_label = match goal {
        GoalKind::Lose => format!("Goal: lose {}", per_week(rate)),
        GoalKind::Maintain => "Goal: keep your weight".to_string(),
        GoalKind::Gain => format!("Goal: gain {}", per_week(rate)),
    };
    let sex_label = if sex == Sex::Other { "Resting energy (Mifflin-St Jeor, midpoint)" } else { "Resting energy (Mifflin-St Jeor)" };
    let line = |label: String, value: String| Line { label, value };
    Ok(Breakdown {
        lines: vec![
            line(sex_label.into(), kcal_text(resting_printed)),
            line(format!("Daily activity (x {})", activity.factor()), kcal_text(maintenance - resting_printed)),
            line("Maintenance".into(), kcal_text(maintenance)),
            line(goal_label, signed_kcal_text(kcal - maintenance)),
            line("Your target".into(), kcal_text(kcal)),
        ],
        adjustments,
        bmr: resting,
        maintenance,
        kcal,
        floor_kcal,
        rate_kg_week: rate,
    })
}

pub const KCAL_PER_G_PROTEIN: f64 = 4.0;
pub const KCAL_PER_G_CARBS: f64 = 4.0;
pub const KCAL_PER_G_FAT: f64 = 9.0;

/// Shown on a weekday whose number was raised to the floor.
pub const FLOOR_DAY: &str = "Raised to {floor}, the lowest Tiffin sets.";
pub const BAD_WEIGHT_PER_KG: &str = "Add a weight to set protein per kg.";
pub const FAT_UNDER_20: &str = "Fat is under 20% of your calories. Set at least 20%.";
/// The most templates a record holds, and the longest a template name is kept.
pub const MAX_TEMPLATES: usize = 20;
pub const MAX_TEMPLATE_NAME: usize = 40;
/// The furthest a template can move a day from the base target, in kcal either way.
pub const MAX_OFFSET_KCAL: i32 = 2000;

const fn percent_plan(protein: f64, carbs: f64, fat: f64) -> MacroSpec {
    MacroSpec { protein: MacroMode::Percent(protein), carbs: MacroMode::Percent(carbs), fat: MacroMode::Percent(fat) }
}

/// Starting points that fill the three macros as percent and lock nothing (D-10).
pub const PRESETS: [(&str, MacroSpec); 4] = [
    ("Balanced", percent_plan(20.0, 50.0, 30.0)),
    ("High Protein", percent_plan(30.0, 40.0, 30.0)),
    ("Low Carb", percent_plan(30.0, 20.0, 50.0)),
    ("Keto", percent_plan(20.0, 5.0, 75.0)),
];

/// One macro, resolved to grams, percent and kcal together. The numbers are unrounded; the text is rounded once.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct MacroRow {
    pub name: &'static str,
    pub grams: f64,
    pub pct: f64,
    pub kcal: f64,
    /// "100 g · 20% · 400 kcal"
    pub readout: String,
    /// The screen-reader text, "100 g protein".
    pub said: String,
    pub problem: Option<String>,
}

/// The three macros for a day's calories: always Protein, Carbs, Fat.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct MacroResult {
    pub rows: [MacroRow; 3],
    /// "Total 2,000 kcal · 100%"
    pub total_text: String,
    pub problems: Vec<Problem>,
}

/// What one day is judged against.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct DayTarget {
    pub kcal: i32,
    /// The template in force that weekday: its id and name.
    pub template: Option<(u32, String)>,
    pub offset: i32,
    /// The number before it was raised to the floor.
    pub raised_from: Option<i32>,
    pub note: Option<String>,
    pub macros: MacroResult,
}

/// One weekday of the plan.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct WeekRow {
    pub label: &'static str,
    pub template: Option<u32>,
    pub template_name: Option<String>,
    /// "2,400 kcal"
    pub kcal_text: String,
    pub note: Option<String>,
}

const MACRO_NAMES: [&str; 3] = ["Protein", "Carbs", "Fat"];
const KCAL_PER_G: [f64; 3] = [KCAL_PER_G_PROTEIN, KCAL_PER_G_CARBS, KCAL_PER_G_FAT];

/// Turn a macro plan into grams, percent and kcal together at a day's calories (D-08, D-09). Grams stay put as
/// the calories change; Percent follows them; Grams per kg (protein only) is worked out from `basis_kg`; Rest
/// (carbs or fat, one of them) takes what the other two leave. Without a Rest the three shares must add up to
/// 100 within 0.5. Every problem is tied to the macro it belongs to, and nothing missing or not a number is
/// ever read as 0.
pub fn resolve_macros(spec: &MacroSpec, kcal: i32, basis_kg: Option<f64>) -> MacroResult {
    let modes = [spec.protein, spec.carbs, spec.fat];
    let total = kcal.max(0) as f64;
    let lower = |i: usize| MACRO_NAMES[i].to_lowercase();
    let mut grams = [0.0f64; 3];
    let mut bad: [Option<String>; 3] = [None, None, None];

    for i in 0..3 {
        let (value, allowed) = match modes[i] {
            MacroMode::Grams(v) | MacroMode::Percent(v) => (Some(v), true),
            MacroMode::PerKg(v) => (Some(v), i == 0),
            MacroMode::Rest => (None, i != 0),
        };
        if !allowed {
            bad[i] = Some(if i == 0 { "Protein can't fill the rest. Choose carbs or fat for that.".into() } else { "Grams per kg is only for protein.".to_string() });
        } else if value.is_some_and(|v| !v.is_finite() || v < 0.0) {
            bad[i] = Some(format!("Enter a number of 0 or more for {}.", lower(i)));
        } else {
            match modes[i] {
                MacroMode::Grams(v) => grams[i] = v,
                MacroMode::Percent(v) => grams[i] = total * v / 100.0 / KCAL_PER_G[i],
                MacroMode::PerKg(per_kg) => match basis_kg.filter(|w| w.is_finite() && *w > 0.0) {
                    Some(w) => grams[i] = per_kg * w,
                    None => bad[i] = Some(BAD_WEIGHT_PER_KG.into()),
                },
                MacroMode::Rest => {}
            }
        }
    }

    let rests: Vec<usize> = (1..3).filter(|&i| modes[i] == MacroMode::Rest).collect();
    if rests.len() == 2 {
        for i in rests {
            bad[i] = Some("Only one of carbs and fat can fill the rest.".into());
        }
    } else if let [r] = rests[..] {
        let used: f64 = (0..3).filter(|&i| i != r).map(|i| grams[i] * KCAL_PER_G[i]).sum();
        let left = total - used;
        if left < -1e-9 {
            let others: Vec<usize> = (0..3).filter(|&i| i != r).collect();
            bad[r] = Some(format!("{} and {} already use more than {} kcal.", MACRO_NAMES[others[0]], lower(others[1]), kcal_text(kcal)));
        } else {
            grams[r] = left.max(0.0) / KCAL_PER_G[r];
        }
    }

    let kcals: [f64; 3] = std::array::from_fn(|i| grams[i] * KCAL_PER_G[i]);
    let pcts: [f64; 3] = std::array::from_fn(|i| if total > 0.0 { kcals[i] / total * 100.0 } else { 0.0 });
    if total > 0.0 && bad.iter().all(Option::is_none) {
        let sum: f64 = pcts.iter().sum();
        if (sum - 100.0).abs() > 0.5 + 1e-9 {
            let text = format!("Percentages add up to {}%. They need to add up to 100%.", trimmed(sum, 1));
            bad[1] = Some(text.clone());
            bad[2] = Some(text);
        } else if pcts[2] < 20.0 - 1e-9 {
            bad[2] = Some(FAT_UNDER_20.into());
        }
    }

    let problems = (0..3).filter_map(|i| bad[i].clone().map(|text| Problem { field: lower(i), text })).collect();
    let rows = std::array::from_fn(|i| MacroRow {
        name: MACRO_NAMES[i],
        grams: grams[i],
        pct: pcts[i],
        kcal: kcals[i],
        readout: format!("{} g · {:.0}% · {} kcal", grams[i].round(), pcts[i], kcal_text(kcals[i].round() as i32)),
        said: format!("{} g {}", grams[i].round(), lower(i)),
        problem: bad[i].clone(),
    });
    MacroResult { rows, total_text: format!("Total {} kcal · {:.0}%", kcal_text(kcals.iter().sum::<f64>().round() as i32), pcts.iter().sum::<f64>()), problems }
}

/// One weekday of a record: its template (if the id still exists), the offset, and the day's number raised to the
/// floor with the number before the raise.
fn weekday_plan(set: &TargetSet, wd: usize) -> (Option<&Template>, i32, i32, Option<i32>) {
    let t = set.week[wd].and_then(|id| set.templates.iter().find(|t| t.id == id));
    let offset = t.map_or(0, |t| t.kcal_offset);
    let raw = set.kcal + offset;
    // never above the target itself: a record saved before WR-01 may hold a floor over its Maintain/Gain kcal
    let floor = set.floor_kcal.min(set.kcal);
    if raw < floor {
        (t, floor, offset, Some(raw))
    } else {
        (t, raw, offset, None)
    }
}

fn floor_note(kcal: i32) -> String {
    FLOOR_DAY.replace("{floor}", &kcal_text(kcal))
}

/// The target for one day, read only from the dated record and the weight passed in, never from the journal,
/// the live profile or today's weight (GOAL-06): `weight_kg` is the weight as of `day`.
pub fn day_target(set: &TargetSet, day: &str, weight_kg: Option<f64>) -> Result<DayTarget, String> {
    let wd = model::weekday(diary::day_number(day)?) as usize;
    let (t, kcal, offset, raised_from) = weekday_plan(set, wd);
    let spec = t.map_or(set.macros, |t| t.macros);
    Ok(DayTarget {
        kcal,
        template: t.map(|t| (t.id, t.name.clone())),
        offset,
        raised_from,
        note: raised_from.map(|_| floor_note(kcal)),
        macros: resolve_macros(&spec, kcal, weight_kg.or(set.basis_weight_kg)),
    })
}

pub(crate) const WEEK_LABELS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// Monday first through Sunday, so the page does no index maths (`TargetSet.week` starts on Sunday).
pub fn week_rows(set: &TargetSet) -> Vec<WeekRow> {
    (0..7)
        .map(|i| {
            let (t, kcal, _, raised_from) = weekday_plan(set, (i + 1) % 7);
            WeekRow { label: WEEK_LABELS[i], template: t.map(|t| t.id), template_name: t.map(|t| t.name.clone()), kcal_text: format!("{} kcal", kcal_text(kcal)), note: raised_from.map(|_| floor_note(kcal)) }
        })
        .collect()
}

pub fn check_templates(t: &[Template]) -> Vec<Problem> {
    let mut out = vec![];
    if t.len() > MAX_TEMPLATES {
        out.push(problem("templates", format!("Tiffin keeps up to {MAX_TEMPLATES} templates.")));
    }
    let mut seen = BTreeSet::new();
    for x in t {
        if x.name.trim().is_empty() {
            out.push(problem(&format!("template {} name", x.id), "Give the template a name.".into()));
        }
        if !(-MAX_OFFSET_KCAL..=MAX_OFFSET_KCAL).contains(&x.kcal_offset) {
            out.push(problem(&format!("template {} offset", x.id), "Enter a change from -2,000 to 2,000 kcal.".into()));
        }
        if !seen.insert(x.id) {
            out.push(problem("templates", format!("Two templates share the number {}.", x.id)));
        }
    }
    out
}

/// Trim every name and cut it to the longest kept; run before a record is saved.
pub fn clean_templates(t: &mut [Template]) {
    for x in t {
        x.name = x.name.trim().chars().take(MAX_TEMPLATE_NAME).collect::<String>().trim_end().to_string();
    }
}

/// The record in force on `day`: the one with the greatest `effective_from` on or before it. Dates compare as
/// text (`YYYY-MM-DD`). A record whose date fails `Day::parse` (a hand-edited or restored file) is skipped, never fatal.
pub fn target_for_day<'a>(targets: &'a [TargetSet], day: &str) -> Option<&'a TargetSet> {
    targets.iter().filter(|t| Day::parse(&t.effective_from).is_ok() && t.effective_from.as_str() <= day).max_by(|a, b| a.effective_from.cmp(&b.effective_from))
}

/// Save a record: one with the same `effective_from` is replaced, otherwise it is inserted keeping the list ascending.
pub fn put_target(p: &mut Profile, t: TargetSet) {
    match p.targets.iter().position(|x| x.effective_from >= t.effective_from) {
        Some(i) if p.targets[i].effective_from == t.effective_from => p.targets[i] = t,
        Some(i) => p.targets.insert(i, t),
        None => p.targets.push(t),
    }
}

/// A test fixture: save a record into a journal's profile.
#[cfg(test)]
pub(crate) fn put_target_in(j: &mut crate::tracker::journal::Journal, t: TargetSet) {
    put_target(j.profile_mut().unwrap(), t);
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "Nov 17", built from the parts of a day key (never a parsed date string).
pub(crate) fn date_text(day: &str) -> String {
    let m: Option<usize> = day.get(5..7).and_then(|s| s.parse().ok());
    let d: Option<u32> = day.get(8..).and_then(|s| s.parse().ok());
    match (m.filter(|m| (1..=12).contains(m)), d) {
        (Some(m), Some(d)) => format!("{} {d}", MONTHS[m - 1]),
        _ => day.to_string(),
    }
}

/// A change in weight, "2.1 kg" or "22 lb": one decimal, no trailing zero.
pub fn mass_change_text(kg: f64, unit: MassUnit) -> String {
    format!("{} {}", trimmed(in_unit(kg.abs(), unit), 1), unit_word(unit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracker::targets::*;
    use crate::tracker::journal::Journal;
    use std::fs;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-goals-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn set(day: &str, kcal: i32) -> TargetSet {
        TargetSet { effective_from: day.into(), kcal, ..Default::default() }
    }

    #[test]
    fn a_profile_and_weigh_in_give_a_dated_target() {
        assert_eq!(age_on("2000-05-01", "2026-11-17"), Ok(26));
        assert_eq!(age_on("2000-11-17", "2026-11-17"), Ok(26), "the birthday itself counts");
        assert_eq!(age_on("2000-11-18", "2026-11-17"), Ok(25));
        assert!(age_on("2027-01-01", "2026-11-17").is_err());
        let d = tdir("dated");
        let mut j = Journal::open(d.clone());
        let today = "2026-11-17";
        {
            let p = j.profile_mut().unwrap();
            p.sex = Some(Sex::Male);
            p.birth_date = Some("2000-05-01".into());
            p.height_cm = Some(180.0);
            p.activity = Some(Activity::Light);
        }
        body::set_value(&mut j, today, today, "weight", 80.0, 1).unwrap();
        let w = body::weight_basis(&mut j, today).map(|b| b.kg);
        let b = compute(j.profile().unwrap(), w, today).unwrap();
        assert_eq!((b.bmr, b.kcal), (1800.0, 2475));
        put_target(j.profile_mut().unwrap(), set(today, b.kcal));
        j.save_dirty().unwrap();

        let mut fresh = Journal::open(d.clone());
        let t = &fresh.profile().unwrap().targets;
        assert_eq!(target_for_day(t, "2026-11-17").map(|t| t.kcal), Some(2475));
        assert_eq!(target_for_day(t, "2026-11-30").map(|t| t.kcal), Some(2475));
        assert_eq!(target_for_day(t, "2026-11-16"), None, "the day before has none");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn later_records_leave_earlier_days_alone() {
        let mut p = Profile::default();
        put_target(&mut p, set("2026-11-17", 2475));
        put_target(&mut p, set("2026-11-20", 1925));
        let kcal = |p: &Profile, day: &str| target_for_day(&p.targets, day).map(|t| t.kcal);
        assert_eq!((kcal(&p, "2026-11-18"), kcal(&p, "2026-11-20"), kcal(&p, "2026-12-01")), (Some(2475), Some(1925), Some(1925)));
        put_target(&mut p, set("2026-11-20", 1800));
        assert_eq!((p.targets.len(), kcal(&p, "2026-11-21")), (2, Some(1800)), "the same day replaces its record");
        put_target(&mut p, set("2026-11-10", 2600)); // inserted in order
        assert_eq!(p.targets.iter().map(|t| t.effective_from.as_str()).collect::<Vec<_>>(), ["2026-11-10", "2026-11-17", "2026-11-20"]);
        p.targets.push(set("2026-02-30", 999));
        assert_eq!(kcal(&p, "2026-12-01"), Some(1800), "a record with a bad date is skipped");
    }

    const TODAY: &str = "2026-11-17";

    fn prof(sex: Sex, birth: &str, cm: f64, act: Activity, goal: GoalKind, rate: f64) -> Profile {
        Profile { sex: Some(sex), birth_date: Some(birth.into()), height_cm: Some(cm), activity: Some(act), goal, rate_kg_week: rate, ..Default::default() }
    }

    /// A 25-year-old man, 180 cm, light movement.
    fn male(goal: GoalKind, rate: f64) -> Profile {
        prof(Sex::Male, "2001-05-01", 180.0, Activity::Light, goal, rate)
    }

    /// A 60-year-old woman, 155 cm, mostly sitting.
    fn woman(goal: GoalKind, rate: f64) -> Profile {
        prof(Sex::Female, "1966-01-01", 155.0, Activity::Sitting, goal, rate)
    }

    fn run(p: &Profile, kg: f64) -> Breakdown {
        compute(p, Some(kg), TODAY).unwrap()
    }

    fn kinds(b: &Breakdown) -> Vec<AdjustKind> {
        b.adjustments.iter().map(|a| a.kind).collect()
    }

    fn labels_values(b: &Breakdown) -> (Vec<String>, Vec<String>) {
        (b.lines.iter().map(|l| l.label.clone()).collect(), b.lines.iter().map(|l| l.value.clone()).collect())
    }

    #[test]
    fn bmr_and_target() {
        assert_eq!(bmr(80.0, 180.0, 25, Sex::Male), 1805.0);
        assert_eq!(bmr(55.0, 155.0, 60, Sex::Female), 1057.75);
        assert_eq!(bmr(80.0, 180.0, 25, Sex::Other), 1805.0 - 83.0, "the midpoint offset is -78");
        let b = run(&male(GoalKind::Maintain, 0.5), 80.0);
        let (l, v) = labels_values(&b);
        assert_eq!(l, ["Resting energy (Mifflin-St Jeor)", "Daily activity (x 1.375)", "Maintenance", "Goal: keep your weight", "Your target"]);
        assert_eq!(v, ["1,805", "677", "2,482", "+0", "2,482"]);
        assert_eq!((b.kcal, b.maintenance, b.floor_kcal, b.adjustments.len()), (2482, 2482, 1500, 0));
        let w = run(&woman(GoalKind::Maintain, 0.5), 55.0);
        assert_eq!(labels_values(&w).1[0], "1,058", "1,057.75 rounded once");
        let o = run(&prof(Sex::Other, "2001-05-01", 180.0, Activity::Light, GoalKind::Maintain, 0.5), 80.0);
        assert!(o.lines[0].label.contains("midpoint") && !b.lines[0].label.contains("midpoint"));
        assert_eq!((o.floor_kcal, o.bmr), (1500, 1722.0));
        assert_eq!((other_note(Sex::Other), other_note(Sex::Male), other_note(Sex::Female)), (Some(OTHER_NOTE), None, None));
        assert_eq!((Activity::Sitting.name(), Activity::Physical.description()), ("Mostly sitting", "Hard physical work most of the day"));
        let lose = run(&male(GoalKind::Lose, 0.5), 80.0);
        assert_eq!((lose.lines[3].label.as_str(), lose.lines[3].value.as_str(), lose.kcal, lose.adjustments.len()), ("Goal: lose 0.5 kg a week", "\u{2212}550", 1932, 0), "BMI 24.7 sets the cap to 0.5, so 0.5 is used unchanged");
        let mut lb = male(GoalKind::Lose, 0.5);
        lb.units.rate = MassUnit::Lb;
        assert_eq!(run(&lb, 80.0).lines[3].label, "Goal: lose 1.1 lb a week");
    }

    #[test]
    fn clamps() {
        // BMI under 25: lose 0.75 is capped to 0.5
        let b = run(&male(GoalKind::Lose, 0.75), 80.0);
        assert_eq!(kinds(&b), [AdjustKind::RateCap]);
        let a = &b.adjustments[0];
        assert_eq!((a.original.as_str(), a.adjusted.as_str()), ("0.75 kg a week", "0.5 kg a week"));
        assert!(a.reason.contains("(the pace for a BMI under 25)") && !a.reason.contains("1%"), "{}", a.reason);
        assert_eq!((b.rate_kg_week, b.kcal), (0.5, 1932));
        // BMI 34: the 1 kg ceiling sets the cap, no parenthetical
        let b = run(&male(GoalKind::Lose, 1.5), 110.0);
        assert_eq!(kinds(&b), [AdjustKind::RateCap]);
        assert!(!b.adjustments[0].reason.contains("(about") && !b.adjustments[0].reason.contains("(the pace"), "{}", b.adjustments[0].reason);
        assert_eq!(b.adjustments[0].adjusted, "1 kg a week");
        assert!(run(&male(GoalKind::Lose, 1.0), 110.0).adjustments.is_empty());
        assert_eq!(kinds(&run(&male(GoalKind::Lose, 1.01), 110.0)), [AdjustKind::RateCap]);
        // BMI 31 at 90 kg: 1% of the weight is 0.9
        let b = run(&prof(Sex::Male, "2001-05-01", 170.0, Activity::Light, GoalKind::Lose, 1.0), 90.0);
        assert_eq!((b.adjustments[0].adjusted.as_str(), b.rate_kg_week), ("0.9 kg a week", 0.9));
        assert!(b.adjustments[0].reason.contains("(about 1% of your weight)"), "{}", b.adjustments[0].reason);
        // gain
        let b = run(&male(GoalKind::Gain, 0.75), 80.0);
        assert_eq!(kinds(&b), [AdjustKind::RateCap]);
        assert_eq!(b.adjustments[0].reason, "You asked to gain 0.75 kg a week. Tiffin limits gain to 0.5 kg a week. Your target uses 0.5 kg a week.");
        assert_eq!((b.lines[3].value.as_str(), b.kcal), ("+550", 3032));
        // BMI guard turns lose into maintain
        let b = run(&male(GoalKind::Lose, 0.5), 50.0);
        assert_eq!(kinds(&b), [AdjustKind::BmiGuard]);
        assert_eq!(b.adjustments[0].reason, BMI_GUARD);
        assert_eq!((b.lines[3].label.as_str(), b.kcal, b.kcal == b.maintenance), ("Goal: keep your weight", 2069, true));
        // clamp order: the rate cap runs before the floor
        let b = run(&woman(GoalKind::Lose, 0.75), 55.0);
        assert_eq!(kinds(&b), [AdjustKind::RateCap, AdjustKind::Floor]);
        // under 18: no target
        let young = prof(Sex::Male, "2009-11-17", 175.0, Activity::Light, GoalKind::Maintain, 0.5);
        let e = compute(&young, Some(60.0), TODAY).unwrap_err();
        assert_eq!(e, [Problem { field: "birth date".into(), text: UNDER_18.into() }]);
    }

    #[test]
    fn floor_never_raises_loss_above_maintenance() {
        // Female 60, 55 kg, 155 cm: maintenance 1,269, request 719, floor 1,200
        let b = run(&woman(GoalKind::Lose, 0.75), 55.0);
        assert_eq!((b.bmr.round() as i32, b.maintenance, b.kcal, b.floor_kcal), (1058, 1269, 1200, 1200));
        let f = b.adjustments.iter().find(|a| a.kind == AdjustKind::Floor).unwrap();
        assert_eq!((f.original.as_str(), f.adjusted.as_str()), ("719 kcal", "1,200 kcal"));
        assert_eq!(f.reason, FLOOR.replace("{raw}", "719").replace("{floor}", "1,200"));
        // Female 70, 45 kg, 150 cm: maintenance 1,052 is under the 1,200 floor, so the target is maintenance
        let small = prof(Sex::Female, "1956-01-01", 150.0, Activity::Sitting, GoalKind::Lose, 0.5);
        let b = run(&small, 45.0);
        assert_eq!((b.maintenance, b.kcal, b.floor_kcal), (1052, 1052, 1052));
        let f = b.adjustments.iter().find(|a| a.kind == AdjustKind::Floor).unwrap();
        assert_eq!(f.adjusted, "1,052 kcal", "the floor adjustment names maintenance as the result");
        assert!(f.reason.contains("Your maintenance is 1,052 kcal"), "{}", f.reason);
        // maintain is never floored
        let keep = run(&Profile { goal: GoalKind::Maintain, ..small.clone() }, 45.0);
        assert_eq!((keep.kcal, keep.adjustments.len(), keep.floor_kcal), (1052, 0, 1052));
        // and the Diary day uses the same number, not raised to 1,200 (WR-01)
        let rec = TargetSet { effective_from: TODAY.into(), kcal: keep.kcal, floor_kcal: keep.floor_kcal, ..Default::default() };
        let dt = day_target(&rec, TODAY, Some(45.0)).unwrap();
        assert_eq!((dt.kcal, dt.raised_from), (1052, None));
        // a record saved before the fix (floor 1,200 over kcal 1,052) is read the same way
        let old = TargetSet { floor_kcal: 1200, ..rec.clone() };
        assert_eq!(day_target(&old, TODAY, Some(45.0)).unwrap().kcal, 1052);
        // a template day below the floor is still lifted, to the target
        let rest = Template { id: 1, name: "Rest".into(), kcal_offset: -200, macros: MacroSpec::default() };
        let wd = model::weekday(diary::day_number(TODAY).unwrap()) as usize;
        let mut week = [None; 7];
        week[wd] = Some(1);
        let with_rest = TargetSet { templates: vec![rest], week, ..old };
        let dt = day_target(&with_rest, TODAY, Some(45.0)).unwrap();
        assert_eq!((dt.kcal, dt.raised_from), (1052, Some(852)));
        let gain = run(&Profile { goal: GoalKind::Gain, rate_kg_week: 0.1, ..small }, 45.0);
        assert!(gain.kcal > gain.floor_kcal && gain.floor_kcal == 1052, "{gain:?}");
    }

    #[test]
    fn an_empty_rate_box_is_a_problem() {
        let d = tdir("rate-empty");
        let mut j = Journal::open(d.clone());
        let mut dr = draft();
        dr.goal = GoalKind::Lose;
        dr.rate.value = None;
        let r = preview(&mut j, &dr, TODAY);
        assert!(!r.can_save && r.problems.iter().any(|p| p.field == "rate" && p.text == "Enter how much you want to change each week."), "{:?}", r.problems);
        assert!(save(&mut j, &dr, TODAY, 1, false).is_err());
        assert_eq!(r.inputs.rate.value, None, "a unit change keeps the box empty");
        dr.goal = GoalKind::Gain;
        assert!(!preview(&mut j, &dr, TODAY).can_save);
        let mut own = dr.clone();
        own.source = TargetSource::Manual;
        own.manual_kcal = Some(2000.0);
        assert!(preview(&mut j, &own, TODAY).can_save, "a number the person chose never needs a rate");
        dr.goal = GoalKind::Maintain;
        assert!(preview(&mut j, &dr, TODAY).can_save, "Maintain has no rate to enter");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn manual_below_floor() {
        let (k, adj) = manual_target(1000.0, Sex::Female, 1058.0, Some(30)).unwrap();
        assert_eq!(k, 1200);
        assert_eq!(adj.len(), 1);
        assert_eq!((adj[0].kind, adj[0].reason.as_str()), (AdjustKind::Manual, "Your own number, 1,000 kcal, is below 1,200 kcal, so Tiffin uses 1,200 kcal."));
        assert_eq!((adj[0].original.as_str(), adj[0].adjusted.as_str()), ("1,000 kcal", "1,200 kcal"));
        assert_eq!(manual_target(2000.0, Sex::Female, 1058.0, Some(30)).unwrap(), (2000, vec![]));
        // exactly at the floor is unchanged, 1 below is raised; no maintenance guard on a manual number
        assert_eq!(manual_target(1200.0, Sex::Female, 1058.0, None).unwrap().1.len(), 0);
        assert_eq!(manual_target(1199.0, Sex::Female, 1058.0, None).unwrap().0, 1200);
        // 0.8 x resting energy can be the higher floor
        assert_eq!(manual_target(1500.0, Sex::Male, 2000.0, None).unwrap().0, 1600);
        for bad in [f64::NAN, f64::INFINITY, 0.0, 0.5, 10_001.0, -5.0] {
            assert!(manual_target(bad, Sex::Male, 1800.0, None).is_err(), "{bad}");
        }
    }

    #[test]
    fn manual_under_18_keeps_the_floor() {
        let (k, adj) = manual_target(1000.0, Sex::Female, 1058.0, Some(16)).unwrap();
        assert_eq!(k, 1200);
        assert_eq!(adj.iter().map(|a| a.kind).collect::<Vec<_>>(), [AdjustKind::Age, AdjustKind::Manual]);
        assert_eq!((adj[0].reason.as_str(), adj[0].original.as_str(), adj[0].adjusted.as_str()), (UNDER_18, "1,000 kcal", "1,200 kcal"));
        let (k, adj) = manual_target(2000.0, Sex::Female, 1058.0, Some(17)).unwrap();
        assert_eq!((k, adj.iter().map(|a| a.kind).collect::<Vec<_>>()), (2000, vec![AdjustKind::Age]));
        for age in [Some(18), None] {
            assert!(manual_target(2000.0, Sex::Female, 1058.0, age).unwrap().1.is_empty());
        }
    }

    #[test]
    fn boundaries() {
        // age: 18 gets a target on the birthday, 17 the day before
        let adult = |birth: &str| compute(&prof(Sex::Male, birth, 180.0, Activity::Light, GoalKind::Maintain, 0.5), Some(80.0), TODAY);
        assert!(adult("2008-11-17").is_ok());
        assert!(adult("2008-11-18").is_err());
        // BMI exactly 18.5 keeps lose, just below does not
        let thin = |kg: f64| run(&prof(Sex::Male, "2001-05-01", 200.0, Activity::Light, GoalKind::Lose, 0.5), kg);
        assert!(!kinds(&thin(74.0)).contains(&AdjustKind::BmiGuard));
        assert!(kinds(&thin(73.9)).contains(&AdjustKind::BmiGuard));
        // a floor result exactly at the floor is unchanged, 1 kcal below is raised (maintenance 1,269, floor 1,200)
        let b = run(&woman(GoalKind::Lose, 0.0627), 55.0);
        assert_eq!((b.kcal, b.adjustments.len()), (1200, 0), "the request is exactly 1,200");
        let b = run(&woman(GoalKind::Lose, 0.0636), 55.0);
        assert_eq!((b.kcal, kinds(&b)), (1200, vec![AdjustKind::Floor]));
        assert_eq!(b.adjustments[0].original, "1,199 kcal");
        // bad inputs come back as problems tied to their field
        let bad = |f: &dyn Fn(&mut Profile)| {
            let mut p = male(GoalKind::Lose, 0.5);
            f(&mut p);
            compute(&p, Some(80.0), TODAY).unwrap_err()
        };
        assert_eq!(bad(&|p| p.height_cm = Some(f64::NAN))[0].field, "height");
        assert_eq!(bad(&|p| p.height_cm = Some(49.0))[0].field, "height");
        assert_eq!(bad(&|p| p.rate_kg_week = 2.5)[0].field, "rate");
        assert_eq!(bad(&|p| p.rate_kg_week = f64::NAN)[0].field, "rate");
        assert_eq!(bad(&|p| p.birth_date = Some("1899-12-31".into()))[0].field, "birth date");
        assert_eq!(bad(&|p| p.birth_date = Some("2026-11-18".into()))[0].field, "birth date");
        assert_eq!(compute(&male(GoalKind::Maintain, 0.5), Some(500.1), TODAY).unwrap_err()[0].field, "weight");
        assert_eq!(compute(&male(GoalKind::Maintain, 0.5), Some(f64::NAN), TODAY).unwrap_err()[0].field, "weight");
    }

    #[test]
    fn units_round_trip() {
        assert!((kg_from(180.0, MassUnit::Lb) - 81.6466266).abs() < 1e-6);
        assert_eq!(mass_text(kg_from(180.0, MassUnit::Lb), MassUnit::Lb), "180.0 lb");
        assert_eq!(mass_text(80.0, MassUnit::Kg), "80.0 kg");
        for i in 500..=4500 {
            let lb = i as f64 / 10.0;
            assert_eq!(mass_text(kg_from(lb, MassUnit::Lb), MassUnit::Lb), format!("{lb:.1} lb"), "{lb}");
            assert!((in_unit(kg_from(lb, MassUnit::Lb), MassUnit::Lb) - lb).abs() < 1e-9);
        }
        assert!((cm_from_ft_in(5.0, 11.0) - 180.34).abs() < 1e-9);
        assert_eq!(ft_in_text(180.34), "5 ft 11 in");
        assert_eq!(ft_in_text(cm_from_ft_in(5.0, 11.5)), "5 ft 11.5 in");
        assert_eq!(ft_in_parts(cm_from_ft_in(6.0, 0.0)), (6, 0.0));
        assert_eq!(ft_in_text(cm_from_ft_in(5.0, 11.96)), "6 ft 0 in", "inches carry into feet");
        assert_eq!((height_text(180.34, HeightUnit::Cm).as_str(), height_text(180.0, HeightUnit::Cm).as_str()), ("180.3 cm", "180 cm"));
        assert_eq!(height_text(180.34, HeightUnit::FtIn), "5 ft 11 in");
        assert_eq!((rate_text(0.75, MassUnit::Kg).as_str(), rate_text(0.5, MassUnit::Kg).as_str(), rate_text(1.0, MassUnit::Kg).as_str()), ("0.75 kg", "0.5 kg", "1 kg"));
        assert_eq!((kcal_text(2475), kcal_text(999), kcal_text(-1100), kcal_text(0), kcal_text(1_234_567)), ("2,475".to_string(), "999".to_string(), "-1,100".to_string(), "0".to_string(), "1,234,567".to_string()));
        assert_eq!((signed_kcal_text(300), signed_kcal_text(-1300), signed_kcal_text(0)), ("+300".to_string(), "\u{2212}1,300".to_string(), "+0".to_string()), "a true minus, as the Diary prints exercise");
        for ok in [50.0, 180.34, 272.0] {
            assert_eq!(check_height_cm(ok), Ok(ok));
        }
        for bad in [f64::NAN, f64::INFINITY, 49.9, 272.1, -170.0] {
            assert!(check_height_cm(bad).unwrap_err().contains("50 to 272 cm"), "{bad}");
        }
    }

    #[test]
    fn lines_add_up() {
        let n = |s: &str| s.replace([',', '+'], "").replace('\u{2212}', "-").parse::<i32>().unwrap();
        let fixtures: Vec<(Profile, f64)> = vec![
            (male(GoalKind::Maintain, 0.5), 80.0),
            (male(GoalKind::Lose, 0.5), 80.0),
            (male(GoalKind::Lose, 0.75), 80.0),
            (male(GoalKind::Lose, 1.5), 110.0),
            (male(GoalKind::Gain, 0.75), 80.0),
            (male(GoalKind::Lose, 0.5), 50.0),
            (woman(GoalKind::Lose, 0.75), 55.0),
            (woman(GoalKind::Lose, 0.0636), 55.0),
            (prof(Sex::Female, "1956-01-01", 150.0, Activity::Sitting, GoalKind::Lose, 0.5), 45.0),
            (prof(Sex::Other, "2001-05-01", 180.0, Activity::OnFeet, GoalKind::Lose, 0.5), 80.0),
        ];
        for (p, kg) in fixtures {
            let b = run(&p, kg);
            let v: Vec<i32> = b.lines.iter().map(|l| n(&l.value)).collect();
            assert_eq!(v[0] + v[1], v[2], "resting + activity = maintenance: {:?}", b.lines);
            assert_eq!(v[2] + v[3], v[4], "maintenance + goal = target: {:?}", b.lines);
            assert_eq!((v[2], v[4]), (b.maintenance, b.kcal));
        }
    }

    fn plan(protein: MacroMode, carbs: MacroMode, fat: MacroMode) -> MacroSpec {
        MacroSpec { protein, carbs, fat }
    }

    fn readouts(r: &MacroResult) -> Vec<&str> {
        r.rows.iter().map(|x| x.readout.as_str()).collect()
    }

    #[test]
    fn macros() {
        let r = resolve_macros(&MacroSpec::default(), 2000, None);
        assert_eq!(r.rows.iter().map(|x| x.name).collect::<Vec<_>>(), ["Protein", "Carbs", "Fat"]);
        assert_eq!(readouts(&r), ["100 g · 20% · 400 kcal", "250 g · 50% · 1,000 kcal", "67 g · 30% · 600 kcal"]);
        assert_eq!((r.total_text.as_str(), r.rows[0].said.as_str(), r.problems.len()), ("Total 2,000 kcal · 100%", "100 g protein", 0));
        assert_eq!(TargetSet::default().macros, PRESETS[0].1, "a new plan starts from Balanced");
        // percentages: 99.5 and 100.5 pass, 99.4 and 100.6 and 95 fail with the sum named
        let split = |f: f64| resolve_macros(&plan(MacroMode::Percent(30.0), MacroMode::Percent(40.0), MacroMode::Percent(f)), 2000, None);
        assert!(split(29.5).problems.is_empty() && split(30.5).problems.is_empty());
        for (f, n) in [(25.0, "95"), (29.4, "99.4"), (30.6, "100.6")] {
            let r = split(f);
            let text = format!("Percentages add up to {n}%. They need to add up to 100%.");
            assert_eq!(r.problems.iter().map(|p| (p.field.as_str(), p.text.as_str())).collect::<Vec<_>>(), [("carbs", text.as_str()), ("fat", text.as_str())], "{f}");
            assert_eq!((r.rows[1].problem.as_deref(), r.rows[2].problem.as_deref(), r.rows[0].problem.as_deref()), (Some(text.as_str()), Some(text.as_str()), None));
        }
        // a missing or non-finite number is a problem naming the macro, never a silent 0
        let r = resolve_macros(&plan(MacroMode::Grams(f64::NAN), MacroMode::Percent(50.0), MacroMode::Percent(30.0)), 2000, None);
        assert_eq!((r.problems[0].field.as_str(), r.problems[0].text.as_str()), ("protein", "Enter a number of 0 or more for protein."));
        let r = resolve_macros(&plan(MacroMode::Percent(20.0), MacroMode::Percent(f64::INFINITY), MacroMode::Percent(30.0)), 2000, None);
        assert_eq!(r.problems[0].field, "carbs");
        let r = resolve_macros(&plan(MacroMode::Percent(20.0), MacroMode::Percent(50.0), MacroMode::Percent(-30.0)), 2000, None);
        assert_eq!(r.problems[0].field, "fat");
    }

    #[test]
    fn presets_fill_and_resolve() {
        assert_eq!(PRESETS.iter().map(|p| p.0).collect::<Vec<_>>(), ["Balanced", "High Protein", "Low Carb", "Keto"]);
        for (name, spec) in PRESETS {
            for kcal in [1200, 4000] {
                let r = resolve_macros(&spec, kcal, None);
                assert!(r.problems.is_empty(), "{name} at {kcal}: {:?}", r.problems);
                assert!(r.rows.iter().all(|x| x.grams >= 0.0));
            }
        }
        let keto = resolve_macros(&PRESETS[3].1, 2000, None);
        assert_eq!((keto.rows[1].pct.round(), keto.rows[1].grams.round()), (5.0, 25.0));
    }

    #[test]
    fn macro_modes_follow_the_right_thing() {
        // protein per kg, carbs the rest, fat a share
        let spec = plan(MacroMode::PerKg(2.0), MacroMode::Rest, MacroMode::Percent(30.0));
        let r = resolve_macros(&spec, 2000, Some(80.0));
        assert_eq!(readouts(&r), ["160 g · 32% · 640 kcal", "190 g · 38% · 760 kcal", "67 g · 30% · 600 kcal"]);
        assert!(r.problems.is_empty());
        let r = resolve_macros(&spec, 2000, None);
        assert_eq!((r.problems[0].field.as_str(), r.problems[0].text.as_str()), ("protein", BAD_WEIGHT_PER_KG));
        // what is left can't be negative, and exactly nothing left is fine
        let over = plan(MacroMode::Grams(250.0), MacroMode::Rest, MacroMode::Grams(120.0));
        let r = resolve_macros(&over, 1500, None);
        assert_eq!(r.problems.len(), 1);
        assert_eq!(r.problems[0].field, "carbs");
        assert!(r.problems[0].text.contains("already use more than 1,500 kcal"), "{}", r.problems[0].text);
        let none_left = plan(MacroMode::Grams(100.0), MacroMode::Rest, MacroMode::Grams(100.0));
        let r = resolve_macros(&none_left, 1300, None);
        assert_eq!((r.rows[1].grams, r.problems.len()), (0.0, 0));
        // fat floor
        let r = resolve_macros(&plan(MacroMode::Percent(30.0), MacroMode::Percent(55.0), MacroMode::Percent(15.0)), 2000, None);
        assert_eq!((r.problems[0].field.as_str(), r.problems[0].text.as_str()), ("fat", FAT_UNDER_20));
        // modes a macro does not allow
        let r = resolve_macros(&plan(MacroMode::Percent(20.0), MacroMode::Rest, MacroMode::Rest), 2000, None);
        assert_eq!(r.problems.iter().map(|p| p.field.as_str()).collect::<Vec<_>>(), ["carbs", "fat"]);
        let r = resolve_macros(&plan(MacroMode::Rest, MacroMode::Percent(50.0), MacroMode::Percent(30.0)), 2000, None);
        assert_eq!(r.problems[0].field, "protein");
        let r = resolve_macros(&plan(MacroMode::Percent(20.0), MacroMode::PerKg(2.0), MacroMode::Percent(30.0)), 2000, Some(80.0));
        assert_eq!(r.problems[0].field, "carbs");
        // grams stay put as calories change; percent follows (D-09)
        let fixed = plan(MacroMode::Grams(150.0), MacroMode::Rest, MacroMode::Percent(30.0));
        let (low, high) = (resolve_macros(&fixed, 1800, None), resolve_macros(&fixed, 2400, None));
        assert_eq!((low.rows[0].grams, high.rows[0].grams), (150.0, 150.0));
        assert_eq!((low.rows[0].pct.round(), high.rows[0].pct.round()), (33.0, 25.0));
        let (b_low, b_high) = (resolve_macros(&MacroSpec::default(), 1800, None), resolve_macros(&MacroSpec::default(), 2400, None));
        assert_eq!((b_low.rows[0].grams, b_high.rows[0].grams), (90.0, 120.0), "percent-mode grams rescale with the calories");
    }

    fn week_set() -> TargetSet {
        let training = Template { id: 1, name: "Training".into(), kcal_offset: 300, macros: PRESETS[1].1 };
        let rest = Template { id: 2, name: "Rest".into(), kcal_offset: -700, macros: PRESETS[0].1 };
        let mut week = [None; 7];
        week[1] = Some(1); // Monday
        week[0] = Some(2); // Sunday
        TargetSet { effective_from: "2026-11-01".into(), kcal: 2100, floor_kcal: 1500, templates: vec![training, rest], week, ..Default::default() }
    }

    #[test]
    fn weekday_templates() {
        let set = week_set();
        let mon = day_target(&set, "2026-11-16", None).unwrap();
        assert_eq!((mon.kcal, mon.template.clone(), mon.offset, mon.raised_from), (2400, Some((1, "Training".into())), 300, None));
        assert_eq!(mon.macros.rows[0].pct.round(), 30.0, "the template's own macros");
        let sun = day_target(&set, "2026-11-15", None).unwrap();
        assert_eq!((sun.kcal, sun.raised_from, sun.note.as_deref()), (1500, Some(1400), Some("Raised to 1,500, the lowest Tiffin sets.")));
        let tue = day_target(&set, "2026-11-17", None).unwrap();
        assert_eq!((tue.kcal, tue.template, tue.offset, tue.note), (2100, None, 0, None));
        // a template id that no longer exists falls back to the default plan
        let mut gone = set.clone();
        gone.week[2] = Some(99);
        assert_eq!(day_target(&gone, "2026-11-17", None).unwrap().template, None);
        // exactly at the floor is unchanged, 1 kcal below is raised
        let mut edge = set.clone();
        edge.templates[1].kcal_offset = -600;
        assert_eq!(day_target(&edge, "2026-11-15", None).unwrap().raised_from, None);
        edge.templates[1].kcal_offset = -601;
        assert_eq!(day_target(&edge, "2026-11-15", None).unwrap().raised_from, Some(1499));
        assert!(day_target(&set, "2026-02-30", None).is_err());
        // week rows: Monday first, two weekdays can share a template, a missing one is the default plan
        let mut shared = set.clone();
        shared.week[3] = Some(1); // Wednesday
        shared.week[5] = Some(99); // Friday: no such template
        let rows = week_rows(&shared);
        assert_eq!(rows.iter().map(|r| r.label).collect::<Vec<_>>(), ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]);
        assert_eq!((rows[0].template, rows[0].kcal_text.as_str()), (Some(1), "2,400 kcal"));
        assert_eq!((rows[2].template, rows[2].template_name.as_deref()), (Some(1), Some("Training")));
        assert_eq!((rows[4].template, rows[4].kcal_text.as_str()), (None, "2,100 kcal"));
        assert_eq!((rows[6].kcal_text.as_str(), rows[6].note.as_deref()), ("1,500 kcal", Some("Raised to 1,500, the lowest Tiffin sets.")));
        assert_eq!(rows[1].template, None);
        // with no templates every weekday is the default plan
        let plain = TargetSet { kcal: 2000, floor_kcal: 1500, ..Default::default() };
        assert!(week_rows(&plain).iter().all(|r| r.template.is_none() && r.kcal_text == "2,000 kcal"));
    }

    #[test]
    fn per_kg_follows_the_days_weight() {
        let set = TargetSet {
            effective_from: "2026-11-01".into(),
            kcal: 2000,
            floor_kcal: 1500,
            basis_weight_kg: Some(80.0),
            macros: plan(MacroMode::PerKg(2.0), MacroMode::Rest, MacroMode::Percent(30.0)),
            ..Default::default()
        };
        let grams = |w: Option<f64>| {
            let m = day_target(&set, "2026-11-17", w).unwrap().macros;
            (m.rows[0].grams.round(), m.rows[1].grams.round())
        };
        assert_eq!((grams(Some(81.0)), grams(Some(80.0)), grams(None)), ((162.0, 188.0), (160.0, 190.0), (160.0, 190.0)), "the record's basis is the fallback");
        let fixed = TargetSet { macros: plan(MacroMode::Grams(150.0), MacroMode::Rest, MacroMode::Percent(30.0)), ..set.clone() };
        assert_eq!(day_target(&fixed, "2026-11-17", Some(90.0)).unwrap().macros.rows[0].grams, 150.0);
        let bare = TargetSet { basis_weight_kg: None, ..set.clone() };
        assert_eq!(day_target(&bare, "2026-11-17", None).unwrap().macros.problems[0].text, BAD_WEIGHT_PER_KG);
        // through the body log: a later weigh-in moves later days and never an earlier one (GOAL-06)
        let d = tdir("perkg");
        let mut j = Journal::open(d.clone());
        body::set_value(&mut j, "2026-11-10", "2026-11-25", "weight", 80.0, 1).unwrap();
        body::set_value(&mut j, "2026-11-20", "2026-11-25", "weight", 81.0, 2).unwrap();
        let mut protein = |day: &str| {
            let w = body::weight_basis(&mut j, day).map(|b| b.kg);
            day_target(&set, day, w).unwrap().macros.rows[0].grams.round()
        };
        assert_eq!((protein("2026-11-15"), protein("2026-11-19"), protein("2026-11-20"), protein("2026-11-25")), (160.0, 160.0, 162.0, 162.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn template_checks() {
        let t = |id: u32, name: &str, off: i32| Template { id, name: name.into(), kcal_offset: off, macros: MacroSpec::default() };
        assert!(check_templates(&[t(1, "Training", 300), t(2, "Rest", -2000), t(3, "Big", 2000)]).is_empty());
        let e = check_templates(&[t(1, "   ", 0)]);
        assert_eq!(e.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["Give the template a name."]);
        assert_eq!(check_templates(&[t(1, "A", 2001)]).len(), 1);
        assert_eq!(check_templates(&[t(1, "A", -2001)]).len(), 1);
        assert_eq!(check_templates(&[t(1, "A", 0), t(1, "B", 0)]).len(), 1, "ids are unique");
        let many: Vec<Template> = (0..21).map(|i| t(i, "T", 0)).collect();
        assert_eq!(check_templates(&many).len(), 1);
        assert!(check_templates(&many[..20]).is_empty());
        let mut long = vec![t(1, &format!("  {}  ", "x".repeat(60)), 0)];
        clean_templates(&mut long);
        assert_eq!(long[0].name, "x".repeat(40));
    }

    #[test]
    fn missing_fields_give_no_target_and_one_problem_each() {
        let p = Profile { height_cm: Some(180.0), ..Default::default() };
        let e = compute(&p, None, "2026-11-17").unwrap_err();
        assert_eq!(e.iter().map(|x| x.field.as_str()).collect::<Vec<_>>(), ["sex", "birth date", "activity level", "weight"]);
        assert_eq!(e[0].text, "Enter your sex.");
    }

    // ---- the Targets page: draft, preview, save ----

    /// Every file under `dir` with its bytes, so a test can say nothing was written.
    fn bytes_of(dir: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
            let Ok(rd) = fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else {
                    out.insert(p.strip_prefix(base).unwrap().display().to_string(), fs::read(&p).unwrap());
                }
            }
        }
        let mut out = Default::default();
        walk(dir, dir, &mut out);
        out
    }

    /// Male, born 2000-05-01, 180 cm, 80 kg typed, light movement, keep weight, Balanced, add-back 100.
    fn draft() -> TargetsDraft {
        TargetsDraft {
            sex: Some(Sex::Male),
            birth_date: Some("2000-05-01".into()),
            height: HeightInput { unit: HeightUnit::Cm, cm: Some(180.0), ft: None, inches: None },
            weight: MassInput { value: Some(80.0), unit: MassUnit::Kg },
            weight_edited: true,
            activity: Some(Activity::Light),
            ..Default::default()
        }
    }

    fn weights(j: &mut Journal) -> Vec<(String, f64)> {
        j.month("2026-11").unwrap().body.iter().filter(|e| e.measure == "weight").map(|e| (e.day.clone(), e.value)).collect()
    }

    #[test]
    fn preview_works_out_the_draft_and_writes_nothing() {
        let d = tdir("preview");
        let mut j = Journal::open(d.clone());
        let r = preview(&mut j, &draft(), TODAY);
        assert!(r.ready && r.can_save && r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!((r.kcal, r.kcal_text.as_str(), r.age_text.as_deref()), (Some(2475), "2,475", Some("Age 26")));
        assert_eq!(r.lines.len(), 5);
        assert_eq!(r.macros.rows.iter().map(|m| m.name).collect::<Vec<_>>(), ["Protein", "Carbs", "Fat"]);
        assert_eq!((r.week.len(), r.week[0].label, r.week[6].label), (7, "Monday", "Sunday"));
        assert_eq!(r.today_text.as_deref(), Some("Today, Tuesday: 2,475 kcal."));
        assert_eq!(r.weight_basis_text.as_deref(), Some("This target uses 80.0 kg (the weight you typed)."));
        assert!(j.month("2026-11").unwrap().body.is_empty(), "a preview logs no weigh-in");
        assert!(bytes_of(&d).is_empty(), "nothing on disk after a preview of a fresh journal");

        // and over saved files: every byte stays
        save(&mut j, &draft(), TODAY, 10, false).unwrap();
        j.save_dirty().unwrap();
        let before = bytes_of(&d);
        let mut other = draft();
        other.goal = GoalKind::Lose;
        other.source = TargetSource::Manual;
        other.manual_kcal = Some(1800.0);
        let mut fresh = Journal::open(d.clone());
        let r = preview(&mut fresh, &other, TODAY);
        assert_eq!(r.kcal, Some(1800));
        fresh.save_dirty().unwrap();
        assert_eq!(bytes_of(&d), before);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn save_writes_the_details_one_dated_target_and_todays_weigh_in() {
        let d = tdir("save");
        let mut j = Journal::open(d.clone());
        save(&mut j, &draft(), TODAY, 10, false).unwrap();
        j.save_dirty().unwrap();
        let mut fresh = Journal::open(d.clone());
        let p = fresh.profile().unwrap().clone();
        assert_eq!(p.format, 2);
        assert_eq!((p.sex, p.birth_date.as_deref(), p.height_cm, p.activity), (Some(Sex::Male), Some("2000-05-01"), Some(180.0), Some(Activity::Light)));
        assert_eq!(p.targets.len(), 1);
        let t = &p.targets[0];
        assert_eq!((t.effective_from.as_str(), t.kcal, t.addback_pct, t.basis_weight_kg, t.manual_kcal, t.calculated_kcal), (TODAY, 2475, 100, Some(80.0), None, Some(2475)));
        assert_eq!(weights(&mut fresh), [(TODAY.to_string(), 80.0)]);

        // the same day again replaces that day's record; an earlier record stays
        let mut earlier = Journal::open(tdir("save-earlier"));
        put_target(earlier.profile_mut().unwrap(), set("2026-11-01", 2000));
        let mut lose = draft();
        lose.goal = GoalKind::Lose;
        lose.rate = MassInput { value: Some(0.5), unit: MassUnit::Kg };
        save(&mut earlier, &lose, TODAY, 11, false).unwrap();
        save(&mut earlier, &lose, TODAY, 12, false).unwrap();
        let t = &earlier.profile().unwrap().targets;
        assert_eq!(t.iter().map(|x| (x.effective_from.as_str(), x.kcal)).collect::<Vec<_>>(), [("2026-11-01", 2000), (TODAY, 1925)]);
        assert_eq!(weights(&mut earlier).len(), 1, "the same day's weigh-in is replaced, not added");

        // a missing field: its sentence, nothing written
        let d2 = tdir("save-missing");
        let mut j2 = Journal::open(d2.clone());
        let mut no_sex = draft();
        no_sex.sex = None;
        assert_eq!(save(&mut j2, &no_sex, TODAY, 1, false), Err("Enter your sex.".into()));
        j2.save_dirty().unwrap();
        assert!(bytes_of(&d2).is_empty());
        for x in [&d, &d2] {
            let _ = fs::remove_dir_all(x);
        }
        let _ = fs::remove_dir_all(std::env::temp_dir().join(format!("rb-goals-save-earlier-{}", std::process::id())));
    }

    #[test]
    fn an_unchanged_field_keeps_its_stored_value() {
        let d = tdir("keeps");
        let mut j = Journal::open(d.clone());
        {
            let p = j.profile_mut().unwrap();
            p.height_cm = Some(180.34);
            p.rate_kg_week = 0.5;
            p.units.rate = MassUnit::Lb;
        }
        let mut dr = draft();
        dr.height.cm = Some(180.3);
        dr.rate = MassInput { value: Some(1.1), unit: MassUnit::Lb };
        save(&mut j, &dr, TODAY, 1, false).unwrap();
        let p = j.profile().unwrap();
        assert_eq!((p.height_cm, p.rate_kg_week), (Some(180.34), 0.5), "shown rounded, saved unrounded");
        dr.height.cm = Some(181.0);
        dr.rate.value = Some(1.5);
        save(&mut j, &dr, TODAY, 2, false).unwrap();
        let p = j.profile().unwrap();
        assert_eq!(p.height_cm, Some(181.0));
        assert!((p.rate_kg_week - 1.5 * KG_PER_LB).abs() < 1e-12);
        // a protein figure shown in g per lb and left alone stays its g per kg
        let mut with_per_kg = draft();
        with_per_kg.macros = MacroSpec { protein: MacroMode::PerKg(2.0), carbs: MacroMode::Rest, fat: MacroMode::Percent(30.0) };
        save(&mut j, &with_per_kg, TODAY, 3, false).unwrap();
        let mut again = draft();
        again.weight_edited = false;
        again.macros = MacroSpec { protein: MacroMode::PerKg(0.9), carbs: MacroMode::Rest, fat: MacroMode::Percent(30.0) };
        again.per_kg_unit = MassUnit::Lb;
        save(&mut j, &again, TODAY, 4, false).unwrap();
        assert_eq!(j.profile().unwrap().targets[0].macros.protein, MacroMode::PerKg(2.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_weight_that_was_not_typed_writes_no_weigh_in() {
        let d = tdir("noweigh");
        let mut j = Journal::open(d.clone());
        body::set_value(&mut j, "2026-11-10", TODAY, "weight", 79.0, 1).unwrap();
        let mut dr = draft();
        dr.weight_edited = false;
        dr.weight.value = None;
        let r = preview(&mut j, &dr, TODAY);
        assert_eq!(r.weight_basis_text.as_deref(), Some("This target uses 79.0 kg (latest weigh-in)."));
        save(&mut j, &dr, TODAY, 2, false).unwrap();
        assert_eq!(weights(&mut j), [("2026-11-10".to_string(), 79.0)]);
        assert_eq!(j.profile().unwrap().targets[0].basis_weight_kg, Some(79.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_manual_number_is_raised_to_the_floor_and_needs_no_body_details() {
        let d = tdir("manual");
        let mut j = Journal::open(d.clone());
        let mut dr = draft();
        dr.sex = Some(Sex::Female);
        dr.source = TargetSource::Manual;
        dr.manual_kcal = Some(1000.0);
        let r = preview(&mut j, &dr, TODAY);
        assert_eq!(r.kcal, Some(1307), "0.8 x resting energy (1,634) is above the 1,200 floor for this body");
        assert_eq!(r.adjustments.len(), 1);
        assert!(r.calculated_text.as_deref().is_some_and(|t| t.starts_with("Calculated for you: ")));
        save(&mut j, &dr, TODAY, 1, false).unwrap();
        let t = j.profile().unwrap().targets[0].clone();
        assert_eq!((t.kcal, t.manual_kcal), (1307, Some(1000)));
        assert!(t.calculated_kcal.is_some(), "the calculated number is kept for reference");

        // sex unknown and no other detail: the higher floor
        let mut bare = TargetsDraft { source: TargetSource::Manual, manual_kcal: Some(1000.0), ..Default::default() };
        bare.addback_pct = Some(100.0);
        let r = preview(&mut j, &bare, "2026-11-18");
        assert_eq!((r.kcal, r.can_save), (Some(1500), true), "{:?}", r.problems);
        bare.manual_kcal = None;
        assert_eq!(preview(&mut j, &bare, TODAY).problems[0].text, NEED_MANUAL);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn manual_under_18_saves_with_the_floor() {
        let d = tdir("under18");
        let mut j = Journal::open(d.clone());
        let mut dr = TargetsDraft {
            sex: Some(Sex::Female),
            birth_date: Some("2010-03-01".into()),
            height: HeightInput { unit: HeightUnit::Cm, cm: Some(160.0), ..Default::default() },
            weight: MassInput { value: Some(50.0), unit: MassUnit::Kg },
            weight_edited: true,
            activity: Some(Activity::Light),
            ..Default::default()
        };
        let r = preview(&mut j, &dr, TODAY);
        assert!(!r.can_save);
        assert!(r.problems.iter().any(|p| p.text == UNDER_18), "{:?}", r.problems);
        assert_eq!(save(&mut j, &dr, TODAY, 1, false), Err(UNDER_18.into()));
        assert!(j.profile().unwrap().targets.is_empty());

        dr.source = TargetSource::Manual;
        dr.manual_kcal = Some(1000.0);
        let r = preview(&mut j, &dr, TODAY);
        assert!(r.can_save, "{:?}", r.problems);
        assert_eq!(r.adjustments.iter().map(|a| (a.kind, a.reason.as_str())).next(), Some((AdjustKind::Age, UNDER_18)));
        assert_eq!(r.adjustments.iter().map(|a| a.kind).collect::<Vec<_>>(), [AdjustKind::Age, AdjustKind::Manual]);
        save(&mut j, &dr, TODAY, 2, false).unwrap();
        let t = &j.profile().unwrap().targets[0];
        assert_eq!((t.kcal, t.manual_kcal), (1200, Some(1000)));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unit_flip_converts_in_the_crate() {
        let d = tdir("flip");
        let mut j = Journal::open(d.clone());
        let mut dr = draft();
        dr.units = UnitChoice { height: HeightUnit::FtIn, weight: MassUnit::Lb, rate: MassUnit::Lb, goal_weight: MassUnit::Lb };
        dr.macros = MacroSpec { protein: MacroMode::PerKg(2.0), carbs: MacroMode::Rest, fat: MacroMode::Percent(30.0) };
        let i = preview(&mut j, &dr, TODAY).inputs;
        assert_eq!((i.height.unit, i.height.ft, i.height.inches), (HeightUnit::FtIn, Some(5.0), Some(10.9)));
        assert_eq!((i.weight.value, i.weight.unit), (Some(176.4), MassUnit::Lb));
        assert_eq!(i.rate.value, Some(1.1));
        assert_eq!((i.per_kg_unit, i.macros.protein), (MassUnit::Lb, MacroMode::PerKg(0.9)));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn save_asks_once_for_an_implausible_weight() {
        let d = tdir("ask");
        let mut j = Journal::open(d.clone());
        body::set_value(&mut j, "2026-11-10", TODAY, "weight", 150.0, 1).unwrap();
        j.save_dirty().unwrap();
        let before = bytes_of(&d);
        let r = save(&mut j, &draft(), TODAY, 2, false).unwrap(); // 80 kg typed, 70 kg below the last weigh-in
        assert_eq!(r.confirm.as_deref(), Some("That's 70 kg less than your last weigh-in. Save anyway?"));
        j.save_dirty().unwrap();
        assert_eq!(bytes_of(&d), before, "nothing written on the first press");
        assert!(j.profile().unwrap().targets.is_empty());
        // the same draft with the answer yes saves the weigh-in and the target
        let r = save(&mut j, &draft(), TODAY, 3, true).unwrap();
        assert_eq!(r.confirm, None);
        assert_eq!(weights(&mut j), [("2026-11-10".to_string(), 150.0), (TODAY.to_string(), 80.0)]);
        assert_eq!(j.profile().unwrap().targets.len(), 1);
        // exactly at max(2 kg, 5%) saves without asking; just past it asks
        let mut e = Journal::open(tdir("ask-edge"));
        body::set_value(&mut e, "2026-11-10", TODAY, "weight", 80.0, 1).unwrap();
        let mut at = draft();
        at.weight.value = Some(84.0);
        assert_eq!(save(&mut e, &at, TODAY, 2, false).unwrap().confirm, None);
        let mut past = draft();
        past.weight.value = Some(84.1);
        assert!(save(&mut e, &past, TODAY, 3, false).unwrap().confirm.is_some(), "4.1 kg from the 80 before today");
        // impossible values are refused outright, with or without the answer
        for bad in [0.0, -5.0, 500.1] {
            let mut x = draft();
            x.weight.value = Some(bad);
            assert!(save(&mut e, &x, "2026-11-19", 4, true).is_err(), "{bad}");
        }
        assert_eq!(weights(&mut e).len(), 2, "only the two that were accepted");
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(std::env::temp_dir().join(format!("rb-goals-ask-edge-{}", std::process::id())));
    }

    /// A target built on a typed 82 kg on 2026-11-01, then daily weigh-ins at `kg` up to `last` (a day in November).
    fn drifting(dir: &std::path::Path, kg: f64, last: u32) -> Journal {
        let mut j = Journal::open(dir.to_path_buf());
        let mut dr = draft();
        dr.weight.value = Some(82.0);
        save(&mut j, &dr, "2026-11-01", 1, false).unwrap();
        for day in 2..=last {
            body::set_value(&mut j, &format!("2026-11-{day:02}"), "2026-11-30", "weight", kg, 1).unwrap();
        }
        j
    }

    #[test]
    fn drift_is_offered_never_applied() {
        let d = tdir("drift");
        let mut j = drifting(&d, 78.0, 15); // 14 days at 78 kg: the trend is about 78.9
        j.save_dirty().unwrap();
        let profile_before = fs::read(d.join("profile.json")).unwrap();
        let basis = body::weight_basis(&mut j, "2026-11-15").unwrap();
        assert!(basis.trend && (basis.kg - 78.915).abs() < 0.01, "{basis:?}");
        let offer = drift(&mut j, "2026-11-15").expect("a drift of about 3.1 kg is offered");
        let want = compute(j.profile().unwrap(), Some(basis.kg), "2026-11-15").unwrap().kcal;
        assert_eq!(offer.kcal, want);
        assert_eq!(offer.text, format!("Your trend weight is {} lower. Update target to {} kcal?", mass_change_text(82.0 - basis.kg, MassUnit::Kg), kcal_text(want)));
        assert!(offer.text.starts_with("Your trend weight is 3.1 kg lower."), "{}", offer.text);
        // the view carries it, and looking changes nothing
        assert_eq!(targets_view(&mut j, "2026-11-15").drift, Some(offer));
        j.save_dirty().unwrap();
        assert_eq!(fs::read(d.join("profile.json")).unwrap(), profile_before);
        assert_eq!(j.profile().unwrap().targets.iter().map(|t| t.kcal).collect::<Vec<_>>(), [2503]);

        // under 2 kg (and under 2.5%): no offer
        let small = tdir("drift-small");
        let mut j = drifting(&small, 81.0, 15);
        assert_eq!(drift(&mut j, "2026-11-15"), None);
        // before there is a trend (four weigh-ins), no offer
        let few = tdir("drift-few");
        let mut j = drifting(&few, 70.0, 4);
        assert!(!body::weight_basis(&mut j, "2026-11-04").unwrap().trend);
        assert_eq!(drift(&mut j, "2026-11-04"), None);
        // a number the person chose is never second-guessed
        let manual = tdir("drift-manual");
        let mut j = drifting(&manual, 78.0, 15);
        j.profile_mut().unwrap().targets[0].manual_kcal = Some(2400);
        assert_eq!(drift(&mut j, "2026-11-15"), None);
        for x in [&d, &small, &few, &manual] {
            let _ = fs::remove_dir_all(x);
        }
    }

    #[test]
    fn a_macro_save_keeps_the_target_in_force() {
        let d = tdir("keep");
        let mut j = drifting(&d, 81.0, 15); // the trend moves under the drift threshold: no offer
        assert_eq!(drift(&mut j, "2026-11-15"), None);
        let before = j.profile().unwrap().targets[0].clone();
        let v = targets_view(&mut j, "2026-11-15");
        assert_eq!(v.result.kcal, Some(before.kcal), "the page shows the target in force, as the Diary does");
        assert_eq!(v.result.weight_basis_text.as_deref(), Some("This target uses 82.0 kg (the weight it was set with)."));
        assert!(!v.result.lines.is_empty() && v.result.calculated_text.is_none());
        let mut dr = v.draft;
        dr.macros = PRESETS[1].1;
        save(&mut j, &dr, "2026-11-15", 2, false).unwrap();
        let after = target_for_day(&j.profile().unwrap().targets, "2026-11-15").unwrap().clone();
        assert_eq!((after.kcal, after.calculated_kcal, after.floor_kcal, after.basis_weight_kg), (before.kcal, before.calculated_kcal, before.floor_kcal, before.basis_weight_kg));
        assert_eq!(after.macros, PRESETS[1].1);
        // a year older since the target was set (the age moves): the number is kept, today's figure is for reference only
        j.profile_mut().unwrap().birth_date = Some("2001-11-10".into());
        let v = targets_view(&mut j, "2026-11-15");
        assert_eq!(v.result.kcal, Some(before.kcal));
        assert!(v.result.lines.is_empty() && v.result.calculated_text.as_deref().is_some_and(|t| t.starts_with("With today's details: ")), "{:?}", v.result.calculated_text);
        j.profile_mut().unwrap().birth_date = Some("2000-05-01".into());
        // a changed body detail still recomputes, from the live weight
        let mut dr = targets_view(&mut j, "2026-11-15").draft;
        dr.activity = Some(Activity::Physical);
        save(&mut j, &dr, "2026-11-15", 3, false).unwrap();
        let moved = target_for_day(&j.profile().unwrap().targets, "2026-11-15").unwrap().clone();
        assert!(moved.kcal > before.kcal && moved.basis_weight_kg != before.basis_weight_kg, "{moved:?}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn accept_update_dates_a_new_target() {
        let d = tdir("accept");
        let mut j = drifting(&d, 78.0, 15);
        assert_eq!(accept_update(&mut j, "2026-11-06"), Err("There's no update to make.".into()), "a trend of about 80.4 is under the threshold");
        let offer = drift(&mut j, "2026-11-15").unwrap();
        let old = j.profile().unwrap().targets[0].clone();
        assert_eq!(accept_update(&mut j, "2026-11-20"), Ok(offer.kcal), "accepted on a later day, with the weigh-ins as of then");
        let p = j.profile().unwrap().clone();
        assert_eq!(p.targets.len(), 2);
        let new = &p.targets[1];
        assert_eq!((new.effective_from.as_str(), new.kcal, new.calculated_kcal), ("2026-11-20", offer.kcal, Some(offer.kcal)));
        assert!(new.basis_weight_kg.is_some_and(|w| w < 79.0 && w > 78.0), "{:?}", new.basis_weight_kg);
        assert_eq!((&new.macros, &new.templates, new.week, new.addback_pct), (&old.macros, &old.templates, old.week, old.addback_pct));
        // an earlier day keeps the old number
        assert_eq!(day_target(target_for_day(&p.targets, "2026-11-19").unwrap(), "2026-11-19", None).unwrap().kcal, 2503);
        assert_eq!(target_for_day(&p.targets, "2026-11-20").map(|t| t.kcal), Some(offer.kcal));
        // the update is used up: nothing more to offer
        assert_eq!(drift(&mut j, "2026-11-20"), None);
        assert_eq!(accept_update(&mut j, "2026-11-20"), Err("There's no update to make.".into()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_view_opens_on_what_is_saved() {
        let d = tdir("view");
        let mut j = Journal::open(d.clone());
        let v = targets_view(&mut j, TODAY);
        assert!(!v.has_target && v.read_only.is_none() && v.in_use_text.is_none() && v.earlier.is_empty());
        assert_eq!((v.activities.len(), v.presets.len(), v.presets[0].name), (4, 4, "Balanced"));
        let mut dr = draft();
        dr.templates = vec![Template { id: 1, name: "Training".into(), kcal_offset: 300, macros: MacroSpec::default() }];
        dr.week = [Some(1), None, None, None, None, None, None]; // Monday
        save(&mut j, &dr, "2026-11-10", 1, false).unwrap();
        save(&mut j, &dr, TODAY, 2, false).unwrap();
        let v = targets_view(&mut j, TODAY);
        assert!(v.has_target);
        assert_eq!(v.in_use_text.as_deref(), Some("In use from today."));
        assert_eq!(v.earlier.iter().map(|e| (e.date_text.as_str(), e.kcal_text.as_str())).collect::<Vec<_>>(), [("Nov 10", "2,475 kcal")]);
        assert_eq!(v.draft.week, [Some(1), None, None, None, None, None, None], "Monday first on the way out");
        assert_eq!(v.result.templates[0].offset_text, "+300 kcal");
        assert_eq!(v.result.templates[0].day_text, "= 2,775 kcal");
        assert_eq!(v.result.week[0].template_name.as_deref(), Some("Training"));
        let v = targets_view(&mut j, "2026-11-20");
        assert_eq!(v.in_use_text.as_deref(), Some("In use from Nov 17. Earlier days keep the target they had."));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_template_row_carries_its_own_macro_readouts() {
        let d = tdir("template-macros");
        let mut j = Journal::open(d.clone());
        let mut dr = draft();
        dr.templates = vec![Template { id: 1, name: "Training".into(), kcal_offset: 300, macros: MacroSpec::default() }];
        let m = &preview(&mut j, &dr, TODAY).templates[0].macros;
        assert_eq!(m.total_text, "Total 2,775 kcal · 100%");
        assert_eq!(m.rows[0].readout, "139 g · 20% · 555 kcal");
        assert!(m.problems.is_empty());
        // an empty protein box (sent as -1 by the page) is answered with the crate's own sentence, on that template's macros
        dr.templates[0].macros.protein = MacroMode::Grams(-1.0);
        let m = &preview(&mut j, &dr, TODAY).templates[0].macros;
        assert_eq!(m.problems.iter().map(|p| (p.field.as_str(), p.text.as_str())).collect::<Vec<_>>(), [("protein", "Enter a number of 0 or more for protein.")]);
        let _ = fs::remove_dir_all(&d);
    }
}
