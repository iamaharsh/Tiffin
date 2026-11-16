//! Release regression checks: timers, step-by-step mode, video playback links, import rules,
//! the ingredient parser and data safety. Run from Help > Run self-tests, with `--self-test`
//! on the command line (exit code 1 on failure, for release scripts), and by `cargo test`.

use crate::cooking::{video_kind, video_play_url, StepMode, Timer, VideoKind};
use crate::ingredients::{find_times, parse_ingredient, split_steps};
use crate::model::{Library, Recipe};

#[derive(Clone, Debug)]
pub struct TestResult {
    pub group: &'static str,
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

fn t(group: &'static str, name: &str, passed: bool, detail: impl Into<String>) -> TestResult {
    TestResult { group, name: name.into(), passed, detail: detail.into() }
}

pub fn run_all(lib: &Library) -> Vec<TestResult> {
    let mut out = vec![];

    // ---- timers
    let mut tm = Timer::new("Pasta", 90);
    tm.tick(30.0);
    out.push(t("Timers", "Counts down", (tm.remaining - 60.0).abs() < 1e-6, tm.display()));
    tm.toggle();
    tm.tick(30.0);
    out.push(t("Timers", "Pause holds the time", (tm.remaining - 60.0).abs() < 1e-6, tm.display()));
    tm.toggle();
    let finished = tm.tick(61.0);
    out.push(t("Timers", "Finishes once and stops at 0:00", finished && tm.done && tm.display() == "0:00" && !tm.tick(1.0), tm.display()));
    tm.reset();
    out.push(t("Timers", "Reset restores the time", tm.display() == "1:30" && !tm.done, tm.display()));
    out.push(t("Timers", "New timer starts at 5:00", crate::cooking::timer_parts(crate::cooking::NEW_TIMER_SECS) == (0, 5, 0), ""));
    out.push(t("Timers", "Hours, minutes and seconds add up", crate::cooking::timer_seconds(1, 15, 30) == 4530, ""));
    let d: Vec<f64> = (0..6).map(crate::storage::sync_retry_delay).collect();
    out.push(t("Data safety", "Sync retries back off 5, 20, 45… up to 600 s", d == vec![5.0, 20.0, 45.0, 80.0, 125.0, 180.0] && crate::storage::sync_retry_delay(20) == 600.0, format!("{:?}", d)));
    let long = Timer::new("Roast", 4500);
    out.push(t("Timers", "Shows hours", long.display() == "1:15:00", long.display()));
    let spans = find_times("Simmer for 25 minutes, then rest 1 hour and 15 minutes or 30 seconds.");
    let secs: Vec<u32> = spans.iter().map(|s| s.seconds).collect();
    out.push(t("Timers", "Finds times in directions", secs == vec![1500, 4500, 30], format!("{:?}", secs)));

    // ---- step-by-step mode
    let steps = split_steps("1. Heat the oven. 2. Mix for 2 minutes. 3. Bake for 30 minutes.");
    let mut sm = StepMode::new(1, steps.clone());
    out.push(t("Step-by-step", "Splits numbered steps", steps.len() == 3, format!("{} steps", steps.len())));
    sm.prev();
    out.push(t("Step-by-step", "Can't go before the first step", sm.index == 0, ""));
    sm.next();
    sm.next();
    sm.next();
    out.push(t("Step-by-step", "Stops at the last step", sm.index == 2, format!("step {}", sm.index + 1)));
    out.push(t("Step-by-step", "Offers the step's timer", sm.timers_in_step().first().map(|s| s.seconds) == Some(1800), sm.current()));
    let empty = StepMode::new(1, vec![]);
    out.push(t("Step-by-step", "Copes with a recipe without steps", empty.current().is_empty() && empty.progress() == 0.0, ""));

    // ---- video playback
    out.push(t("Video", "YouTube link", video_kind("https://www.youtube.com/watch?v=abc123&list=x") == Some(VideoKind::YouTube("abc123".into())), ""));
    out.push(t("Video", "Short YouTube link", video_kind("https://youtu.be/abc123?si=1") == Some(VideoKind::YouTube("abc123".into())), ""));
    out.push(t("Video", "Vimeo link", video_kind("https://vimeo.com/123456") == Some(VideoKind::Vimeo("123456".into())), ""));
    out.push(t("Video", "Video file", video_kind("https://cdn.example.com/v/clip.mp4?x=1") == Some(VideoKind::File), ""));
    out.push(t("Video", "Starts at a step's time", video_play_url("https://youtu.be/abc", 95).as_deref() == Some("https://www.youtube.com/watch?v=abc&t=95s"), ""));
    out.push(t("Video", "Rejects non-web links", video_kind("C:\\video.mp4").is_none(), ""));

    // ---- parser
    let cases: [(&str, Option<f64>, Option<&str>, &str); 6] = [
        ("1 1/2 cups all-purpose flour, sifted", Some(1.5), Some("cup"), "all-purpose flour"),
        ("½ tsp salt", Some(0.5), Some("teaspoon"), "salt"),
        ("2-3 cloves garlic, minced", Some(2.0), Some("clove"), "garlic"),
        ("200g butter", Some(200.0), Some("gram"), "butter"),
        ("1 can (14 oz) diced tomatoes", Some(1.0), Some("can"), "diced tomatoes"),
        ("3 large eggs", Some(3.0), None, "large eggs"),
    ];
    for (line, q, u, name) in cases {
        let p = parse_ingredient(line);
        let ok = p.quantity == q && p.unit.as_deref() == u && p.name == name && !p.flagged();
        out.push(t("Importer", &format!("Reads \"{}\"", line), ok, format!("{} | {} | {}", p.qty_text(), p.unit_text(), p.name)));
    }
    let bad = parse_ingredient("Advertisement - Jump to Recipe");
    out.push(t("Importer", "Flags page clutter for review", bad.flagged(), bad.issues.join("; ")));

    // ---- site rules
    for rt in crate::rules::test_all(&lib.rule_overrides) {
        out.push(t("Site rules", &rt.domain, rt.passed, rt.detail));
    }

    // ---- trying amounts: the recipe stays the same, nutrition follows the trial
    let base = Recipe { ingredients: "1 cup flour\n2 tbsp butter".into(), servings: "2".into(), ..Default::default() };
    let mut trial = crate::cooking::Trial { recipe_id: 0, amounts: Default::default() };
    trial.amounts.insert(1, 4.0);
    let tried = trial.apply(&base);
    let (before, after) = (crate::nutrition::calculate(&base).total.calories.unwrap_or(f32::NAN), crate::nutrition::calculate(&tried).total.calories.unwrap_or(f32::NAN));
    out.push(t("Nutrition", "Trying more butter raises calories without changing the recipe", after > before + 150.0 && base.ingredients.contains("2 tbsp butter") && tried.ingredients.contains("4 tbsp butter"), format!("{:.0} → {:.0} kcal", before, after)));

    // ---- search (any word, any field)
    let r = Recipe { name: "Garlic Shrimp".into(), ingredients: "1 lb shrimp".into(), notes: "spicy".into(), ..Default::default() };
    let m = |q: &str| crate::model::recipe_matches(&r, &crate::model::search_words(q));
    out.push(t("Search", "Any word in any field matches", m("SPICY pasta") && m("garlic") && !m("chocolate") && m(""), ""));

    // ---- data safety
    let mut a = Library::default();
    let id = a.add_recipe(Recipe { name: "Soup".into(), uid: "x".into(), modified: 1, ..Default::default() });
    let mut b = a.clone();
    b.recipe_mut(id).unwrap().favorite = true;
    b.recipe_mut(id).unwrap().modified = 2;
    crate::storage::merge_libraries(&mut a, &b);
    out.push(t("Data safety", "Favorites survive sync", a.recipe(id).map(|r| r.favorite).unwrap_or(false), ""));
    let text = serde_json::to_string(lib).unwrap_or_default();
    let back: Result<Library, _> = serde_json::from_str(&text);
    out.push(t("Data safety", "Library saves and loads without loss", back.map(|l| l.recipes.len() == lib.recipes.len()).unwrap_or(false), format!("{} recipes", lib.recipes.len())));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_self_tests_pass() {
        let lib = crate::model::sample_library(20000);
        for r in super::run_all(&lib) {
            assert!(r.passed, "{} / {}: {}", r.group, r.name, r.detail);
        }
    }
}
