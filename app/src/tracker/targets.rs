//! The Targets page: what the person typed (a draft), what it works out to (a preview), the view, the save and
//! the drift offer. The engine it calls (formula, clamps, macros, weekday plan) is in [`super::goals`].

use super::goals::*;
use crate::model;
use crate::tracker::body;
use crate::tracker::day::Day;
use crate::tracker::diary;
use crate::tracker::journal::{Journal, Profile};
use serde::{Deserialize, Serialize};

/// Height as typed: centimetres, or feet and inches, in the unit named.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct HeightInput {
    pub unit: HeightUnit,
    pub cm: Option<f64>,
    pub ft: Option<f64>,
    pub inches: Option<f64>,
}

/// A mass or a weekly rate as typed, in the unit named.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct MassInput {
    pub value: Option<f64>,
    pub unit: MassUnit,
}

/// Where the calorie number comes from: the calculation, or the person's own number (D-05).
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TargetSource {
    #[default]
    Calculated,
    Manual,
}

/// Everything the Targets page can set, in the units the person chose (D-02). `units` is the display unit per
/// field; each input's own `unit` (and `per_kg_unit` for every Grams per kg value in `macros` and in the
/// templates) is the unit its number is written in. `week` is Monday first, as the page shows it.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct TargetsDraft {
    pub units: UnitChoice,
    pub sex: Option<Sex>,
    pub birth_date: Option<String>,
    pub height: HeightInput,
    pub weight: MassInput,
    /// True only when the person typed a weight; Save then writes it as today's weigh-in (D-06).
    pub weight_edited: bool,
    pub activity: Option<Activity>,
    pub goal: GoalKind,
    pub rate: MassInput,
    pub goal_weight: MassInput,
    pub source: TargetSource,
    pub manual_kcal: Option<f64>,
    pub addback_pct: Option<f64>,
    pub macros: MacroSpec,
    pub per_kg_unit: MassUnit,
    pub templates: Vec<Template>,
    pub week: [Option<u32>; 7],
}

impl Default for TargetsDraft {
    fn default() -> Self {
        TargetsDraft {
            units: UnitChoice::default(),
            sex: None,
            birth_date: None,
            height: HeightInput::default(),
            weight: MassInput::default(),
            weight_edited: false,
            activity: None,
            goal: GoalKind::default(),
            rate: MassInput { value: Some(0.5), unit: MassUnit::Kg },
            goal_weight: MassInput::default(),
            source: TargetSource::default(),
            manual_kcal: None,
            addback_pct: Some(100.0),
            macros: MacroSpec::default(),
            per_kg_unit: MassUnit::Kg,
            templates: vec![],
            week: [None; 7],
        }
    }
}

/// The draft's numbers written in the units the person chose, so a unit picker change shows converted figures
/// without the page doing any maths. Mass, height and g per kg are rounded to 0.1 (rates to 0.01, as they read).
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct TargetsInputs {
    pub height: HeightInput,
    pub weight: MassInput,
    pub rate: MassInput,
    pub goal_weight: MassInput,
    pub macros: MacroSpec,
    pub templates: Vec<Template>,
    pub per_kg_unit: MassUnit,
}

/// One weekday template as the page lists it.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct TemplateRow {
    pub id: u32,
    pub name: String,
    /// "+300 kcal"
    pub offset_text: String,
    /// "= 2,400 kcal"
    pub day_text: String,
    /// "Protein 150 g · Carbs 250 g · Fat 67 g"
    pub macros_text: String,
    /// The template's own three macros at its day's calories, so its editor shows the same readouts as the default plan.
    pub macros: MacroResult,
}

/// What a draft works out to. Nothing here has been written.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct TargetsResult {
    /// A calorie number could be worked out.
    pub ready: bool,
    pub problems: Vec<Problem>,
    /// "Age 34"
    pub age_text: Option<String>,
    pub other_note: Option<&'static str>,
    pub lines: Vec<Line>,
    pub adjustments: Vec<Adjustment>,
    pub kcal: Option<i32>,
    pub kcal_text: String,
    /// The calculated figure, kept for reference while the person's own number is in use.
    pub calculated_text: Option<String>,
    /// The default plan at the base calories.
    pub macros: MacroResult,
    pub templates: Vec<TemplateRow>,
    pub week: Vec<WeekRow>,
    /// "Today, Tuesday: Training, 2,400 kcal."
    pub today_text: Option<String>,
    /// "This target uses 78.4 kg (trend weight)."
    pub weight_basis_text: Option<String>,
    pub inputs: TargetsInputs,
    pub can_save: bool,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct EarlierRow {
    pub date_text: String,
    pub kcal_text: String,
    pub macros_text: String,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct ActivityOption {
    pub id: Activity,
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct PresetOption {
    pub name: &'static str,
    pub spec: MacroSpec,
}

/// A recomputed target offered because the trend weight has moved (D-07). Nothing changes until it is accepted.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Drift {
    pub text: String,
    pub kcal: i32,
}

/// What the Targets page opens with: what is saved, what it works out to, and the lists the page offers.
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct TargetsView {
    pub draft: TargetsDraft,
    pub result: TargetsResult,
    pub has_target: bool,
    pub in_use_text: Option<String>,
    pub earlier: Vec<EarlierRow>,
    pub activities: Vec<ActivityOption>,
    pub presets: Vec<PresetOption>,
    pub rate_presets_kg: [f64; 4],
    pub rate_presets_lb: [f64; 4],
    pub drift: Option<Drift>,
    /// The profile's refusal sentence when it can't be changed (damaged or from a newer build).
    pub read_only: Option<String>,
}

pub const ADDBACK_BAD: &str = "Enter an add-back between 0 and 100%.";
pub const NEED_MANUAL: &str = "Enter your own number of calories.";
const NO_DRAFT_DAY: &str = "Today's date isn't valid.";

fn round_to(v: f64, decimals: i32) -> f64 {
    let f = 10f64.powi(decimals);
    (v * f).round() / f
}

fn height_cm_of(h: &HeightInput) -> Option<f64> {
    match h.unit {
        HeightUnit::Cm => h.cm,
        HeightUnit::FtIn => h.ft.map(|ft| cm_from_ft_in(ft, h.inches.unwrap_or(0.0))),
    }
}

fn height_input(cm: f64, unit: HeightUnit) -> HeightInput {
    match unit {
        HeightUnit::Cm => HeightInput { unit, cm: Some(round_to(cm, 1)), ft: None, inches: None },
        HeightUnit::FtIn => {
            let (ft, inches) = ft_in_parts(cm);
            HeightInput { unit, cm: None, ft: Some(ft as f64), inches: Some(inches) }
        }
    }
}

/// A height the draft shows unchanged keeps the stored unrounded value, so saving never drifts it (PITFALLS 9).
fn height_value(stored: Option<f64>, h: &HeightInput) -> Option<f64> {
    let same = |s: f64| match h.unit {
        HeightUnit::Cm => h.cm == Some(round_to(s, 1)),
        HeightUnit::FtIn => {
            let (ft, inches) = ft_in_parts(s);
            h.ft == Some(ft as f64) && h.inches.unwrap_or(0.0) == inches
        }
    };
    match stored {
        Some(s) if same(s) => Some(s),
        _ => height_cm_of(h),
    }
}

/// A mass in kg: the stored value when the draft shows it unchanged (at `decimals`), else what was typed.
fn kg_value(stored_kg: Option<f64>, m: &MassInput, decimals: i32) -> Option<f64> {
    let v = m.value?;
    match stored_kg {
        Some(s) if v == round_to(in_unit(s, m.unit), decimals) => Some(s),
        _ => Some(kg_from(v, m.unit)),
    }
}

fn mass_input(kg: Option<f64>, unit: MassUnit, decimals: i32) -> MassInput {
    MassInput { value: kg.map(|k| round_to(in_unit(k, unit), decimals)), unit }
}

/// Grams per kg to grams per lb and back: a pound is lighter, so the per-pound figure is smaller.
fn per_kg_in(v: f64, unit: MassUnit) -> f64 {
    match unit {
        MassUnit::Kg => v,
        MassUnit::Lb => v * KG_PER_LB,
    }
}

fn per_kg_to_kg(v: f64, unit: MassUnit, stored: Option<f64>) -> f64 {
    match (stored, unit) {
        (Some(s), _) if v == round_to(per_kg_in(s, unit), 1) => s,
        (_, MassUnit::Kg) => v,
        (_, MassUnit::Lb) => v / KG_PER_LB,
    }
}

fn spec_to_kg(spec: &MacroSpec, unit: MassUnit, stored: Option<&MacroSpec>) -> MacroSpec {
    let one = |m: MacroMode, s: Option<MacroMode>| match m {
        MacroMode::PerKg(v) => MacroMode::PerKg(per_kg_to_kg(v, unit, if let Some(MacroMode::PerKg(x)) = s { Some(x) } else { None })),
        other => other,
    };
    MacroSpec { protein: one(spec.protein, stored.map(|s| s.protein)), carbs: one(spec.carbs, stored.map(|s| s.carbs)), fat: one(spec.fat, stored.map(|s| s.fat)) }
}

fn spec_in_unit(spec: &MacroSpec, unit: MassUnit) -> MacroSpec {
    let one = |m: MacroMode| match m {
        MacroMode::PerKg(v) => MacroMode::PerKg(round_to(per_kg_in(v, unit), 1)),
        other => other,
    };
    MacroSpec { protein: one(spec.protein), carbs: one(spec.carbs), fat: one(spec.fat) }
}

/// Template macros to kg, each judged against the same template in the record in force.
fn templates_to_kg(t: &[Template], unit: MassUnit, stored: Option<&TargetSet>) -> Vec<Template> {
    t.iter()
        .map(|x| {
            let old = stored.and_then(|s| s.templates.iter().find(|o| o.id == x.id)).map(|o| &o.macros);
            Template { macros: spec_to_kg(&x.macros, unit, old), ..x.clone() }
        })
        .collect()
}

/// Monday-first (the page) to Sunday-first (the record).
fn sunday_first(w: [Option<u32>; 7]) -> [Option<u32>; 7] {
    std::array::from_fn(|i| w[(i + 6) % 7])
}

/// Sunday-first (the record) to Monday-first (the page).
fn monday_first(w: [Option<u32>; 7]) -> [Option<u32>; 7] {
    std::array::from_fn(|i| w[(i + 1) % 7])
}

fn macros_line(m: &MacroResult) -> String {
    m.rows.iter().map(|r| format!("{} {} g", r.name, r.grams.round() as i32)).collect::<Vec<_>>().join(" · ")
}



/// What one draft works out to, and the record Save would write for it. Reads only.
struct Work {
    result: TargetsResult,
    set: Option<TargetSet>,
    profile: Profile,
    weight_kg: Option<f64>,
}

fn work(j: &mut Journal, d: &TargetsDraft, today: &str) -> Work {
    let stored = j.profile().cloned().unwrap_or_default();
    let in_force = target_for_day(&stored.targets, today).cloned();
    let mut problems: Vec<Problem> = vec![];
    let mut profile = stored.clone();
    profile.units = d.units;
    profile.sex = d.sex;
    profile.birth_date = d.birth_date.clone().filter(|s| !s.trim().is_empty());
    profile.height_cm = height_value(stored.height_cm, &d.height);
    profile.activity = d.activity;
    profile.goal = d.goal;
    profile.rate_kg_week = kg_value(Some(stored.rate_kg_week), &d.rate, 2).unwrap_or(stored.rate_kg_week);
    profile.goal_weight_kg = kg_value(stored.goal_weight_kg, &d.goal_weight, 1);
    if d.source == TargetSource::Calculated && d.goal != GoalKind::Maintain && d.rate.value.is_none() {
        problems.push(problem("rate", "Enter how much you want to change each week.".into()));
    }
    if d.goal == GoalKind::Maintain && !(profile.rate_kg_week.is_finite() && (0.0..=2.0).contains(&profile.rate_kg_week)) {
        problems.push(problem("rate", "That weekly rate isn't possible. Enter a number from 0 to 2 kg a week.".into()));
    }
    if let Some(g) = profile.goal_weight_kg {
        if let Err(e) = body::check_value(body::MeasureKind::Mass, g) {
            problems.push(problem("goal weight", e.into()));
        }
    }
    if Day::parse(today).is_err() {
        problems.push(problem("today", NO_DRAFT_DAY.into()));
    }

    let basis = if d.weight_edited { None } else { body::weight_basis(j, today) };
    let weight_kg = if d.weight_edited { d.weight.value.map(|v| kg_from(v, d.weight.unit)) } else { basis.as_ref().map(|b| b.kg) };
    let age = profile.birth_date.as_deref().and_then(|b| age_on(b, today).ok());
    // D-07: with the body details unchanged, a calculated target in force keeps its calories and basis weight;
    // only an accepted drift offer moves them (CR-01). Per-kg macros still follow the live weight, as on the Diary.
    // Assumes the stored body details are the ones the record was built from: only `save` and `accept_update` write them.
    let kept = in_force.as_ref().filter(|s| {
        !d.weight_edited
            && d.source == TargetSource::Calculated
            && s.manual_kcal.is_none()
            && s.basis_weight_kg.is_some()
            && (profile.sex, &profile.birth_date, profile.height_cm, profile.activity, profile.goal, profile.rate_kg_week)
                == (stored.sex, &stored.birth_date, stored.height_cm, stored.activity, stored.goal, stored.rate_kg_week)
    });
    let calc_weight = kept.map_or(weight_kg, |s| s.basis_weight_kg);
    let breakdown = if problems.iter().any(|p| p.field == "today") { Err(vec![]) } else { compute(&profile, calc_weight, today) };

    let (mut kcal, mut calculated, mut manual, mut floor) = (None, None, None, 0);
    let (mut lines, mut adjustments, mut calculated_text) = (vec![], vec![], None);
    match d.source {
        TargetSource::Calculated => match &breakdown {
            Ok(b) => {
                (kcal, calculated, floor) = match kept {
                    Some(s) => (Some(s.kcal), s.calculated_kcal, s.floor_kcal.min(s.kcal)),
                    None => (Some(b.kcal), Some(b.kcal), b.floor_kcal),
                };
                if kcal == Some(b.kcal) {
                    (lines, adjustments) = (b.lines.clone(), b.adjustments.clone());
                } else {
                    // a birthday since the target was set: today's lines wouldn't add up to the kept number
                    calculated_text = Some(format!("With today's details: {} kcal", kcal_text(b.kcal)));
                }
            }
            Err(ps) => problems.extend(ps.iter().cloned()),
        },
        TargetSource::Manual => {
            // The calculation is for reference only: a missing body detail never blocks the person's own number,
            // but a detail that was typed must still make sense.
            if let Some(b) = profile.birth_date.as_deref() {
                match age_on(b, today) {
                    Err(e) => problems.push(problem("birth date", e)),
                    Ok(_) if b < "1900-01-01" => problems.push(problem("birth date", "The birth date isn't possible. Enter a date from 1900 on.".into())),
                    Ok(_) => {}
                }
            }
            if let Some(cm) = profile.height_cm {
                if let Err(e) = check_height_cm(cm) {
                    problems.push(problem("height", e));
                }
            }
            if let Some(kg) = weight_kg {
                if let Err(e) = body::check_value(body::MeasureKind::Mass, kg) {
                    problems.push(problem("weight", e.into()));
                }
            }
            match d.manual_kcal {
                None => problems.push(problem("calories", NEED_MANUAL.into())),
                Some(k) => {
                    let resting = match (profile.sex, age, profile.height_cm, weight_kg) {
                        (Some(s), Some(a), Some(cm), Some(kg)) if check_height_cm(cm).is_ok() && kg.is_finite() && kg > 0.0 => bmr(kg, cm, a, s),
                        _ => 0.0,
                    };
                    // sex unknown: the higher floor, as for Other
                    let sex = profile.sex.unwrap_or(Sex::Other);
                    match manual_target(k, sex, resting, age) {
                        Ok((used, adj)) => {
                            (kcal, manual, floor, adjustments) = (Some(used), Some(k.round() as i32), floor_kcal(sex, resting), adj);
                            calculated = breakdown.as_ref().ok().map(|b| b.kcal);
                            calculated_text = calculated.map(|c| format!("Calculated for you: {} kcal", kcal_text(c)));
                        }
                        Err(e) => problems.push(problem("calories", e)),
                    }
                }
            }
        }
    }

    let addback = match d.addback_pct {
        Some(v) if v.is_finite() && (0.0..=100.0).contains(&v) && v.fract() == 0.0 => v as u8,
        _ => {
            problems.push(problem("add-back", ADDBACK_BAD.into()));
            100
        }
    };
    let spec = spec_to_kg(&d.macros, d.per_kg_unit, in_force.as_ref().map(|s| &s.macros));
    let mut templates = templates_to_kg(&d.templates, d.per_kg_unit, in_force.as_ref());
    clean_templates(&mut templates);
    problems.extend(check_templates(&templates));
    let week = sunday_first(d.week);
    if week.iter().flatten().any(|id| !templates.iter().any(|t| t.id == *id)) {
        problems.push(problem("week", "A weekday uses a template that isn't in the list.".into()));
    }

    let macros = resolve_macros(&spec, kcal.unwrap_or(0), weight_kg);
    if kcal.is_some() {
        problems.extend(macros.problems.iter().cloned());
    }
    let set = kcal.map(|k| TargetSet {
        effective_from: today.into(),
        kcal: k,
        calculated_kcal: calculated,
        manual_kcal: manual,
        floor_kcal: floor,
        basis_weight_kg: calc_weight,
        macros: spec,
        templates: templates.clone(),
        week,
        addback_pct: addback,
    });

    let mut template_rows = vec![];
    let (mut week_rows_out, mut today_text) = (vec![], None);
    if let (Some(set), Some(base)) = (&set, kcal) {
        for t in &templates {
            let day_kcal = (base + t.kcal_offset).max(set.floor_kcal);
            let m = resolve_macros(&t.macros, day_kcal, weight_kg);
            problems.extend(m.problems.iter().map(|p| problem(&format!("template {} {}", t.id, p.field), p.text.clone())));
            template_rows.push(TemplateRow { id: t.id, name: t.name.clone(), offset_text: format!("{} kcal", signed_kcal_text(t.kcal_offset)), day_text: format!("= {} kcal", kcal_text(day_kcal)), macros_text: macros_line(&m), macros: m });
        }
        week_rows_out = week_rows(set);
        if let Ok(dt) = day_target(set, today, weight_kg) {
            let name = diary::day_number(today).map(|n| WEEK_LABELS[(model::weekday(n) as usize + 6) % 7]).unwrap_or("");
            today_text = Some(match &dt.template {
                Some((_, t)) => format!("Today, {name}: {t}, {} kcal.", kcal_text(dt.kcal)),
                None => format!("Today, {name}: {} kcal.", kcal_text(dt.kcal)),
            });
        }
    }

    let weight_basis_text = if d.weight_edited {
        weight_kg.map(|k| format!("This target uses {} (the weight you typed).", mass_text(k, d.units.weight)))
    } else if let Some(k) = kept.and_then(|s| s.basis_weight_kg) {
        Some(format!("This target uses {} (the weight it was set with).", mass_text(k, d.units.weight)))
    } else {
        basis.as_ref().map(|b| format!("This target uses {} ({}).", mass_text(b.kg, d.units.weight), if b.trend { "trend weight" } else { "latest weigh-in" }))
    };
    let inputs = TargetsInputs {
        height: profile.height_cm.map_or(HeightInput { unit: d.units.height, ..Default::default() }, |cm| height_input(cm, d.units.height)),
        weight: mass_input(d.weight.value.map(|v| kg_from(v, d.weight.unit)), d.units.weight, 1),
        // an emptied box stays empty through a unit change (WR-04)
        rate: mass_input(d.rate.value.map(|_| profile.rate_kg_week), d.units.rate, 2),
        goal_weight: mass_input(profile.goal_weight_kg, d.units.goal_weight, 1),
        macros: spec_in_unit(&spec, d.units.weight),
        templates: templates.iter().map(|t| Template { macros: spec_in_unit(&t.macros, d.units.weight), ..t.clone() }).collect(),
        per_kg_unit: d.units.weight,
    };
    let can_save = kcal.is_some() && problems.is_empty();
    let result = TargetsResult {
        ready: kcal.is_some(),
        problems,
        age_text: age.map(|a| format!("Age {a}")),
        other_note: profile.sex.and_then(other_note),
        lines,
        adjustments,
        kcal,
        kcal_text: kcal.map_or("–".to_string(), kcal_text),
        calculated_text,
        macros,
        templates: template_rows,
        week: week_rows_out,
        today_text,
        weight_basis_text,
        inputs,
        can_save,
    };
    Work { result, set, profile, weight_kg }
}

/// What a draft works out to, exactly as [`save`] would store it. Writes nothing.
pub fn preview(j: &mut Journal, d: &TargetsDraft, today: &str) -> TargetsResult {
    work(j, d, today).result
}

/// The draft that shows what is saved, in the chosen units. A field shown unchanged is saved back as its stored
/// unrounded value, so opening and saving the page never drifts a number.
fn draft_from(p: &Profile, set: Option<&TargetSet>, weight_kg: Option<f64>) -> TargetsDraft {
    let u = p.units;
    let mut d = TargetsDraft {
        units: u,
        sex: p.sex,
        birth_date: p.birth_date.clone(),
        height: p.height_cm.map_or(HeightInput { unit: u.height, ..Default::default() }, |cm| height_input(cm, u.height)),
        weight: mass_input(weight_kg, u.weight, 1),
        activity: p.activity,
        goal: p.goal,
        rate: mass_input(Some(p.rate_kg_week), u.rate, 2),
        goal_weight: mass_input(p.goal_weight_kg, u.goal_weight, 1),
        per_kg_unit: u.weight,
        ..Default::default()
    };
    if let Some(s) = set {
        d.source = if s.manual_kcal.is_some() { TargetSource::Manual } else { TargetSource::Calculated };
        d.manual_kcal = s.manual_kcal.map(f64::from);
        d.addback_pct = Some(f64::from(s.addback_pct));
        d.macros = spec_in_unit(&s.macros, u.weight);
        d.templates = s.templates.iter().map(|t| Template { macros: spec_in_unit(&t.macros, u.weight), ..t.clone() }).collect();
        d.week = monday_first(s.week);
    }
    d
}

pub fn targets_view(j: &mut Journal, today: &str) -> TargetsView {
    let (profile, read_only) = match j.profile() {
        Ok(p) => (p.clone(), None),
        Err(e) => (Profile::default(), Some(e)),
    };
    let in_force = target_for_day(&profile.targets, today).cloned();
    let weight = body::weight_basis(j, today).map(|b| b.kg);
    let draft = draft_from(&profile, in_force.as_ref(), weight);
    let result = preview(j, &draft, today);
    let in_use_text = in_force.as_ref().map(|s| {
        if s.effective_from == today {
            "In use from today.".to_string()
        } else {
            format!("In use from {}. Earlier days keep the target they had.", date_text(&s.effective_from))
        }
    });
    let mut earlier = vec![];
    for s in profile.targets.iter().rev().filter(|s| Some(&s.effective_from) != in_force.as_ref().map(|f| &f.effective_from)) {
        let w = body::weight_basis(j, &s.effective_from).map(|b| b.kg);
        let macros_text = day_target(s, &s.effective_from, w).map(|t| macros_line(&t.macros)).unwrap_or_default();
        earlier.push(EarlierRow { date_text: date_text(&s.effective_from), kcal_text: format!("{} kcal", kcal_text(s.kcal)), macros_text });
    }
    let activities = [Activity::Sitting, Activity::Light, Activity::OnFeet, Activity::Physical].map(|a| ActivityOption { id: a, name: a.name(), description: a.description() }).to_vec();
    TargetsView {
        draft,
        result,
        has_target: in_force.is_some(),
        in_use_text,
        earlier,
        activities,
        presets: PRESETS.iter().map(|(name, spec)| PresetOption { name: *name, spec: *spec }).collect(),
        rate_presets_kg: [0.25, 0.5, 0.75, 1.0],
        rate_presets_lb: [0.5, 1.0, 1.5, 2.0],
        drift: drift(j, today),
        read_only,
    }
}

/// What a save did: `confirm` is the sentence to ask when a typed weight is far from the last weigh-in. Nothing
/// is written until the person says yes (D-21).
#[derive(Clone, Serialize, Debug, PartialEq, Default)]
pub struct Saved {
    pub confirm: Option<String>,
}

/// The least the trend weight must have moved from the weight the target was built on, in kg, for an update to be offered (D-07).
pub const DRIFT_MIN_KG: f64 = 2.0;
/// The same as a share of that weight; the larger of the two applies.
pub const DRIFT_MIN_SHARE: f64 = 0.025;
/// The least the recomputed target must differ, in kcal.
pub const DRIFT_MIN_KCAL: i32 = 25;


const NO_UPDATE: &str = "There's no update to make.";

/// The record in force, the target recomputed from the trend weight, and that weight, when an update is worth
/// offering: a calculated target (never a number the person chose) built on a weight, a trend weight at least
/// max(2 kg, 2.5%) from it, and a recomputed target at least 25 kcal away. Reads only.
fn drift_parts(j: &mut Journal, today: &str) -> Option<(TargetSet, Breakdown, f64)> {
    let p = j.profile().ok()?.clone();
    let set = target_for_day(&p.targets, today)?.clone();
    if set.manual_kcal.is_some() {
        return None;
    }
    let basis = set.basis_weight_kg?;
    let w = body::weight_basis(j, today).filter(|w| w.trend)?;
    if (w.kg - basis).abs() < DRIFT_MIN_KG.max(DRIFT_MIN_SHARE * basis) {
        return None;
    }
    let b = compute(&p, Some(w.kg), today).ok()?;
    ((b.kcal - set.kcal).abs() >= DRIFT_MIN_KCAL).then_some((set, b, w.kg))
}

/// An update to the calorie target, offered when the trend weight has moved. Dismissing it is not stored (R-14): it
/// is offered again while the condition holds. Reads only.
pub fn drift(j: &mut Journal, today: &str) -> Option<Drift> {
    let (set, b, kg) = drift_parts(j, today)?;
    let basis = set.basis_weight_kg?;
    let unit = j.profile().ok()?.units.weight;
    let way = if kg < basis { "lower" } else { "higher" };
    Some(Drift { text: format!("Your trend weight is {} {way}. Update target to {} kcal?", mass_change_text(kg - basis, unit), kcal_text(b.kcal)), kcal: b.kcal })
}

/// Write the offered update as a new target effective `today`: the record in force with the recomputed calories,
/// floor and basis weight, and its macros, templates, week and add-back as they were. Earlier days keep their
/// numbers. The only writer a drift ever has; returns the new calories.
pub fn accept_update(j: &mut Journal, today: &str) -> Result<i32, String> {
    Day::parse(today)?;
    let (set, b, kg) = drift_parts(j, today).ok_or(NO_UPDATE)?;
    let new = TargetSet { effective_from: today.into(), kcal: b.kcal, calculated_kcal: Some(b.kcal), floor_kcal: b.floor_kcal, basis_weight_kg: Some(kg), ..set };
    put_target(j.profile_mut()?, new);
    Ok(b.kcal)
}

/// Save a draft: the body details and unit choices, one target effective `today` (a second save the same day
/// replaces that day's record, earlier records are untouched), and, when a weight was typed, today's weigh-in.
/// Nothing is written when anything is wrong; the first problem's sentence comes back. A typed weight far from the last
/// weigh-in comes back as `Saved::confirm` and is saved only when `confirmed` is true.
pub fn save(j: &mut Journal, d: &TargetsDraft, today: &str, now: u64, confirmed: bool) -> Result<Saved, String> {
    let w = work(j, d, today);
    let Some(set) = w.set.filter(|_| w.result.can_save) else {
        return Err(w.result.problems.first().map_or_else(|| "Nothing to save yet.".to_string(), |p| p.text.clone()));
    };
    // a typed weight is a weigh-in, so it gets the same ask-once check as Log Weight (D-06, D-21); nothing is written before the answer
    if d.weight_edited {
        let kg = w.weight_kg.ok_or("Enter your weight.")?;
        match body::check_weight(body::previous_weight(j, today), kg, d.units.weight) {
            body::WeightCheck::Ok => {}
            body::WeightCheck::Refused(s) => return Err(s),
            body::WeightCheck::Confirm(s) if !confirmed => return Ok(Saved { confirm: Some(s) }),
            body::WeightCheck::Confirm(_) => {}
        }
    }
    let p = j.profile_mut()?;
    p.sex = w.profile.sex;
    p.birth_date = w.profile.birth_date;
    p.height_cm = w.profile.height_cm;
    p.activity = w.profile.activity;
    p.goal = w.profile.goal;
    p.rate_kg_week = w.profile.rate_kg_week;
    p.goal_weight_kg = w.profile.goal_weight_kg;
    p.units = w.profile.units;
    put_target(p, set);
    if d.weight_edited {
        let kg = w.weight_kg.ok_or("Enter your weight.")?;
        body::set_value(j, today, today, "weight", kg, now)?;
    }
    Ok(Saved { confirm: None })
}
