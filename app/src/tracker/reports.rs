//! The Reports tab: averages over a range of finished days (ending yesterday) that have food logged. Calories eaten and
//! their split by macro, calories burned as resting energy plus logged exercise, what was left against the target, macros
//! against their goals, and every nutrient. A value that isn't known (a food or an activity without it) is never counted
//! as 0: it leaves that day out of that one average, and the text says how many days the average is from. Reads only.

use crate::model::civil_from_days;
use crate::tracker::body::{self, BodyRange};
use crate::tracker::diary;
use crate::tracker::goals;
use crate::tracker::journal::Journal;
use crate::tracker::trends::{self, DaySummary};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Slice {
    pub name: &'static str,
    /// protein | carbs | fat, for the colour
    pub tone: &'static str,
    pub kcal: f64,
    /// The share in whole percent; the three always add to 100.
    pub pct: u32,
    /// "Protein 24% · 480 kcal"
    pub text: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MacroLine {
    pub name: &'static str,
    pub tone: &'static str,
    pub eaten: Option<f64>,
    pub goal: Option<f64>,
    /// Eaten is above the goal as printed (never judged on a hidden decimal).
    pub over: bool,
    /// "96 g of 150 g a day", or what is known
    pub text: String,
    pub note: Option<String>,
}

/// One line of a two-column summary ("Resting energy", "1,780 kcal").
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Row {
    pub label: &'static str,
    pub value_text: String,
    pub total: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct NutrientLine {
    pub name: &'static str,
    pub value_text: String,
    /// "From 8 of 10 days; …" when some days didn't report it
    pub note: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Report {
    /// "Nov 7 to Nov 20 (14 days)"
    pub range_text: String,
    pub days_logged: u32,
    /// "10 of 14 days have food logged. Today counts once it's over."
    pub days_text: String,
    /// Said instead of everything below when no day in the range has food.
    pub empty_text: Option<String>,
    /// "2,000 kcal a day on average", or why it isn't known
    pub consumed_text: String,
    /// The number alone, "2,000", and its unit line, for the middle of the ring.
    pub consumed_figure: Option<String>,
    pub consumed_unit: &'static str,
    pub consumed_note: Option<String>,
    pub split: Vec<Slice>,
    /// The ring's screen-reader sentence, or why there is no ring.
    pub split_label: String,
    pub split_note: Option<String>,
    pub burned: Vec<Row>,
    pub burned_note: Option<String>,
    pub remaining_text: String,
    /// false when no day had both a target and calories, so the page offers Set Your Targets
    pub remaining_known: bool,
    pub remaining_over: bool,
    pub remaining_note: Option<String>,
    pub macros: Vec<MacroLine>,
    pub nutrients: Vec<NutrientLine>,
}

fn kcal(v: f64) -> String {
    goals::kcal_text(v.round() as i32)
}

fn avg(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// "From 8 of 10 days; {why}." when some days are left out of an average.
fn from_note(known: usize, of: usize, why: &str) -> Option<String> {
    (known < of).then(|| format!("From {known} of {of} days; {why}."))
}

const NOT_REPORTED: &str = "some foods didn't report it";
const UNKNOWN: &str = "Not reported";

/// One decimal, no trailing zero, grouped like calories: "1,619.2", "44.1", "925".
fn g1(v: f64) -> String {
    let t = format!("{v:.1}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    let (whole, frac) = t.split_once('.').map_or((t, None), |(w, f)| (w, Some(f)));
    let grouped = whole.parse::<i32>().map_or(whole.to_string(), goals::kcal_text);
    frac.map_or(grouped.clone(), |f| format!("{grouped}.{f}"))
}

/// Whole-percent shares of `parts` that add to exactly 100: the largest remainders take the leftover points.
fn shares(parts: &[f64]) -> Vec<u32> {
    let total: f64 = parts.iter().sum();
    let exact: Vec<f64> = parts.iter().map(|p| p * 100.0 / total).collect();
    let mut out: Vec<u32> = exact.iter().map(|e| e.floor() as u32).collect();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
    for &i in order.iter().take(100 - out.iter().sum::<u32>() as usize) {
        out[i] += 1;
    }
    out
}

/// The report for `range` ending yesterday (today isn't finished). `range` is the Body page's choice of lengths; all time
/// starts at the first day with food.
pub fn report(j: &mut Journal, range: BodyRange, today: &str) -> Result<Report, String> {
    let yesterday = diary::day_number(today)? - 1;
    let this_year = civil_from_days(yesterday + 1).0;
    let start = match range.days() {
        Some(n) => yesterday - (n - 1),
        None => trends::first_day(j).unwrap_or(yesterday).max(yesterday - trends::MAX_DAYS + 1).min(yesterday),
    };
    let mut days = trends::day_summaries(j, &trends::day_text(start), &trends::day_text(yesterday))?;
    if range.days().is_none() {
        // all time begins at the first logged day, not the first of its month
        let first = days.iter().position(|d| d.logged).unwrap_or(days.len().saturating_sub(1));
        days.drain(..first);
    }
    let start = days.first().map_or(start, |d| d.n);
    let span = days.len();
    let range_text = format!("{} to {} ({span} day{})", body::date_short(start, this_year), body::date_short(yesterday, this_year), if span == 1 { "" } else { "s" });
    let logged: Vec<&DaySummary> = days.iter().filter(|d| d.logged).collect();
    let n = logged.len();
    let days_text = format!("{n} of {span} day{} {} food logged. Today counts once it's over.", if span == 1 { "" } else { "s" }, if n == 1 { "has" } else { "have" });
    let blank = Report {
        range_text,
        days_logged: n as u32,
        days_text,
        empty_text: None,
        consumed_text: UNKNOWN.into(),
        consumed_figure: None,
        consumed_unit: "kcal a day",
        consumed_note: None,
        split: vec![],
        split_label: String::new(),
        split_note: None,
        burned: vec![],
        burned_note: None,
        remaining_text: "–".into(),
        remaining_known: false,
        remaining_over: false,
        remaining_note: None,
        macros: vec![],
        nutrients: vec![],
    };
    if n == 0 {
        return Ok(Report { empty_text: Some("No food logged in this range. Pick a longer range, or log meals in the Diary.".into()), ..blank });
    }

    // each logged day's exercise: the person's own number, else the frozen estimate; an activity with neither leaves the
    // day's exercise unknown (as the Diary leaves it out and says so), and a day with no activity burned 0
    let mut exercise_by_day: BTreeMap<i32, Option<f64>> = BTreeMap::new();
    let months: std::collections::BTreeSet<String> = logged.iter().map(|d| d.day[..7].to_string()).collect();
    for m in &months {
        let Ok(month) = j.month(m) else { continue };
        for e in month.exercise.iter().filter(|e| e.deleted_at.is_none()) {
            if let Ok(dn) = diary::day_number(&e.day) {
                let slot = exercise_by_day.entry(dn).or_insert(Some(0.0));
                *slot = slot.zip(e.kcal_override.or(e.est_kcal)).map(|(a, b)| a + b);
            }
        }
    }
    let exercise_on = |d: &DaySummary| exercise_by_day.get(&d.n).copied().unwrap_or(Some(0.0));
    let profile = j.profile().ok().cloned();
    let targets = profile.as_ref().map(|p| p.targets.clone()).unwrap_or_default();

    // calories eaten and the split
    let cal: Vec<f64> = logged.iter().filter_map(|d| d.sum.calories.map(f64::from)).collect();
    let consumed = avg(&cal);
    let split_days: Vec<[f64; 3]> = logged.iter().filter_map(|d| Some([f64::from(d.sum.protein_g?) * 4.0, f64::from(d.sum.carbs_g?) * 4.0, f64::from(d.sum.fat_g?) * 9.0])).collect();
    let mut split = vec![];
    let split_label = if split_days.is_empty() {
        "Calories by macro: not reported for some foods.".to_string()
    } else {
        let k = split_days.len() as f64;
        let parts: Vec<f64> = (0..3).map(|i| split_days.iter().map(|d| d[i]).sum::<f64>() / k).collect();
        if parts.iter().sum::<f64>() <= 0.0 {
            "Calories by macro: the foods logged have no protein, carbs or fat.".to_string()
        } else {
            let pct = shares(&parts);
            split = [("Protein", "protein"), ("Carbs", "carbs"), ("Fat", "fat")]
                .into_iter()
                .zip(parts.iter().zip(&pct))
                .map(|((name, tone), (&v, &p))| Slice { name, tone, kcal: v, pct: p, text: format!("{name} {p}% · {} kcal", kcal(v)) })
                .collect();
            format!("Calories by macro, daily average: {}.", split.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(", "))
        }
    };

    // burned: resting energy at each day's weight and age (adults only, as Targets) plus that day's logged exercise,
    // averaged over the days where both are known so the two parts and the total cover the same days
    let resting_on = |d: &DaySummary| -> Option<f64> {
        let p = profile.as_ref()?;
        let age = goals::age_on(p.birth_date.as_deref()?, &d.day).ok().filter(|a| *a >= 18)?;
        let cm = goals::check_height_cm(p.height_cm?).ok()?;
        let kg = d.weight_kg.filter(|kg| kg.is_finite() && *kg > 0.0)?;
        Some(goals::bmr(kg, cm, age, p.sex?))
    };
    let burned_days: Vec<(f64, f64)> = logged.iter().filter_map(|d| Some((resting_on(d)?, exercise_on(d)?))).collect();
    let (burned, burned_note) = if burned_days.is_empty() {
        let any_resting = logged.iter().any(|d| resting_on(d).is_some());
        let why = if any_resting {
            "An activity on these days has no calories, so the total can't be worked out. Add its calories in the Diary."
        } else {
            "Add your sex, birth date and height on Targets and a weigh-in on Body to see resting energy (adults only)."
        };
        (vec![], Some(why.to_string()))
    } else {
        let rest = avg(&burned_days.iter().map(|b| b.0).collect::<Vec<_>>()).unwrap_or(0.0);
        let ex = avg(&burned_days.iter().map(|b| b.1).collect::<Vec<_>>()).unwrap_or(0.0);
        let rows = vec![
            Row { label: "Resting energy", value_text: format!("{} kcal", kcal(rest)), total: false },
            Row { label: "Logged exercise", value_text: format!("{} kcal", kcal(ex)), total: false },
            Row { label: "A day on average", value_text: format!("{} kcal", kcal(rest + ex)), total: true },
        ];
        (rows, from_note(burned_days.len(), n, "the others are missing a weigh-in or body detail, or an activity had no calories"))
    };

    // remaining against the target in force each day, with that day's add-back share of exercise
    let remaining: Vec<f64> = logged
        .iter()
        .filter_map(|d| {
            let set = goals::target_for_day(&targets, &d.day)?;
            let target = f64::from(d.goals.as_ref()?.kcal);
            let food = f64::from(d.sum.calories?);
            Some(target + exercise_on(d)? * f64::from(set.addback_pct) / 100.0 - food)
        })
        .collect();
    let left = avg(&remaining);
    let remaining_over = left.is_some_and(|r| r.round() < 0.0);
    let remaining_text = left.map_or("–".to_string(), |r| if remaining_over { format!("Over by {} kcal a day on average", kcal(-r)) } else { format!("{} kcal left a day on average", kcal(r)) });
    let remaining_note = if remaining.is_empty() {
        Some("No target was in force on these days, or their calories weren't reported. Set a target on Targets.".to_string())
    } else {
        from_note(remaining.len(), n, "the others had no target, a food without calories or an activity without calories")
    };

    // macros against their goals: both averages from the same days, the ones with a goal and the macro reported; with no
    // goal on any day, what was eaten alone
    let macro_line = |name: &'static str, tone: &'static str, eaten: &dyn Fn(&DaySummary) -> Option<f64>, goal: &dyn Fn(&trends::DayGoals) -> f64| {
        let both: Vec<(f64, f64)> = logged.iter().filter_map(|d| Some((eaten(d)?, goal(d.goals.as_ref()?)))).collect();
        if both.is_empty() {
            let e: Vec<f64> = logged.iter().filter_map(|d| eaten(d)).collect();
            let text = avg(&e).map_or(format!("{UNKNOWN} for some foods"), |v| format!("{} g a day", g1(v)));
            return MacroLine { name, tone, eaten: avg(&e), goal: None, over: false, text, note: from_note(e.len(), n, NOT_REPORTED) };
        }
        let e = avg(&both.iter().map(|b| b.0).collect::<Vec<_>>()).unwrap_or(0.0);
        let g = avg(&both.iter().map(|b| b.1).collect::<Vec<_>>()).unwrap_or(0.0);
        let (e_text, g_text) = (g1(e), format!("{}", g.round()));
        let over = e_text.parse::<f64>().unwrap_or(e) > g.round();
        MacroLine { name, tone, eaten: Some(e), goal: Some(g), over, text: format!("{e_text} g of {g_text} g a day"), note: from_note(both.len(), n, "the others had no target or didn't report it") }
    };
    let net = |d: &DaySummary| diary::net_carbs(&d.sum).map(f64::from);
    let macros = vec![
        macro_line("Protein", "protein", &|d| d.sum.protein_g.map(f64::from), &|g| g.protein_g),
        macro_line("Net Carbs", "carbs", &net, &|g| g.net_carbs_g),
        macro_line("Fat", "fat", &|d| d.sum.fat_g.map(f64::from), &|g| g.fat_g),
    ];

    // every nutrient, averaged over the days that reported it
    let field = |f: &dyn Fn(&DaySummary) -> Option<f64>| -> (Option<f64>, usize) {
        let v: Vec<f64> = logged.iter().filter_map(|d| f(d)).collect();
        (avg(&v), v.len())
    };
    let line = |name: &'static str, (v, known): (Option<f64>, usize), unit: &str, whole: bool| NutrientLine {
        name,
        value_text: v.map_or(UNKNOWN.to_string(), |v| if whole { format!("{} {unit}", kcal(v)) } else { format!("{} {unit}", g1(v)) }),
        note: if v.is_some() { from_note(known, n, NOT_REPORTED) } else { None },
    };
    let nutrients = vec![
        line("Calories", field(&|d| d.sum.calories.map(f64::from)), "kcal", true),
        line("Protein", field(&|d| d.sum.protein_g.map(f64::from)), "g", false),
        line("Carbs", field(&|d| d.sum.carbs_g.map(f64::from)), "g", false),
        line("Fiber", field(&|d| d.sum.fiber_g.map(f64::from)), "g", false),
        line("Net carbs", field(&net), "g", false),
        line("Sugar", field(&|d| d.sum.sugar_g.map(f64::from)), "g", false),
        line("Fat", field(&|d| d.sum.fat_g.map(f64::from)), "g", false),
        line("Sodium", field(&|d| d.sum.sodium_mg.map(f64::from)), "mg", true),
    ];

    Ok(Report {
        consumed_text: consumed.map_or(format!("{UNKNOWN} for some foods"), |c| format!("{} kcal a day on average", kcal(c))),
        consumed_figure: consumed.map(kcal),
        consumed_note: if consumed.is_some() { from_note(cal.len(), n, NOT_REPORTED) } else { None },
        split_note: if split.is_empty() { None } else { from_note(split_days.len(), n, NOT_REPORTED) },
        split,
        split_label,
        burned,
        burned_note,
        remaining_text,
        remaining_known: left.is_some(),
        remaining_over,
        remaining_note,
        macros,
        nutrients,
        ..blank
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MealType, Nutrients};
    use crate::tracker::journal::ExerciseEntry;
    use std::fs;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-reports-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn eat(j: &mut Journal, day: &str, n: Nutrients) {
        let e = diary::NewEntry { name: "Meal".into(), source: crate::tracker::journal::FoodRef::Custom, grams: None, amount: 1.0, unit: "serving".into(), basis: n, per_100g: false };
        diary::add(j, day, MealType::Lunch, e, 1).unwrap();
    }

    fn full(kcal: f32, p: f32, c: f32, f: f32) -> Nutrients {
        Nutrients { calories: Some(kcal), protein_g: Some(p), carbs_g: Some(c), fat_g: Some(f), fiber_g: Some(5.0), sugar_g: Some(20.0), sodium_mg: Some(1000.0) }
    }

    fn adult(j: &mut Journal, birth: &str) {
        let p = j.profile_mut().unwrap();
        p.sex = Some(goals::Sex::Male);
        p.birth_date = Some(birth.into());
        p.height_cm = Some(180.0);
    }

    fn workout(j: &mut Journal, day: &str, est: Option<f64>, over: Option<f64>) {
        let w = ExerciseEntry { uid: diary::new_uid(), day: day.into(), name: "Run".into(), minutes: 30.0, est_kcal: est, kcal_override: over, ..Default::default() };
        j.month_mut(&day[..7]).unwrap().exercise.push(w);
    }

    #[test]
    fn nothing_logged_says_so() {
        let d = tdir("empty");
        let mut j = Journal::open(d.clone());
        let r = report(&mut j, BodyRange::W2, "2026-11-21").unwrap();
        assert_eq!((r.range_text.as_str(), r.days_text.as_str(), r.days_logged), ("Nov 7 to Nov 20 (14 days)", "0 of 14 days have food logged. Today counts once it's over.", 0));
        assert!(r.empty_text.is_some() && r.nutrients.is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn averages_cover_logged_days_and_say_what_is_missing() {
        let d = tdir("avg");
        let mut j = Journal::open(d.clone());
        adult(&mut j, "2000-05-01");
        goals::put_target_in(&mut j, goals::TargetSet { effective_from: "2026-11-01".into(), kcal: 2000, floor_kcal: 1200, addback_pct: 50, ..Default::default() });
        body::set_value(&mut j, "2026-11-01", "2026-11-21", "weight", 80.0, 1).unwrap();
        eat(&mut j, "2026-11-19", full(1800.0, 100.0, 200.0, 60.0));
        eat(&mut j, "2026-11-20", Nutrients { calories: Some(2200.0), protein_g: Some(120.0), carbs_g: Some(240.0), fat_g: Some(70.0), ..Default::default() });
        eat(&mut j, "2026-11-21", full(9000.0, 1.0, 1.0, 1.0)); // today: never counted
        workout(&mut j, "2026-11-20", Some(500.0), Some(400.0)); // the person's own number beats the estimate
        let r = report(&mut j, BodyRange::W2, "2026-11-21").unwrap();
        assert_eq!(r.days_logged, 2);
        assert_eq!((r.consumed_text.as_str(), r.consumed_figure.as_deref(), r.consumed_note.as_deref()), ("2,000 kcal a day on average", Some("2,000"), None));
        assert_eq!(r.split.iter().map(|s| s.pct).sum::<u32>(), 100);
        assert_eq!(r.split[0].kcal, 440.0, "protein (100 + 120) g x 4 / 2");
        // resting energy at 80 kg, age 26: 10*80 + 6.25*180 - 5*26 + 5 = 1,800; exercise (0 + 400) / 2 = 200
        assert_eq!(r.burned.iter().map(|b| b.value_text.as_str()).collect::<Vec<_>>(), ["1,800 kcal", "200 kcal", "2,000 kcal"]);
        assert_eq!(r.burned_note, None);
        // remaining: (2000 - 1800) and (2000 + 400 * 50% - 2200) average to 100
        assert_eq!((r.remaining_text.as_str(), r.remaining_over, r.remaining_known), ("100 kcal left a day on average", false, true));
        let sugar = r.nutrients.iter().find(|n| n.name == "Sugar").unwrap();
        assert_eq!((sugar.value_text.as_str(), sugar.note.as_deref()), ("20 g", Some("From 1 of 2 days; some foods didn't report it.")));
        assert_eq!(r.nutrients.iter().find(|n| n.name == "Net carbs").unwrap().value_text, "217.5 g", "(195 + 240) / 2 from the raw sum, unreported fiber as 0");
        let protein = r.macros.iter().find(|m| m.name == "Protein").unwrap();
        assert_eq!((protein.eaten, protein.note.as_deref()), (Some(110.0), None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn an_activity_without_calories_is_never_counted_as_zero() {
        let d = tdir("noexkcal");
        let mut j = Journal::open(d.clone());
        adult(&mut j, "2000-05-01");
        body::set_value(&mut j, "2026-11-01", "2026-11-21", "weight", 80.0, 1).unwrap();
        eat(&mut j, "2026-11-19", full(1800.0, 100.0, 200.0, 60.0));
        eat(&mut j, "2026-11-20", full(1800.0, 100.0, 200.0, 60.0));
        workout(&mut j, "2026-11-20", None, None);
        let r = report(&mut j, BodyRange::W2, "2026-11-21").unwrap();
        assert_eq!(r.burned.iter().map(|b| b.value_text.as_str()).collect::<Vec<_>>(), ["1,800 kcal", "0 kcal", "1,800 kcal"], "only Nov 19 is known");
        assert_eq!(r.burned_note.as_deref(), Some("From 1 of 2 days; the others are missing a weigh-in or body detail, or an activity had no calories."));
        assert!(!r.remaining_known && r.remaining_text == "–", "no target was in force");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn under_eighteen_gets_no_resting_energy() {
        let d = tdir("young");
        let mut j = Journal::open(d.clone());
        adult(&mut j, "2010-05-01");
        body::set_value(&mut j, "2026-11-01", "2026-11-21", "weight", 60.0, 1).unwrap();
        eat(&mut j, "2026-11-20", full(1800.0, 100.0, 200.0, 60.0));
        let r = report(&mut j, BodyRange::W2, "2026-11-21").unwrap();
        assert!(r.burned.is_empty() && r.burned_note.as_deref().is_some_and(|n| n.contains("adults only")), "{:?}", r.burned_note);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn macro_goals_and_eaten_come_from_the_same_days() {
        let d = tdir("macrodays");
        let mut j = Journal::open(d.clone());
        let macros = goals::MacroSpec { protein: goals::MacroMode::Grams(150.0), carbs: goals::MacroMode::Grams(200.0), fat: goals::MacroMode::Grams(60.0) };
        goals::put_target_in(&mut j, goals::TargetSet { effective_from: "2026-11-20".into(), kcal: 2000, floor_kcal: 1200, macros, ..Default::default() });
        eat(&mut j, "2026-11-19", full(1000.0, 50.0, 100.0, 30.0)); // before any target
        eat(&mut j, "2026-11-20", full(2000.0, 160.0, 200.0, 60.0));
        let r = report(&mut j, BodyRange::W2, "2026-11-21").unwrap();
        let p = r.macros.iter().find(|m| m.name == "Protein").unwrap();
        assert_eq!((p.eaten, p.goal, p.over, p.text.as_str()), (Some(160.0), Some(150.0), true, "160 g of 150 g a day"), "Nov 19 has no goal, so it's out of both");
        assert_eq!(p.note.as_deref(), Some("From 1 of 2 days; the others had no target or didn't report it."));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn all_time_starts_at_the_first_logged_day() {
        let d = tdir("all");
        let mut j = Journal::open(d.clone());
        eat(&mut j, "2026-11-17", full(1500.0, 80.0, 150.0, 50.0));
        let r = report(&mut j, BodyRange::All, "2026-11-21").unwrap();
        assert_eq!((r.range_text.as_str(), r.days_text.as_str()), ("Nov 17 to Nov 20 (4 days)", "1 of 4 days has food logged. Today counts once it's over."));
        // first food only today: a one-day range ending yesterday, with nothing in it
        let t = tdir("all-today");
        let mut k = Journal::open(t.clone());
        eat(&mut k, "2026-11-21", full(1500.0, 80.0, 150.0, 50.0));
        let r = report(&mut k, BodyRange::All, "2026-11-21").unwrap();
        assert_eq!((r.days_logged, r.empty_text.is_some()), (0, true));
        for x in [&d, &t] {
            let _ = fs::remove_dir_all(x);
        }
    }

    #[test]
    fn a_split_of_nothing_says_why_and_shares_add_to_100() {
        let d = tdir("zero");
        let mut j = Journal::open(d.clone());
        eat(&mut j, "2026-11-20", full(0.0, 0.0, 0.0, 0.0));
        let r = report(&mut j, BodyRange::W2, "2026-11-21").unwrap();
        assert!(r.split.is_empty());
        assert_eq!(r.split_label, "Calories by macro: the foods logged have no protein, carbs or fat.");
        assert_eq!(shares(&[1.0, 1.0, 1.0]), [34, 33, 33]);
        assert_eq!(shares(&[200.0, 0.0, 0.0]), [100, 0, 0]);
        assert_eq!((g1(1619.24), g1(44.1), g1(925.0), g1(0.04), g1(12345.0)), ("1,619.2".to_string(), "44.1".to_string(), "925".to_string(), "0".to_string(), "12,345".to_string()));
        let _ = fs::remove_dir_all(&d);
    }
}
