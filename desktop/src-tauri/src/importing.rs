//! Importing recipes (web address, pasted text, the inbox folder) and the review step before
//! they're saved. Same rules as the classic app's Browser and Review pages.

use crate::{mutate, touched, Shared, GONE};
use recipe_box::importer::{self, ImportResult};
use recipe_box::ingredients::{check_step, parse_ingredient};
use recipe_box::inbox::{self, InboxItem};
use recipe_box::model::{Library, Recipe};
use recipe_box::{nutrition, rules, storage};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct Dup {
    id: u64,
    name: String,
    score: f32,
    reason: String,
}

/// A recipe waiting for review. The page edits it and sends it back to save.
#[derive(Serialize, Deserialize)]
pub(crate) struct Review {
    recipe: Recipe,
    ing_lines: Vec<String>,
    step_lines: Vec<String>,
    method: String,
    tried: Vec<(String, String)>,
    field_issues: Vec<String>,
    duplicate: Option<Dup>,
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

fn dup_of(lib: &Library, r: &Recipe) -> Option<Dup> {
    importer::find_duplicate(lib, r, None).map(|d| Dup { id: d.id, name: d.name, score: d.score, reason: d.reason })
}

fn review_of(res: ImportResult, lib: &Library, origin: &str) -> Review {
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

fn from_html(state: &Shared, html: &str, url: &str, origin: &str) -> Review {
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
fn web_url(u: &str) -> Result<String, String> {
    // The shared check gives the specific refusal (file://, a leading dash, a user name in the link).
    let u = importer::check_web_url(u)?;
    if crate::web_address(&u) && u.split("://").nth(1).is_some_and(|h| h.contains('.')) {
        Ok(u)
    } else {
        Err("That doesn't look like a web address. Paste the recipe page's address, starting with https://".into())
    }
}

async fn fetch(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || importer::fetch(&url)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn import_url(state: tauri::State<'_, Shared>, url: String) -> Result<Review, String> {
    let url = web_url(&url)?;
    let html = fetch(url.clone()).await.map_err(|e| format!("Couldn't read {url}: {e}"))?;
    Ok(from_html(&state, &html, &url, "Web page"))
}

#[tauri::command]
pub(crate) fn import_text(state: tauri::State<Shared>, text: String) -> Review {
    let s = state.lock().unwrap();
    review_of(importer::import_text(&text, &s.lib), &s.lib, "Pasted text")
}

#[derive(Serialize)]
pub(crate) struct LineCheck {
    ok: bool,
    /// confidence under 0.5: shown red rather than amber
    low: bool,
    issues: Vec<String>,
    /// how an ingredient line was split: amount · unit · ingredient · preparation
    parsed: String,
}

#[derive(Serialize)]
pub(crate) struct Checks {
    ing: Vec<LineCheck>,
    steps: Vec<LineCheck>,
}

#[tauri::command]
pub(crate) fn check_lines(ing: Vec<String>, steps: Vec<String>) -> Checks {
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
    Checks { ing, steps }
}

/// "Fix all formatting": the classic one-pass cleanup, shown as a list of what changed.
#[tauri::command]
pub(crate) fn review_tidy(state: tauri::State<Shared>, mut review: Review) -> Review {
    let short = state.lock().unwrap().lib.settings.unit_style_short;
    review.sync_text();
    review.changes = importer::tidy_recipe(&mut review.recipe, short);
    review.ing_lines = review.recipe.ingredient_lines().iter().map(|s| s.to_string()).collect();
    review.step_lines = review.recipe.steps();
    review
}

/// Review & Tidy a recipe already in the library (opens on the lines that need a look).
#[tauri::command]
pub(crate) fn review_existing(state: tauri::State<Shared>, id: u64) -> Result<Review, String> {
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
pub(crate) struct Saved {
    id: u64,
    message: String,
}

#[tauri::command]
pub(crate) fn save_review(state: tauri::State<Shared>, mut review: Review) -> Result<Saved, String> {
    review.sync_text();
    mutate(&state, |st| {
        let mut r = review.recipe.clone();
        r.review_flags = importer::review_summary(&r);
        if r.name.trim().is_empty() {
            r.name = "Untitled Recipe".into();
        }
        if !r.nutrition_edited {
            r.nutrition = Some(nutrition::calculate(&r).total);
        }
        // The page's own picture becomes the photo, downloaded in the background.
        if r.photo.is_empty() && crate::web_address(&review.image_url) {
            let name = storage::photo_name(&r.name.to_lowercase().replace(' ', "-"));
            let dir = storage::photos_dir(&st.store.dir);
            let (url, file) = (review.image_url.clone(), name.clone());
            std::thread::spawn(move || {
                if let Ok(bytes) = importer::fetch_bytes(&url) {
                    let _ = storage::write_photo(&bytes, &dir, &file);
                }
            });
            r.photo = name;
        }
        st.store.snapshot(&st.lib, "before import")?;
        let set = st.lib.settings.clone();
        if let Some(id) = review.existing_id {
            let e = st.lib.recipe_mut(id).ok_or(GONE)?;
            *e = r;
            touched(e, &set);
            return Ok(Saved { id, message: "Saved".into() });
        }
        if let (Some(d), true) = (&review.duplicate, review.merge) {
            if let Some(e) = st.lib.recipe_mut(d.id) {
                let changes = importer::merge_into(e, &r);
                if e.review_flags.is_empty() {
                    e.review_flags = importer::review_summary(e);
                }
                touched(e, &set);
                let what = if changes.is_empty() { "nothing new to add".to_string() } else { changes.join("; ") };
                return Ok(Saved { id: d.id, message: format!("Merged into “{}”: {}", d.name, what) });
            }
        }
        touched(&mut r, &set);
        Ok(Saved { id: st.lib.add_recipe(r), message: "Saved to your library".into() })
    })
}

#[derive(Serialize, Default)]
pub(crate) struct Inbox {
    reviews: Vec<Review>,
    notes: Vec<String>,
}

/// Pick up anything dropped in the inbox folder (by Claude, `--import`, or by hand). While the
/// library can't be changed (damaged or newer), the files stay where they are.
#[tauri::command]
pub(crate) async fn take_inbox(state: tauri::State<'_, Shared>) -> Result<Inbox, String> {
    // Locked (damaged or newer library): leave the files where they are until the problem is gone (D-06).
    let locked = state.lock().unwrap().store.problem.is_some();
    if locked {
        return Ok(Inbox::default());
    }
    let items = tauri::async_runtime::spawn_blocking(inbox::take_waiting).await.map_err(|e| e.to_string())?;
    let mut out = Inbox::default();
    for (name, item) in items {
        let origin = format!("Inbox: {name}");
        match item {
            InboxItem::Url(u) => match web_url(&u) {
                Ok(u) => match fetch(u.clone()).await {
                    Ok(html) => out.reviews.push(from_html(&state, &html, &u, &origin)),
                    Err(e) => out.notes.push(format!("{u}: {e}")),
                },
                Err(e) => out.notes.push(format!("{name}: {e}")),
            },
            InboxItem::Html { html, url } => out.reviews.push(from_html(&state, &html, &url, &origin)),
            InboxItem::Text(t) => {
                let s = state.lock().unwrap();
                let (body, extra) = t.split_once('\u{1}').map_or((t.as_str(), None), |(a, b)| (a, Some(b)));
                let mut rv = review_of(importer::import_text(body, &s.lib), &s.lib, &origin);
                if let Some(e) = extra {
                    importer::apply_extra(&mut rv.recipe, e);
                    rv.duplicate = dup_of(&s.lib, &rv.recipe);
                }
                out.reviews.push(rv);
            }
            InboxItem::Food(f) => {
                let food = f.name.clone();
                // the same gate the form uses: no calories or a negative value never reaches My Foods
                if let Err(p) = recipe_box::foods::check_food(&f) {
                    out.notes.push(format!("{food}: {}", p.message));
                    continue;
                }
                match mutate(&state, |st| {
                    nutrition::save_my_food(&mut st.lib, f);
                    Ok(())
                }) {
                    Ok(()) => out.notes.push(format!("Added {food} to My Foods. Recipes updated.")),
                    Err(e) => out.notes.push(format!("{food}: {e}")),
                }
            }
        }
    }
    Ok(out)
}

#[derive(Serialize)]
pub(crate) struct RuleRow {
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
pub(crate) struct RulesOverview {
    rules: Vec<RuleRow>,
    updated_any: bool,
    inbox: String,
}

#[tauri::command]
pub(crate) fn rules_overview(state: tauri::State<Shared>, test: bool) -> RulesOverview {
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
    RulesOverview { rules, updated_any: !lib.rule_overrides.is_empty(), inbox: inbox::inbox_dir().display().to_string() }
}

/// A rules update file's text. Only rules that pass their own test are accepted.
#[tauri::command]
pub(crate) fn load_rules(state: tauri::State<Shared>, text: String) -> Result<String, String> {
    let (ok, rejected) = rules::load_update(&text)?;
    let n = ok.len();
    mutate(&state, |st| {
        for r in ok {
            st.lib.rule_overrides.retain(|o| o.domain != r.domain);
            st.lib.rule_health.retain(|h| h.domain != r.domain);
            st.lib.rule_overrides.push(r);
        }
        Ok(())
    })?;
    Ok(if rejected.is_empty() { format!("Updated {n} rule(s)") } else { format!("Updated {n} rule(s); rejected: {}", rejected.join("; ")) })
}

/// Load rules-update.json from the sync folder, if one is set.
#[tauri::command]
pub(crate) fn rules_from_sync(state: tauri::State<Shared>) -> Result<String, String> {
    let folder = state.lock().unwrap().lib.settings.sync_folder.clone();
    if folder.trim().is_empty() {
        return Err("No sync folder is set up (Settings › Sync).".into());
    }
    let p = std::path::Path::new(&folder).join("rules-update.json");
    let text = std::fs::read_to_string(&p).map_err(|_| format!("There's no rules-update.json in {folder}."))?;
    load_rules(state, text)
}

#[tauri::command]
pub(crate) fn use_builtin_rules(state: tauri::State<Shared>) -> Result<(), String> {
    mutate(&state, |st| {
        st.lib.rule_overrides.clear();
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn addresses_get_a_scheme_and_others_are_refused() {
        assert_eq!(super::web_url("www.allrecipes.com/recipe/1").unwrap(), "https://www.allrecipes.com/recipe/1");
        assert_eq!(super::web_url(" https://a.com/x ").unwrap(), "https://a.com/x");
        for bad in ["", "pancakes", "file:///C:/x.html", "javascript:alert(1)", "https://a b.com"] {
            assert!(super::web_url(bad).is_err(), "{bad}");
        }
    }
}
