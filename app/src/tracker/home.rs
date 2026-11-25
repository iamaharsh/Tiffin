//! The Home dashboard's view: today at a glance, planned meals, recent activity, how often the macro goals were met,
//! the logging streak, a daily energy chart stacked by macro and a weight-change chart. Reads only; the page prints it.

use crate::model::civil_from_days;
use crate::tracker::body::{self, BodyRange, Chart, ChartRow, Point};
use crate::foods::RecipeFood;
use crate::model::MyFood;
use crate::tracker::diary::{self, Budget, BudgetState, PlannedMeal, Totals};
use crate::tracker::fit::{self, Fits};
use crate::tracker::goals::{self, MassUnit};
use crate::tracker::journal::Journal;
use crate::tracker::trends::{self, HitRate};
use serde::Serialize;

/// Recent activity reaches this many days back and shows at most this many items.
pub const RECENT_DAYS: i32 = 7;
pub const RECENT_MAX: usize = 6;

/// One day's bar: kcal from protein, carbs and fat (4, 4 and 9 kcal per g). A macro not reported for some food that day
/// leaves the split unknown, and the bar is drawn as one neutral block of the day's calories.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EnergyBar {
    pub day: i32,
    pub protein_kcal: Option<f64>,
    pub carbs_kcal: Option<f64>,
    pub fat_kcal: Option<f64>,
    pub kcal: Option<f64>,
    pub target: Option<i32>,
    pub logged: bool,
}

/// The chart's Show as Table row.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EnergyRow {
    pub date_text: String,
    pub kcal_text: String,
    pub macros_text: String,
    pub target_text: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EnergyChart {
    pub bars: Vec<EnergyBar>,
    pub x_min: i32,
    pub x_max: i32,
    pub y_max: f64,
    /// The sentence a screen reader hears for the whole chart.
    pub label: String,
    /// Said in place of the chart when it has nothing to draw (or can't be read).
    pub empty_text: Option<String>,
    pub table: Vec<EnergyRow>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RecentItem {
    /// food | exercise | weight
    pub kind: &'static str,
    pub title: String,
    pub detail: String,
    pub when_text: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HomeView {
    pub date_text: String,
    pub budget: Option<Budget>,
    pub budget_state: BudgetState,
    pub budget_note: Option<String>,
    pub totals: Totals,
    pub planned: Vec<PlannedMeal>,
    /// Foods and recipes that fit the protein, net carbs and fat left today.
    pub fits: Fits,
    pub recent: Vec<RecentItem>,
    pub hit_rates: Vec<HitRate>,
    /// Said instead of the rates when the targets can't be read.
    pub rates_note: Option<String>,
    pub streak: u32,
    pub streak_text: String,
    pub energy: EnergyChart,
    pub weight: Chart,
    pub weight_change_text: Option<String>,
}

const NO_FOOD: &str = "No food logged in this range.";
const NO_WEIGH_INS: &str = "No weigh-ins in this range. Try a longer range.";

fn kcal(v: f64) -> String {
    goals::kcal_text(v.round() as i32)
}

fn when_text(n: i32, today: i32, this_year: i32) -> String {
    match today - n {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => body::date_long(n, this_year),
    }
}

/// A signed change in the person's unit, with a true minus: "−1.2 kg", "+0.4 lb", "0 kg".
fn change_text(kg: f64, unit: MassUnit) -> String {
    let v = goals::in_unit(kg, unit);
    let t = body::trimmed1(v.abs());
    let sign = if t == "0" { "" } else if v < 0.0 { "\u{2212}" } else { "+" };
    format!("{sign}{t} {}", body::mass_word(unit))
}

/// Where both charts start: the range's length back from today, or for all time the earliest food entry or body record
/// (so the two charts share an x axis), at most [`trends::MAX_DAYS`] back.
fn chart_start(j: &mut Journal, range: BodyRange, today_n: i32) -> i32 {
    match range.days() {
        Some(n) => today_n - (n - 1),
        None => trends::first_day(j).unwrap_or(today_n).min(body::range_start(j, range, today_n)).max(today_n - trends::MAX_DAYS + 1),
    }
}

fn energy_chart(j: &mut Journal, start: i32, range: BodyRange, today_n: i32, this_year: i32) -> Result<EnergyChart, String> {
    let days = trends::day_summaries(j, &trends::day_text(start), &trends::day_text(today_n))?;
    let f64o = |v: Option<f32>| v.map(f64::from);
    let bars: Vec<EnergyBar> = days
        .iter()
        .map(|d| EnergyBar {
            day: d.n,
            protein_kcal: d.logged.then(|| f64o(d.totals.protein).map(|g| g * 4.0)).flatten(),
            carbs_kcal: d.logged.then(|| f64o(d.totals.carbs).map(|g| g * 4.0)).flatten(),
            fat_kcal: d.logged.then(|| f64o(d.totals.fat).map(|g| g * 9.0)).flatten(),
            kcal: d.logged.then(|| f64o(d.totals.calories)).flatten(),
            target: d.goals.as_ref().map(|g| g.kcal),
            logged: d.logged,
        })
        .collect();
    let top = bars.iter().flat_map(|b| [b.kcal, Some(b.protein_kcal.unwrap_or(0.0) + b.carbs_kcal.unwrap_or(0.0) + b.fat_kcal.unwrap_or(0.0)), b.target.map(f64::from)]).flatten().fold(0.0_f64, f64::max);
    let logged: Vec<&EnergyBar> = bars.iter().filter(|b| b.logged).collect();
    let known: Vec<f64> = logged.iter().filter_map(|b| b.kcal).collect();
    let name = body::range_name(range);
    let label = if known.is_empty() {
        format!("Calories, {name}: {}", NO_FOOD.to_lowercase())
    } else {
        let avg = known.iter().sum::<f64>() / known.len() as f64;
        format!("Calories, {name}: average {} kcal over {} logged day{}.", kcal(avg), known.len(), if known.len() == 1 { "" } else { "s" })
    };
    let table = logged
        .iter()
        .rev()
        .map(|b| EnergyRow {
            date_text: body::date_long(b.day, this_year),
            kcal_text: b.kcal.map_or("–".into(), |k| format!("{} kcal", kcal(k))),
            macros_text: match (b.protein_kcal, b.carbs_kcal, b.fat_kcal) {
                (Some(p), Some(c), Some(f)) => format!("Protein {} · Carbs {} · Fat {} kcal", kcal(p), kcal(c), kcal(f)),
                _ => "Not reported for some foods".into(),
            },
            target_text: b.target.map_or("–".into(), |t| format!("{} kcal", goals::kcal_text(t))),
        })
        .collect();
    Ok(EnergyChart { empty_text: logged.is_empty().then(|| NO_FOOD.to_string()), bars, x_min: start, x_max: today_n, y_max: (top * 1.1).max(500.0), label, table })
}

fn weight_chart(j: &mut Journal, start: i32, range: BodyRange, today: &str, today_n: i32, this_year: i32) -> Result<(Chart, Option<String>), String> {
    let unit = j.profile().map(|p| p.units.weight).unwrap_or_default();
    let goal_kg = j.profile().ok().and_then(|p| p.goal_weight_kg);
    let w = trends::weight_change(j, goal_kg, &trends::day_text(start), today)?;
    let conv = |kg: f64| goals::in_unit(kg, unit);
    let points: Vec<Point> = w.points.iter().map(|&(day, ch)| Point { day, raw: None, trend: Some(conv(ch)) }).collect();
    // the change line always shows 0, so it counts as data: a far goal then sits clear of the 0 line too
    let data: Vec<f64> = points.iter().filter_map(|p| p.trend).chain([0.0]).collect();
    let (y_min, y_max, goal, arrow) = body::goal_scale(&data, w.goal_change.map(conv), conv(1.0));
    let word = body::mass_word(unit);
    let name = body::range_name(range);
    let change = w.change_kg.map(|c| change_text(c, unit));
    let label = match &change {
        Some(c) => format!("Weight change, {name}: {c}."),
        None if points.is_empty() => format!("Weight change, {name}: no weigh-ins."),
        None => format!("Weight change, {name}: one weigh-in so far."),
    };
    let table = w.points.iter().rev().map(|&(day, ch)| ChartRow { date_text: body::date_long(day, this_year), raw_text: String::new(), trend_text: change_text(ch, unit) }).collect();
    let chart = Chart {
        empty_text: points.is_empty().then_some(NO_WEIGH_INS),
        points,
        x_min: start,
        x_max: today_n,
        y_min,
        y_max,
        goal,
        goal_text: w.goal_change.map(|g| format!("Goal {}{arrow}", change_text(g, unit))),
        unit_text: word,
        label,
        table,
    };
    Ok((chart, change))
}

/// Recent food, exercise and weigh-ins from the last [`RECENT_DAYS`], newest logged first.
fn recent(j: &mut Journal, today_n: i32, this_year: i32) -> Vec<RecentItem> {
    let unit = j.profile().map(|p| p.units.weight).unwrap_or_default();
    let from = today_n - (RECENT_DAYS - 1);
    let months: std::collections::BTreeSet<String> = (from..=today_n).map(|n| trends::day_text(n)[..7].to_string()).collect();
    let mut items: Vec<(u64, RecentItem)> = vec![];
    let inside = |day: &str| diary::day_number(day).ok().filter(|n| (from..=today_n).contains(n));
    for m in &months {
        let Ok(month) = j.month(m) else { continue };
        for e in month.entries.iter().filter(|e| e.deleted_at.is_none()) {
            let Some(n) = inside(&e.day) else { continue };
            let cal = diary::entry_nutrients(e).calories.map_or("–".to_string(), |c| format!("{} kcal", kcal(f64::from(c))));
            items.push((e.logged_at, RecentItem { kind: "food", title: e.name.clone(), detail: format!("{cal} · {}", diary::slot_label(e.slot)), when_text: when_text(n, today_n, this_year) }));
        }
        for e in month.exercise.iter().filter(|e| e.deleted_at.is_none()) {
            let Some(n) = inside(&e.day) else { continue };
            let k = e.kcal_override.or(e.est_kcal).map_or("–".to_string(), |k| format!("\u{2212}{} kcal", kcal(k)));
            items.push((e.logged_at, RecentItem { kind: "exercise", title: e.name.clone(), detail: format!("{k} · {} min", body::trimmed1(e.minutes)), when_text: when_text(n, today_n, this_year) }));
        }
        for e in month.body.iter().filter(|e| e.deleted_at.is_none() && e.measure == "weight") {
            let Some(n) = inside(&e.day) else { continue };
            items.push((e.logged_at, RecentItem { kind: "weight", title: "Weight".into(), detail: goals::mass_text(e.value, unit), when_text: when_text(n, today_n, this_year) }));
        }
    }
    items.sort_by(|a, b| b.0.cmp(&a.0));
    items.into_iter().take(RECENT_MAX).map(|(_, i)| i).collect()
}

/// Everything Home shows for `today`. `planned` is today's meal plan, and `mine` and `recipes` the foods the suggestions
/// draw on, all built by the shell from the library.
pub fn home_view(j: &mut Journal, today: &str, range: BodyRange, planned: Vec<PlannedMeal>, mine: &[MyFood], recipes: &[RecipeFood]) -> Result<HomeView, String> {
    let today_n = diary::day_number(today)?;
    let this_year = civil_from_days(today_n).0;
    let day = diary::day_view(j, today, today)?;
    let (hit_rates, rates_note) = match trends::hit_rates(j, today) {
        Ok(r) => (r, None),
        Err(e) => (vec![], Some(format!("Macro goals can't be checked right now: {e} Restart Tiffin, or restore a backup in Settings if it keeps happening."))),
    };
    let streak = trends::streak(j, today)?;
    let streak_text = match streak {
        0 => "Log a meal to start a streak.".to_string(),
        1 => "1 day logged in a row.".to_string(),
        n => format!("{n} days logged in a row."),
    };
    let start = chart_start(j, range, today_n);
    // never "no food logged" when the diary simply couldn't be read
    let energy = energy_chart(j, start, range, today_n, this_year).unwrap_or_else(|e| {
        let said = format!("Calories can't be shown right now: {e}");
        EnergyChart { bars: vec![], x_min: start, x_max: today_n, y_max: 500.0, label: said.clone(), empty_text: Some(said), table: vec![] }
    });
    let (weight, weight_change_text) = weight_chart(j, start, range, today, today_n, this_year)?;
    let fits = fit::fits(day.budget.as_ref(), mine, recipes);
    Ok(HomeView {
        date_text: body::date_long(today_n, this_year),
        budget: day.budget,
        budget_state: day.budget_state,
        budget_note: day.budget_note,
        totals: day.totals,
        planned,
        fits,
        recent: recent(j, today_n, this_year),
        hit_rates,
        rates_note,
        streak,
        streak_text,
        energy,
        weight,
        weight_change_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MealType;
    use crate::tracker::journal::ExerciseEntry;
    use std::fs;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = crate::scratch_dir().join(format!("rb-home-{}-{}", name, crate::test_pid()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn eat(j: &mut Journal, day: &str, kcal: f32, p: Option<f32>, c: Option<f32>, f: Option<f32>, at: u64) {
        diary::add(j, day, MealType::Dinner, diary::quick("Stew", Some(kcal), p, c, f).unwrap(), at).unwrap();
    }

    #[test]
    fn an_empty_journal_opens_calmly() {
        let d = tdir("empty");
        let mut j = Journal::open(d.clone());
        let v = home_view(&mut j, "2026-11-20", BodyRange::W2, vec![], &[], &[]).unwrap();
        assert_eq!((v.date_text.as_str(), v.streak, v.streak_text.as_str()), ("Fri, Nov 20", 0, "Log a meal to start a streak."));
        assert_eq!((v.budget_state, v.recent.len(), v.hit_rates.len(), v.rates_note.as_deref()), (BudgetState::Never, 0, 6, None));
        assert_eq!((v.energy.bars.len(), v.energy.empty_text.as_deref(), v.energy.y_max), (14, Some(NO_FOOD), 500.0));
        assert_eq!((v.weight.empty_text, v.weight_change_text.as_deref()), (Some(NO_WEIGH_INS), None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn bars_split_by_macro_and_unknown_stays_one_block() {
        let d = tdir("bars");
        let mut j = Journal::open(d.clone());
        eat(&mut j, "2026-11-19", 1000.0, Some(50.0), Some(100.0), Some(40.0), 1);
        eat(&mut j, "2026-11-20", 600.0, None, Some(60.0), Some(20.0), 2);
        let v = home_view(&mut j, "2026-11-20", BodyRange::W2, vec![], &[], &[]).unwrap();
        let b = &v.energy.bars;
        assert_eq!((b.len(), b[0].day + 13, b[13].day), (14, b[13].day, diary::day_number("2026-11-20").unwrap()), "two weeks ending today");
        assert_eq!((b[12].protein_kcal, b[12].carbs_kcal, b[12].fat_kcal, b[12].kcal), (Some(200.0), Some(400.0), Some(360.0), Some(1000.0)));
        assert_eq!((b[13].protein_kcal, b[13].kcal, b[11].logged, b[11].kcal), (None, Some(600.0), false, None), "unknown protein; an empty day draws nothing");
        assert!((v.energy.y_max - 1100.0).abs() < 1e-9);
        assert_eq!(v.energy.label, "Calories, last 2 weeks: average 800 kcal over 2 logged days.");
        assert_eq!(v.energy.table.iter().map(|r| r.macros_text.as_str()).collect::<Vec<_>>(), ["Not reported for some foods", "Protein 200 · Carbs 400 · Fat 360 kcal"], "newest first");
        assert_eq!((v.streak, v.streak_text.as_str()), (2, "2 days logged in a row."));
        assert_eq!(v.recent.iter().map(|r| (r.title.as_str(), r.detail.as_str(), r.when_text.as_str())).collect::<Vec<_>>(), [("Stew", "600 kcal · Dinner", "Today"), ("Stew", "1,000 kcal · Dinner", "Yesterday")]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn weight_change_is_in_the_persons_unit_with_a_true_minus() {
        let d = tdir("weight");
        let mut j = Journal::open(d.clone());
        j.profile_mut().unwrap().goal_weight_kg = Some(75.0);
        for (day, kg) in [("2026-11-10", 80.0), ("2026-11-15", 79.0), ("2026-11-20", 78.0)] {
            body::set_value(&mut j, day, "2026-11-20", "weight", kg, 1).unwrap();
        }
        let v = home_view(&mut j, "2026-11-20", BodyRange::M1, vec![], &[], &[]).unwrap();
        let change = v.weight_change_text.clone().unwrap();
        assert!(change.starts_with('\u{2212}') && change.ends_with(" kg"), "{change}");
        assert_eq!(v.weight.points.len(), 3);
        assert_eq!(v.weight.points[0].trend, Some(0.0));
        assert!(v.weight.goal.unwrap() < v.weight.points[2].trend.unwrap() && v.weight.y_min < v.weight.goal.unwrap() && v.weight.y_max >= 0.0, "the goal line and 0 are inside the y range");
        assert_eq!(v.weight.goal_text.as_deref(), Some("Goal \u{2212}5 kg"));
        assert_eq!(v.recent[0].kind, "weight");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_far_goal_sits_on_the_edge_instead_of_flattening_the_line() {
        let d = tdir("fargoal");
        let mut j = Journal::open(d.clone());
        j.profile_mut().unwrap().goal_weight_kg = Some(60.0);
        for (day, kg) in [("2026-11-10", 80.0), ("2026-11-15", 80.2), ("2026-11-20", 80.4)] {
            body::set_value(&mut j, day, "2026-11-20", "weight", kg, 1).unwrap();
        }
        let v = home_view(&mut j, "2026-11-20", BodyRange::M1, vec![], &[], &[]).unwrap();
        let w = &v.weight;
        assert!(w.y_min > -3.0, "the scale fits the weigh-ins, not the -20 kg goal: {}", w.y_min);
        assert_eq!(w.goal, Some(w.y_min), "the goal line sits on the bottom edge");
        assert_eq!(w.goal_text.as_deref(), Some("Goal \u{2212}20 kg \u{2193}"));
        let data: Vec<f64> = w.points.iter().filter_map(|p| p.trend).chain([0.0]).collect();
        let span = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max) - data.iter().cloned().fold(f64::INFINITY, f64::min);
        assert!(data.iter().all(|t| t - w.goal.unwrap() >= 0.15 * span - 1e-9), "clear of the weigh-ins and the 0 line");
        // a far goal above (gaining) goes on the top edge, pointing up
        j.profile_mut().unwrap().goal_weight_kg = Some(100.0);
        let w = home_view(&mut j, "2026-11-20", BodyRange::M1, vec![], &[], &[]).unwrap().weight;
        assert_eq!(w.goal, Some(w.y_max));
        assert!(w.goal_text.as_deref().is_some_and(|t| t.ends_with(" \u{2191}")), "{:?}", w.goal_text);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn recent_is_capped_newest_first_and_a_week_long() {
        let d = tdir("recent");
        let mut j = Journal::open(d.clone());
        eat(&mut j, "2026-11-13", 100.0, None, None, None, 1); // 8 days back: outside the week
        for (i, day) in ["2026-11-15", "2026-11-16", "2026-11-17", "2026-11-18", "2026-11-19"].iter().enumerate() {
            eat(&mut j, day, 300.0 + i as f32, None, None, None, 10 + i as u64);
        }
        let e = ExerciseEntry { uid: diary::new_uid(), day: "2026-11-20".into(), name: "Walk".into(), minutes: 30.0, est_kcal: Some(120.0), logged_at: 99, ..Default::default() };
        j.month_mut("2026-11").unwrap().exercise.push(e);
        let v = home_view(&mut j, "2026-11-20", BodyRange::W2, vec![], &[], &[]).unwrap();
        assert_eq!(v.recent.len(), RECENT_MAX);
        assert_eq!((v.recent[0].kind, v.recent[0].detail.as_str(), v.recent[0].when_text.as_str()), ("exercise", "\u{2212}120 kcal · 30 min", "Today"), "logged last, shown first");
        assert_eq!(v.recent.iter().map(|r| r.detail.as_str()).collect::<Vec<_>>()[1..], ["304 kcal · Dinner", "303 kcal · Dinner", "302 kcal · Dinner", "301 kcal · Dinner", "300 kcal · Dinner"]);
        // with room to spare, the 7th day back is in and the 8th is out
        let few = tdir("recent-few");
        let mut k = Journal::open(few.clone());
        eat(&mut k, "2026-11-13", 100.0, None, None, None, 1);
        eat(&mut k, "2026-11-14", 200.0, None, None, None, 2);
        let v = home_view(&mut k, "2026-11-20", BodyRange::W2, vec![], &[], &[]).unwrap();
        assert_eq!(v.recent.iter().map(|r| r.detail.as_str()).collect::<Vec<_>>(), ["200 kcal · Dinner"]);
        for x in [&d, &few] {
            let _ = fs::remove_dir_all(x);
        }
    }

    #[test]
    fn the_target_sets_the_top_when_it_is_above_every_bar() {
        let d = tdir("ymax");
        let mut j = Journal::open(d.clone());
        goals::put_target_in(&mut j, goals::TargetSet { effective_from: "2026-11-01".into(), kcal: 2400, floor_kcal: 1200, ..Default::default() });
        eat(&mut j, "2026-11-20", 600.0, Some(30.0), Some(60.0), Some(20.0), 1);
        let v = home_view(&mut j, "2026-11-20", BodyRange::W2, vec![], &[], &[]).unwrap();
        assert!((v.energy.y_max - 2640.0).abs() < 1e-9, "{}", v.energy.y_max);
        assert!(v.energy.bars.iter().all(|b| b.target == Some(2400)));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn an_unreadable_profile_says_so_instead_of_judging() {
        let d = tdir("badprofile");
        fs::write(d.join("profile.json"), "{ not json").unwrap();
        let mut j = Journal::open(d.clone());
        eat(&mut j, "2026-11-19", 600.0, None, None, None, 1);
        let v = home_view(&mut j, "2026-11-20", BodyRange::W2, vec![], &[], &[]).unwrap();
        assert!(v.hit_rates.is_empty() && v.rates_note.as_deref().is_some_and(|n| n.starts_with("Macro goals can't be checked right now")), "{:?}", v.rates_note);
        assert!(v.energy.empty_text.as_deref().is_some_and(|t| t.starts_with("Calories can't be shown right now")), "never 'no food logged': {:?}", v.energy.empty_text);
        assert_eq!(v.streak, 1, "the streak needs no profile");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_tiny_change_reads_as_zero_without_a_sign() {
        assert_eq!(change_text(0.02, MassUnit::Kg), "0 kg");
        assert_eq!(change_text(-0.04, MassUnit::Kg), "0 kg");
        assert_eq!(change_text(-1.26, MassUnit::Kg), "\u{2212}1.3 kg");
        assert_eq!(change_text(1.0, MassUnit::Lb), "+2.2 lb");
    }
}
