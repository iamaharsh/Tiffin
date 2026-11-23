//! Trends over many days for the Home dashboard and Reports: each day's totals against the target in force that day,
//! the share of days that met the macro goals, the logging streak and the weight change. Reads only.

use crate::model::{civil_from_days, Nutrients};
use crate::tracker::diary::{self, Totals};
use crate::tracker::journal::{DiaryEntry, Journal};
use crate::tracker::{body, goals};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// A day met its macro goals when protein, net carbs and fat are each within this share of the goal (owner, 2026-11-21).
pub const HIT_BAND: f64 = 0.20;
/// The longest span one query reads, so a bad date can't make it walk centuries.
pub const MAX_DAYS: i32 = 3660;

/// The day's goals from the target in force that day (weekday template and floor included).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DayGoals {
    pub kcal: i32,
    pub protein_g: f64,
    pub net_carbs_g: f64,
    pub fat_g: f64,
}

/// One day: what was eaten and what it is judged against. `logged` is any food entry that day (owner: any day with
/// food logged counts, not only days marked complete).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DaySummary {
    pub day: String,
    /// Days since 1970-01-01.
    pub n: i32,
    pub logged: bool,
    pub totals: Totals,
    /// The day's full sum before display rounding, sugar and sodium included; a field is unknown when a food didn't report it.
    pub sum: Nutrients,
    pub goals: Option<DayGoals>,
    /// The weight the day is worked at (the body log's basis that day), when there is one.
    pub weight_kg: Option<f64>,
}

pub(crate) fn day_text(n: i32) -> String {
    let (y, m, d) = civil_from_days(n);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Every day from `from` to `to` (inclusive), oldest first, at most [`MAX_DAYS`]. Each month file is read once and the
/// weigh-ins once; a month that won't open is skipped (as the body log does). An unreadable profile is an error, so
/// the page can say the numbers are unavailable rather than show days that can't be judged.
pub fn day_summaries(j: &mut Journal, from: &str, to: &str) -> Result<Vec<DaySummary>, String> {
    let (f, t) = (diary::day_number(from)?, diary::day_number(to)?);
    if t < f {
        return Ok(vec![]);
    }
    if t - f >= MAX_DAYS {
        return Err("That range is too long.".into());
    }
    let targets = j.profile()?.targets.clone();
    let weights = body::live_weights(j, to);
    let months: BTreeSet<String> = (f..=t).map(|n| day_text(n)[..7].to_string()).collect();
    let mut by_day: BTreeMap<String, Vec<DiaryEntry>> = BTreeMap::new();
    for m in &months {
        let Ok(month) = j.month(m) else { continue };
        for e in month.entries.iter().filter(|e| e.deleted_at.is_none()) {
            by_day.entry(e.day.clone()).or_default().push(e.clone());
        }
    }
    let mut out = Vec::with_capacity((t - f + 1) as usize);
    for n in f..=t {
        let day = day_text(n);
        let entries: Vec<&DiaryEntry> = by_day.get(&day).map(|v| v.iter().collect()).unwrap_or_default();
        let weight = body::basis_from(&weights, n).map(|w| w.kg);
        let goals = goals::target_for_day(&targets, &day)
            .and_then(|set| goals::day_target(set, &day, weight).ok())
            .map(|dt| DayGoals { kcal: dt.kcal, protein_g: dt.macros.rows[0].grams, net_carbs_g: dt.macros.rows[1].grams, fat_g: dt.macros.rows[2].grams });
        let sum = entries.iter().fold(Nutrients::ZERO, |acc, e| acc.plus(&diary::entry_nutrients(e)));
        out.push(DaySummary { logged: !entries.is_empty(), totals: diary::totals(&entries), sum, goals, weight_kg: weight, n, day });
    }
    Ok(out)
}

/// Whether a day met its macro goals. `None` when the day can't be judged: nothing logged, no target that day, a
/// macro not reported for some food, or a goal of 0.
pub fn hit(s: &DaySummary) -> Option<bool> {
    if !s.logged {
        return None;
    }
    let g = s.goals.as_ref()?;
    let mut met = true;
    for (eaten, goal) in [(s.totals.protein, g.protein_g), (s.totals.net_carbs, g.net_carbs_g), (s.totals.fat, g.fat_g)] {
        let eaten = f64::from(eaten?);
        if !(goal > 0.0) {
            return None;
        }
        met &= (eaten - goal).abs() <= HIT_BAND * goal + 1e-9;
    }
    Some(met)
}

/// The share of judged days that met the macro goals over one range ending yesterday (today isn't finished).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct HitRate {
    pub id: &'static str,
    pub label: &'static str,
    pub hits: u32,
    /// Days that could be judged (see [`hit`]).
    pub judged: u32,
    pub pct: Option<u32>,
    /// "5 of 7 days" or "No days to judge yet".
    pub text: String,
}

/// The ranges Home offers: id, label and length in days (`None` is all time).
pub const RANGES: [(&str, &str, Option<i32>); 6] = [("week", "Week", Some(7)), ("month", "Month", Some(30)), ("3m", "3 months", Some(91)), ("6m", "6 months", Some(182)), ("year", "Year", Some(365)), ("all", "All time", None)];

/// The first day of the oldest month holding a live food entry, if any. Months only read (and so cached empty) by an
/// earlier query don't count.
pub(crate) fn first_day(j: &mut Journal) -> Option<i32> {
    for m in j.months_with_data() {
        if j.month(&m).is_ok_and(|month| month.entries.iter().any(|e| e.deleted_at.is_none())) {
            return diary::day_number(&format!("{m}-01")).ok();
        }
    }
    None
}

/// "All time" reaches back at most [`MAX_DAYS`].
pub fn hit_rates(j: &mut Journal, today: &str) -> Result<Vec<HitRate>, String> {
    let yesterday = diary::day_number(today)? - 1;
    let oldest = first_day(j).unwrap_or(yesterday + 1).max(yesterday - MAX_DAYS + 1);
    let longest = RANGES.iter().filter_map(|r| r.2).max().unwrap_or(1);
    let start = oldest.min(yesterday - longest + 1);
    let days = day_summaries(j, &day_text(start), &day_text(yesterday))?;
    Ok(RANGES
        .iter()
        .map(|&(id, label, len)| {
            let from = len.map_or(oldest, |l| yesterday - l + 1);
            let judged: Vec<bool> = days.iter().filter(|d| d.n >= from).filter_map(hit).collect();
            let hits = judged.iter().filter(|&&h| h).count() as u32;
            let n = judged.len() as u32;
            let pct = (n > 0).then(|| ((f64::from(hits) * 100.0 / f64::from(n)).round()) as u32);
            let text = if n == 0 { "No days to judge yet".to_string() } else { format!("{hits} of {n} day{}", if n == 1 { "" } else { "s" }) };
            HitRate { id, label, hits, judged: n, pct, text }
        })
        .collect())
}

/// Consecutive days with food logged, ending today, or ending yesterday while today has nothing yet. Counts at most
/// [`MAX_DAYS`]; a month that won't open is skipped.
pub fn streak(j: &mut Journal, today: &str) -> Result<u32, String> {
    let t = diary::day_number(today)?;
    let mut logged: BTreeSet<i32> = BTreeSet::new();
    for m in j.months_with_data() {
        let Ok(month) = j.month(&m) else { continue };
        for e in month.entries.iter().filter(|e| e.deleted_at.is_none()) {
            if let Ok(n) = diary::day_number(&e.day) {
                logged.insert(n);
            }
        }
    }
    let mut n = if logged.contains(&t) { t } else { t - 1 };
    let mut count = 0;
    while logged.contains(&n) && count < MAX_DAYS as u32 {
        count += 1;
        n -= 1;
    }
    Ok(count)
}

/// The weight trend over a range as change from its first point, in kg, and where the goal sits on the same scale.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct WeightChange {
    /// (day number, change in kg from the first trend point in the range)
    pub points: Vec<(i32, f64)>,
    pub start_kg: Option<f64>,
    /// The latest change, when there are at least two points.
    pub change_kg: Option<f64>,
    /// The goal weight minus the start.
    pub goal_change: Option<f64>,
}

/// The trend is worked over the whole history, so a range starting mid-way keeps the smoothing it had.
pub fn weight_change(j: &mut Journal, goal_kg: Option<f64>, from: &str, to: &str) -> Result<WeightChange, String> {
    let (f, t) = (diary::day_number(from)?, diary::day_number(to)?);
    let trend: Vec<(i32, f64)> = body::trend(&body::weight_points(j, to)).into_iter().filter(|p| p.0 >= f && p.0 <= t).collect();
    let start = trend.first().map(|p| p.1);
    let points: Vec<(i32, f64)> = start.map_or(vec![], |s| trend.iter().map(|&(n, kg)| (n, kg - s)).collect());
    let change_kg = (points.len() >= 2).then(|| points.last().map(|p| p.1)).flatten();
    Ok(WeightChange { goal_change: goal_kg.zip(start).map(|(g, s)| g - s), points, start_kg: start, change_kg })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MealType;
    use crate::tracker::journal::Journal;
    use std::fs;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-trends-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn eat(j: &mut Journal, day: &str, kcal: f32, p: Option<f32>, c: Option<f32>, f: Option<f32>) {
        diary::add(j, day, MealType::Lunch, diary::quick("Meal", Some(kcal), p, c, f).unwrap(), 1).unwrap();
    }

    /// Goals in whole grams (150 g protein, 200 g carbs, 60 g fat), so the ±20% edges fall on exact numbers.
    fn grams_target(j: &mut Journal, from: &str) {
        let macros = goals::MacroSpec { protein: goals::MacroMode::Grams(150.0), carbs: goals::MacroMode::Grams(200.0), fat: goals::MacroMode::Grams(60.0) };
        goals::put_target_in(j, goals::TargetSet { effective_from: from.into(), kcal: 2000, floor_kcal: 1200, macros, ..Default::default() });
    }

    fn on_target(j: &mut Journal, day: &str) {
        eat(j, day, 2000.0, Some(150.0), Some(200.0), Some(60.0));
    }

    #[test]
    fn summaries_cross_months_and_carry_unknown() {
        let d = tdir("summaries");
        let mut j = Journal::open(d.clone());
        eat(&mut j, "2026-10-31", 500.0, Some(30.0), Some(50.0), Some(10.0));
        eat(&mut j, "2026-11-01", 400.0, None, Some(40.0), Some(5.0));
        let s = day_summaries(&mut j, "2026-10-30", "2026-11-02").unwrap();
        assert_eq!(s.iter().map(|d| (d.day.as_str(), d.logged)).collect::<Vec<_>>(), [("2026-10-30", false), ("2026-10-31", true), ("2026-11-01", true), ("2026-11-02", false)]);
        assert_eq!((s[1].totals.calories, s[1].totals.protein, s[2].totals.protein), (Some(500.0), Some(30.0), None), "unknown protein stays unknown");
        assert!(s.iter().all(|d| d.goals.is_none()), "no target set");
        assert_eq!(day_summaries(&mut j, "2026-11-02", "2026-11-01").unwrap(), vec![]);
        assert!(day_summaries(&mut j, "2000-01-01", "2026-11-01").is_err());
        assert!(day_summaries(&mut j, "2026-13-01", "2026-11-01").is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_hit_needs_all_three_within_twenty_percent() {
        let d = tdir("hit");
        let mut j = Journal::open(d.clone());
        grams_target(&mut j, "2026-11-01");
        let g = day_summaries(&mut j, "2026-11-02", "2026-11-02").unwrap()[0].goals.clone().expect("a target is in force");
        assert_eq!((g.kcal, g.protein_g, g.net_carbs_g, g.fat_g), (2000, 150.0, 200.0, 60.0));
        // exactly at +20% and -20% still meets; just past it misses
        eat(&mut j, "2026-11-02", 2000.0, Some(180.0), Some(160.0), Some(72.0));
        eat(&mut j, "2026-11-03", 2000.0, Some(180.5), Some(200.0), Some(60.0));
        eat(&mut j, "2026-11-04", 2000.0, Some(150.0), Some(200.0), Some(47.5));
        eat(&mut j, "2026-11-05", 2000.0, Some(150.0), None, Some(60.0));
        let s = day_summaries(&mut j, "2026-11-02", "2026-11-06").unwrap();
        assert_eq!(s.iter().map(hit).collect::<Vec<_>>(), [Some(true), Some(false), Some(false), None, None], "unknown carbs and an empty day can't be judged");
        // before any target, a logged day can't be judged either
        on_target(&mut j, "2026-10-20");
        assert_eq!(hit(&day_summaries(&mut j, "2026-10-20", "2026-10-20").unwrap()[0]), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hit_rates_end_yesterday_and_count_judged_days() {
        let d = tdir("rates");
        let mut j = Journal::open(d.clone());
        grams_target(&mut j, "2025-01-01");
        // today is Nov 20, so every range ends Nov 19: the week starts Nov 13, the month Oct 21
        for day in ["2026-11-13", "2026-11-19", "2026-11-12", "2026-10-21", "2026-10-20", "2025-09-01", "2026-11-20"] {
            on_target(&mut j, day);
        }
        eat(&mut j, "2026-11-16", 900.0, Some(75.0), Some(200.0), Some(60.0)); // a miss inside the week
        let r = hit_rates(&mut j, "2026-11-20").unwrap();
        let by = |id: &str| r.iter().find(|x| x.id == id).unwrap().clone();
        assert_eq!((by("week").hits, by("week").judged, by("week").pct, by("week").text.as_str()), (2, 3, Some(67), "2 of 3 days"), "Nov 13 in, Nov 12 out, today never");
        assert_eq!((by("month").hits, by("month").judged), (4, 5), "Oct 21 in, Oct 20 out");
        assert_eq!((by("3m").judged, by("6m").judged, by("year").judged), (6, 6, 6));
        assert_eq!((by("all").hits, by("all").judged), (6, 7), "all time reaches the first month with food, 444 days back");
        assert_eq!(r.iter().map(|x| x.id).collect::<Vec<_>>(), ["week", "month", "3m", "6m", "year", "all"]);
        // asking again gives the same answer: months read by the first call don't move "all time"
        assert_eq!(hit_rates(&mut j, "2026-11-20").unwrap(), r);
        let empty = tdir("rates-empty");
        let mut e = Journal::open(empty.clone());
        assert!(hit_rates(&mut e, "2026-11-20").unwrap().iter().all(|x| x.judged == 0 && x.pct.is_none() && x.text == "No days to judge yet"));
        for x in [&d, &empty] {
            let _ = fs::remove_dir_all(x);
        }
    }

    #[test]
    fn goals_match_the_diary_budget_at_each_days_weight() {
        let d = tdir("perkg");
        let mut j = Journal::open(d.clone());
        let macros = goals::MacroSpec { protein: goals::MacroMode::PerKg(2.0), carbs: goals::MacroMode::Rest, fat: goals::MacroMode::Percent(30.0) };
        goals::put_target_in(&mut j, goals::TargetSet { effective_from: "2026-11-01".into(), kcal: 2200, floor_kcal: 1200, macros, ..Default::default() });
        body::set_value(&mut j, "2026-11-01", "2026-11-30", "weight", 80.0, 1).unwrap();
        body::set_value(&mut j, "2026-11-10", "2026-11-30", "weight", 70.0, 1).unwrap();
        let s = day_summaries(&mut j, "2026-11-05", "2026-11-12").unwrap();
        for day in [&s[0], &s[7]] {
            let budget = diary::day_view(&mut j, &day.day, "2026-11-30").unwrap().budget.expect("a budget");
            let g = day.goals.clone().unwrap();
            assert_eq!((g.kcal, g.protein_g.round() as i32, g.net_carbs_g.round() as i32, g.fat_g.round() as i32), (budget.target, budget.protein.goal, budget.carbs.goal, budget.fat.goal), "{}", day.day);
        }
        assert_eq!((s[0].goals.clone().unwrap().protein_g, s[7].goals.clone().unwrap().protein_g), (160.0, 140.0), "the weigh-in on Nov 10 moves later days only");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn streak_counts_back_from_today_or_yesterday() {
        let d = tdir("streak");
        let mut j = Journal::open(d.clone());
        assert_eq!(streak(&mut j, "2026-11-20").unwrap(), 0);
        for day in ["2026-10-30", "2026-10-31", "2026-11-01", "2026-11-02"] {
            eat(&mut j, day, 100.0, None, None, None);
        }
        assert_eq!(streak(&mut j, "2026-11-03").unwrap(), 4, "today not logged yet: the run ending yesterday, across a month end");
        assert_eq!(streak(&mut j, "2026-11-02").unwrap(), 4, "today logged");
        assert_eq!(streak(&mut j, "2026-11-04").unwrap(), 0, "a missed day ends it");
        // a deleted entry doesn't count
        let uid = j.month("2026-11").unwrap().entries.iter().find(|e| e.day == "2026-11-02").unwrap().uid.clone();
        diary::delete(&mut j, "2026-11-02", &uid, 2).unwrap();
        assert_eq!(streak(&mut j, "2026-11-03").unwrap(), 0);
        assert_eq!(streak(&mut j, "2026-11-02").unwrap(), 3);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn weight_change_is_from_the_first_trend_point_in_range() {
        let d = tdir("weight");
        let mut j = Journal::open(d.clone());
        let none = weight_change(&mut j, Some(70.0), "2026-11-01", "2026-11-20").unwrap();
        assert_eq!((none.points.len(), none.start_kg, none.change_kg, none.goal_change), (0, None, None, None));
        for (i, kg) in [80.0, 80.0, 79.0, 79.0, 78.0].iter().enumerate() {
            body::set_value(&mut j, &format!("2026-11-{:02}", i + 10), "2026-11-20", "weight", *kg, 1).unwrap();
        }
        let w = weight_change(&mut j, Some(72.0), "2026-11-11", "2026-11-20").unwrap();
        let all = body::trend(&body::weight_points(&mut j, "2026-11-20"));
        let start = all[1].1; // Nov 11's trend value, smoothed from Nov 10
        assert_eq!(w.start_kg, Some(start));
        assert_eq!(w.points.len(), 4);
        assert!(w.points[0].1.abs() < 1e-9);
        assert!((w.change_kg.unwrap() - (all[4].1 - start)).abs() < 1e-9 && w.change_kg.unwrap() < 0.0);
        assert!((w.goal_change.unwrap() - (72.0 - start)).abs() < 1e-9);
        let one = weight_change(&mut j, None, "2026-11-14", "2026-11-20").unwrap();
        assert_eq!((one.points.len(), one.change_kg, one.goal_change), (1, None, None), "one point: no change yet");
        let _ = fs::remove_dir_all(&d);
    }
}
