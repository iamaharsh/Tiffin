//! Importing recipes (pasted text, parsed HTML, line checks, review/tidy, saving reviewed
//! recipes, site rules). Shared by the desktop window and the web version; commands that fetch
//! over the network, read the desktop inbox folder, or read a sync folder stay in the desktop shell.

use crate::recipes::touched;
use crate::{mutate, Shared, GONE};
use recipe_box::importer::{self, ImportResult};
use recipe_box::ingredients::{check_step, parse_ingredient};
use recipe_box::model::{Library, Recipe};
use recipe_box::{nutrition, rules, storage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Dup {
    id: u64,
    name: String,
    score: f32,
    reason: String,
}

/// A recipe waiting for review. The page edits it and sends it back to save.
#[derive(Serialize, Deserialize)]
pub struct Review {
    pub recipe: Recipe,
    ing_lines: Vec<String>,
    step_lines: Vec<String>,
    method: String,
    tried: Vec<(String, String)>,
    field_issues: Vec<String>,
    pub duplicate: Option<Dup>,
    /// with a duplicate: merge into it (true) or keep both
    merge: bool,
    changes: Vec<String>,
    rule_domain: Option<String>,
    rule_worked: Option<bool>,
    /// reviewing a recipe that's already in the library
    existing_id: Option<u64>,
    origin: String,
    image_url: String,
}

pub fn dup_of(lib: &Library, r: &Recipe) -> Option<Dup> {
    importer::find_duplicate(lib, r, None).map(|d| Dup { id: d.id, name: d.name, score: d.score, reason: d.reason })
}

pub fn review_of(res: ImportResult, lib: &Library, origin: &str) -> Review {
    Review {
        ing_lines: res.recipe.ingredient_lines().iter().map(|s| s.to_string()).collect(),
        step_lines: res.recipe.steps(),
        duplicate: dup_of(lib, &res.recipe),
        recipe: res.recipe,
        method: res.method,
        tried: res.tried,
        field_issues: res.field_issues,
        merge: true,
        changes: vec![],
        rule_domain: res.rule_domain,
        rule_worked: res.rule_worked,
        existing_id: None,
        origin: origin.into(),
        image_url: res.image_url,
    }
}

impl Review {
    fn sync_text(&mut self) {
        let join = |v: &[String]| v.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n");
        self.recipe.ingredients = join(&self.ing_lines);
        self.recipe.directions = join(&self.step_lines);
    }
}

/// Remember whether a site's import rule still works (the classic note_rule_health). Saving it
/// is a nicety, so a refusal (damaged or read-only library) is ignored.
fn note_rule_health(state: &Shared, res: &ImportResult) {
    let (Some(d), Some(w)) = (&res.rule_domain, res.rule_worked) else { return };
    let other_found = res.tried.iter().any(|(m, o)| !m.starts_with("Site rule") && !o.contains("none") && !o.contains("no "));
    let _ = mutate(state, |st| {
        let h = match st.lib.rule_health.iter_mut().position(|h| &h.domain == d) {
            Some(i) => &mut st.lib.rule_health[i],
            None => {
                st.lib.rule_health.push(recipe_box::model::RuleHealth { domain: d.clone(), ..Default::default() });
                st.lib.rule_health.last_mut().unwrap()
            }
        };
        h.last_checked = recipe_box::model::now_secs();
        h.needs_update = !w && other_found;
        h.last_result = if w { "Worked on the last import".into() } else { "Found nothing on the last import".into() };
        Ok(())
    });
}

pub fn from_html(state: &Shared, html: &str, url: &str, origin: &str) -> Review {
    let (res, rv) = {
        let s = state.lock().unwrap();
        let res = importer::import_html(html, url, &s.lib);
        let rv = review_of(res.clone(), &s.lib, origin);
        (res, rv)
    };
    note_rule_health(state, &res);
    rv
}

/// "allrecipes.com/recipe/…" is taken as https://; anything that isn't a web address is refused.
pub fn web_url(u: &str) -> Result<String, String> {
    // The shared check gives the specific refusal (file://, a leading dash, a user name in the link).
    let u = importer::check_web_url(u)?;
    if crate::web_address(&u) && u.split("://").nth(1).is_some_and(|h| h.contains('.')) {
        Ok(u)
    } else {
        Err("That doesn't look like a web address. Paste the recipe page's address, starting with https://".into())
    }
}

pub fn import_text(state: &Shared, text: String) -> Result<Review, String> {
    let s = state.lock().unwrap();
    Ok(review_of(importer::import_text(&text, &s.lib), &s.lib, "Pasted text"))
}

#[derive(Serialize)]
pub struct LineCheck {
    ok: bool,
    /// confidence under 0.5: shown red rather than amber
    low: bool,
    issues: Vec<String>,
    /// how an ingredient line was split: amount · unit · ingredient · preparation
    parsed: String,
}

#[derive(Serialize)]
pub struct Checks {
    ing: Vec<LineCheck>,
    steps: Vec<LineCheck>,
}

pub fn check_lines(ing: Vec<String>, steps: Vec<String>) -> Result<Checks, String> {
    let dash = |s: String, alt: &str| if s.is_empty() { alt.to_string() } else { s };
    let ing = ing
        .iter()
        .map(|l| {
            let p = parse_ingredient(l);
            let parsed = if p.is_heading {
                "section heading".into()
            } else {
                format!(
                    "{} · {} · {}{}",
                    dash(p.qty_text(), "—"),
                    dash(p.unit_text(), "—"),
                    dash(p.name.clone(), "?"),
                    if p.prep.is_empty() { String::new() } else { format!(" · {}", p.prep) }
                )
            };
            LineCheck { ok: !p.flagged(), low: p.confidence < 0.5, issues: p.issues, parsed }
        })
        .collect();
    let steps = steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let r = check_step(s, if i > 0 { Some(steps[i - 1].as_str()) } else { None });
            LineCheck { ok: !r.flagged(), low: r.confidence < 0.5, issues: r.issues, parsed: String::new() }
        })
        .collect();
    Ok(Checks { ing, steps })
}

/// "Fix all formatting": the classic one-pass cleanup, shown as a list of what changed.
pub fn review_tidy(state: &Shared, mut review: Review) -> Result<Review, String> {
    let short = state.lock().unwrap().lib.settings.unit_style_short;
    review.sync_text();
    review.changes = importer::tidy_recipe(&mut review.recipe, short);
    review.ing_lines = review.recipe.ingredient_lines().iter().map(|s| s.to_string()).collect();
    review.step_lines = review.recipe.steps();
    Ok(review)
}

/// Review & Tidy a recipe already in the library (opens on the lines that need a look).
pub fn review_existing(state: &Shared, id: u64) -> Result<Review, String> {
    let s = state.lock().unwrap();
    let r = s.lib.recipe(id).ok_or(GONE)?;
    Ok(Review {
        ing_lines: r.ingredient_lines().iter().map(|s| s.to_string()).collect(),
        step_lines: r.steps(),
        recipe: r.clone(),
        method: "Recipe in your library".into(),
        tried: vec![],
        field_issues: vec![],
        duplicate: None,
        merge: true,
        changes: vec![],
        rule_domain: None,
        rule_worked: None,
        existing_id: Some(id),
        origin: "Review".into(),
        image_url: String::new(),
    })
}

#[derive(Serialize)]
pub struct Saved {
    id: u64,
    message: String,
}

/// A recipe page's picture that [`save_review_in`] reserved a file name for; the caller downloads
/// `url` and writes it to `dir/file` after the save succeeds (a background thread on desktop, the
/// helper `/photo` endpoint on the web).
pub struct PendingPhoto {
    pub url: String,
    pub dir: PathBuf,
    pub file: String,
}

pub fn save_review_in(state: &Shared, mut review: Review) -> Result<(Saved, Option<PendingPhoto>), String> {
    review.sync_text();
    mutate(state, |st| {
        let mut r = review.recipe.clone();
        r.review_flags = importer::review_summary(&r);
        if r.name.trim().is_empty() {
            r.name = "Untitled Recipe".into();
        }
        if !r.nutrition_edited {
            r.nutrition = Some(nutrition::calculate(&r).total);
        }
        // The page's own picture becomes the photo; the caller downloads it after the save succeeds.
        let mut photo = None;
        if r.photo.is_empty() && crate::web_address(&review.image_url) {
            let name = storage::photo_name(&r.name.to_lowercase().replace(' ', "-"));
            photo = Some(PendingPhoto { url: review.image_url.clone(), dir: storage::photos_dir(&st.store.dir), file: name.clone() });
            r.photo = name;
        }
        st.store.snapshot(&st.lib, "before import")?;
        let set = st.lib.settings.clone();
        if let Some(id) = review.existing_id {
            let e = st.lib.recipe_mut(id).ok_or(GONE)?;
            *e = r;
            touched(e, &set);
            return Ok((Saved { id, message: "Saved".into() }, photo));
        }
        if let (Some(d), true) = (&review.duplicate, review.merge) {
            if let Some(e) = st.lib.recipe_mut(d.id) {
                let changes = importer::merge_into(e, &r);
                if e.review_flags.is_empty() {
                    e.review_flags = importer::review_summary(e);
                }
                touched(e, &set);
                let what = if changes.is_empty() { "nothing new to add".to_string() } else { changes.join("; ") };
                return Ok((Saved { id: d.id, message: format!("Merged into “{}”: {}", d.name, what) }, photo));
            }
        }
        touched(&mut r, &set);
        Ok((Saved { id: st.lib.add_recipe(r), message: "Saved to your library".into() }, photo))
    })
}

#[derive(Serialize)]
pub struct RuleRow {
    domain: String,
    name: String,
    version: u32,
    updated: bool,
    verified: String,
    last_import: Option<String>,
    needs_update: bool,
    /// (passed, detail) when the tests were run
    test: Option<(bool, String)>,
}

#[derive(Serialize)]
pub struct RulesOverview {
    rules: Vec<RuleRow>,
    updated_any: bool,
    inbox: String,
}

pub fn rules_overview_in(state: &Shared, test: bool, inbox: String) -> RulesOverview {
    let s = state.lock().unwrap();
    let lib = &s.lib;
    let mut all: Vec<rules::SiteRule> = rules::builtin_rules()
        .into_iter()
        .map(|b| lib.rule_overrides.iter().find(|o| o.domain == b.domain).cloned().unwrap_or(b))
        .collect();
    for o in &lib.rule_overrides {
        if !all.iter().any(|a| a.domain == o.domain) {
            all.push(o.clone());
        }
    }
    let tests = if test { rules::test_all(&lib.rule_overrides) } else { vec![] };
    let rules = all
        .into_iter()
        .map(|r| {
            let h = lib.rule_health.iter().find(|h| h.domain == r.domain);
            RuleRow {
                updated: lib.rule_overrides.iter().any(|o| o.domain == r.domain),
                last_import: h.map(|h| h.last_result.clone()),
                needs_update: h.is_some_and(|h| h.needs_update),
                test: tests.iter().find(|t| t.domain == r.domain).map(|t| (t.passed, t.detail.clone())),
                domain: r.domain,
                name: r.name,
                version: r.version,
                verified: r.verified,
            }
        })
        .collect();
    RulesOverview { rules, updated_any: !lib.rule_overrides.is_empty(), inbox }
}

/// A rules update file's text. Only rules that pass their own test are accepted.
pub fn load_rules(state: &Shared, text: String) -> Result<String, String> {
    let (ok, rejected) = rules::load_update(&text)?;
    let n = ok.len();
    mutate(state, |st| {
        for r in ok {
            st.lib.rule_overrides.retain(|o| o.domain != r.domain);
            st.lib.rule_health.retain(|h| h.domain != r.domain);
            st.lib.rule_overrides.push(r);
        }
        Ok(())
    })?;
    Ok(if rejected.is_empty() { format!("Updated {n} rule(s)") } else { format!("Updated {n} rule(s); rejected: {}", rejected.join("; ")) })
}

pub fn use_builtin_rules(state: &Shared) -> Result<(), String> {
    mutate(state, |st| {
        st.lib.rule_overrides.clear();
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use crate::{dispatch, foods::OffState, load_state, Services};
    use serde_json::json;
    use std::sync::Mutex;

    #[test]
    fn addresses_get_a_scheme_and_others_are_refused() {
        assert_eq!(super::web_url("www.allrecipes.com/recipe/1").unwrap(), "https://www.allrecipes.com/recipe/1");
        assert_eq!(super::web_url(" https://a.com/x ").unwrap(), "https://a.com/x");
        for bad in ["", "pancakes", "file:///C:/x.html", "javascript:alert(1)", "https://a b.com"] {
            assert!(super::web_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn importing_commands_dispatch_with_json() {
        let dir = crate::scratch_dir().join(format!("tiffin-commands-importing-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = Mutex::new(load_state(dir.clone()));
        let off = OffState::new(&dir);
        let app = Services { state, off };

        let rv = dispatch(&app, "import_text", json!({ "text": "Toast\n\nIngredients\n- 2   tablespoons  butter\n\nDirections\n1. Spread butter on toast for 2 minutes." })).unwrap();
        assert_eq!(rv["origin"], "Pasted text");
        assert_eq!(rv["recipe"]["name"], "Toast");

        let checked = dispatch(&app, "check_lines", json!({ "ing": ["2 cups flour"], "steps": ["Bake for 20 minutes."] })).unwrap();
        assert_eq!(checked["ing"][0]["ok"], true);
        assert_eq!(checked["ing"][0]["low"], false);
        assert_eq!(checked["ing"][0]["issues"], json!([]));
        assert!(checked["ing"][0]["parsed"].as_str().unwrap().contains("flour"));
        assert_eq!(checked["steps"][0]["ok"], true);
        assert_eq!(checked["steps"][0]["low"], false);
        assert_eq!(checked["steps"][0]["issues"], json!([]));
        assert_eq!(checked["steps"][0]["parsed"], "");

        let tidied = dispatch(&app, "review_tidy", json!({ "review": rv })).unwrap();
        assert_eq!(tidied["recipe"]["name"], "Toast");
        assert!(!tidied["changes"].as_array().unwrap().is_empty(), "tidy should clean bullet/spacing/units");
        assert!(!tidied["ing_lines"][0].as_str().unwrap().starts_with('-'));

        let first_id = app.state.lock().unwrap().lib.recipes[0].id;
        let existing = dispatch(&app, "review_existing", json!({ "id": first_id })).unwrap();
        assert_eq!(existing["existing_id"], first_id);
        assert_eq!(dispatch(&app, "review_existing", json!({ "id": 999999 })).unwrap_err(), crate::GONE);

        let err = dispatch(&app, "load_rules", json!({ "text": "not json" })).unwrap_err();
        assert!(err.starts_with("Not a rules file:"), "{err}");

        let one = recipe_box::rules::builtin_rules().into_iter().next().unwrap();
        let domain = one.domain.clone();
        app.state.lock().unwrap().lib.rule_health.push(recipe_box::model::RuleHealth { domain: domain.clone(), needs_update: true, ..Default::default() });
        let update_json = serde_json::to_string(&vec![one]).unwrap();
        assert_eq!(dispatch(&app, "load_rules", json!({ "text": update_json })).unwrap(), "Updated 1 rule(s)");
        {
            let st = app.state.lock().unwrap();
            assert_eq!(st.lib.rule_overrides.len(), 1);
            assert_eq!(st.lib.rule_overrides[0].domain, domain);
            assert!(st.lib.rule_health.iter().all(|h| h.domain != domain), "load_rules clears stale rule_health");
        }

        // api.ts calls use_builtin_rules with no arguments (null payload)
        dispatch(&app, "use_builtin_rules", serde_json::Value::Null).unwrap();
        assert!(app.state.lock().unwrap().lib.rule_overrides.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_review_and_rules_overview_work_portably() {
        let dir = crate::scratch_dir().join(format!("tiffin-commands-import-cores-{}", crate::test_pid()));
        let _ = std::fs::remove_dir_all(&dir);
        let state = Mutex::new(load_state(dir.clone()));

        // new recipe with blank name gets Untitled fallback and reserves a pending photo for a web image_url
        let mut rv = super::import_text(&state, "   \n\nIngredients\n1 cup oats\n\nDirections\n1. Cook.".into()).unwrap();
        rv.recipe.name = "   ".into();
        rv.image_url = "https://example.com/oats.jpg".into();
        let (saved, photo) = super::save_review_in(&state, rv).unwrap();
        assert_eq!(saved.message, "Saved to your library");
        let p = photo.expect("web image_url produces a PendingPhoto");
        assert_eq!(p.url, "https://example.com/oats.jpg");
        assert_eq!(p.dir, recipe_box::storage::photos_dir(&dir));
        assert!(p.file.ends_with(".jpg"));
        {
            let st = state.lock().unwrap();
            let r = st.lib.recipe(saved.id).unwrap();
            assert_eq!(r.name, "Untitled Recipe");
            assert_eq!(r.photo, p.file);
        }

        // editing an existing recipe updates it in place; missing existing_id returns GONE
        let mut edit = super::review_existing(&state, saved.id).unwrap();
        edit.recipe.name = "Warm Oats".into();
        let (saved2, photo2) = super::save_review_in(&state, edit).unwrap();
        assert_eq!((saved2.id, saved2.message.as_str()), (saved.id, "Saved"));
        assert!(photo2.is_none());
        assert_eq!(state.lock().unwrap().lib.recipe(saved.id).unwrap().name, "Warm Oats");

        let mut gone = super::review_existing(&state, saved.id).unwrap();
        gone.existing_id = Some(999_999);
        assert_eq!(super::save_review_in(&state, gone).err().as_deref(), Some(crate::GONE));

        // duplicate merge merges into the existing recipe instead of adding a copy
        let count_before = state.lock().unwrap().lib.recipes.len();
        let mut dup = super::import_text(&state, "Warm Oats\n\nIngredients\n1 cup oats\n1 pinch salt\n\nDirections\n1. Cook.".into()).unwrap();
        assert!(dup.duplicate.is_some());
        dup.merge = true;
        let (merged, _) = super::save_review_in(&state, dup).unwrap();
        assert_eq!(merged.id, saved.id);
        assert!(merged.message.starts_with("Merged into “Warm Oats”:"), "{}", merged.message);
        assert_eq!(state.lock().unwrap().lib.recipes.len(), count_before);

        // rules_overview_in without tests, then with tests and an override
        let ov = serde_json::to_value(super::rules_overview_in(&state, false, String::new())).unwrap();
        assert_eq!(ov["updated_any"], false);
        assert_eq!(ov["inbox"], "");
        assert!(!ov["rules"].as_array().unwrap().is_empty());
        assert!(ov["rules"][0]["test"].is_null());

        let one = recipe_box::rules::builtin_rules().into_iter().next().unwrap();
        let domain = one.domain.clone();
        super::load_rules(&state, serde_json::to_string(&vec![one]).unwrap()).unwrap();
        let ov_tested = serde_json::to_value(super::rules_overview_in(&state, true, "C:/Inbox".into())).unwrap();
        assert_eq!(ov_tested["updated_any"], true);
        assert_eq!(ov_tested["inbox"], "C:/Inbox");
        let row = ov_tested["rules"].as_array().unwrap().iter().find(|r| r["domain"] == domain).unwrap();
        assert_eq!(row["updated"], true);
        assert_eq!(row["test"][0], true);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
