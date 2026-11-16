//! Recipe inbox: Claude (or any other app or script) can send recipes to Tiffin by dropping a
//! file into the inbox folder, or by running `tiffin --import <file-or-web-address>`.
//!
//! Accepted files:
//!   *.url / *.txt holding just a web address  -> fetched and imported
//!   *.json  schema.org Recipe JSON (with "@type": "Recipe"), or the simple form:
//!           {"name": "...", "ingredients": ["..."], "directions": ["..."], "servings": "4",
//!            "prep_time": "10 min", "cook_time": "20 min", "categories": ["Dinner"],
//!            "notes": "...", "source_url": "https://..."}
//!   *.html / *.htm  a saved web page
//!   *.txt / *.md    recipe text (title first, then Ingredients / Directions headings)
//! Processed files move to the "Imported" sub-folder so nothing is lost.

use std::fs;
use std::path::{Path, PathBuf};

/// Always the new folder: the README and `--import` go here.
pub fn inbox_dir() -> PathBuf {
    crate::storage::documents_dir().join(crate::storage::MOVED_FOLDERS[2].2)
}

/// The folder from before the rename; an old copy of the Claude skill may still drop files there.
pub fn old_inbox_dir() -> PathBuf {
    crate::storage::documents_dir().join(crate::storage::MOVED_FOLDERS[2].1)
}

/// Waiting items from the inbox and, while it exists, the old inbox.
pub fn take_waiting() -> Vec<(String, InboxItem)> {
    let mut out = take_items(&inbox_dir());
    let old = old_inbox_dir();
    if old.exists() {
        out.extend(take_items(&old));
    }
    out
}

pub const README_NAME: &str = "README - how to send recipes (for Claude).md";

/// The message the person pastes into Claude, with this computer's real paths filled in.
pub fn claude_instructions() -> String {
    let dir = inbox_dir();
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "tiffin".into());
    format!(
        r#"Please save recipes to my Tiffin app whenever I ask you to.

How: write one JSON file per recipe into this folder:
  {dir}
Name it after the recipe, ending in .json, for example "weeknight-chili.json".
Tiffin picks the file up within a few seconds and asks me to review it.

Use exactly this format (only "name" and "ingredients" are required):
{{
  "name": "Weeknight Chili",
  "servings": "6",
  "prep_time": "10 min",
  "cook_time": "40 min",
  "categories": ["Dinner"],
  "ingredients": ["1 lb ground beef", "1 can kidney beans (15 oz)", "2 tbsp chili powder"],
  "directions": ["Brown the beef.", "Add everything else and simmer for 30 minutes."],
  "notes": "Optional notes.",
  "source_url": "https://example.com/recipe"
}}
Put one ingredient per list item with its amount first, and one step per directions item.

To add a packaged food from its nutrition label (for example from a photo of the label), write a
.json file like this into the same folder. Values are for ONE serving as printed on the label;
"aliases" are the words recipes use for it:
{{
  "type": "food",
  "name": "Gold Standard Whey, vanilla",
  "aliases": ["protein powder", "whey"],
  "serving": "1 scoop",
  "serving_g": 31,
  "calories": 120, "protein_g": 24, "fat_g": 1.5, "carbs_g": 3,
  "fiber_g": 0, "sugar_g": 1, "sodium_mg": 130
}}
Tiffin adds it to My Foods and updates the nutrition of every recipe that uses it.

To send a recipe that is on a web page, the preferred way is a .url file containing just the address, written to the same folder. Or run:
  "{exe}" --import "https://the-recipe-page"
"#,
        dir = dir.display(),
        exe = exe
    )
}

/// Create the inbox folder and leave a note in it that Claude can read when it looks there.
pub fn setup_inbox() -> Result<PathBuf, String> {
    let dir = inbox_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {}", dir.display(), e))?;
    crate::storage::write_atomic(&dir.join(README_NAME), claude_instructions().as_bytes(), false)?;
    Ok(dir)
}

#[derive(Clone, Debug)]
pub enum InboxItem {
    Url(String),
    Html { html: String, url: String },
    Text(String),
    /// A food from a package label, for My Foods.
    Food(crate::model::MyFood),
}

fn looks_like_url(s: &str) -> Option<String> {
    let t = s.trim();
    let line = t.lines().find(|l| l.trim_start().starts_with("URL=")).map(|l| l.trim()[4..].to_string());
    if let Some(u) = line {
        return Some(u);
    }
    if !t.contains('\n') && !t.contains(' ') && (t.starts_with("http://") || t.starts_with("https://")) {
        return Some(t.to_string());
    }
    None
}

/// Turn the simple JSON form (or schema.org JSON) into something the importer reads.
pub fn json_to_item(text: &str) -> Option<InboxItem> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    if v.get("type").and_then(|t| t.as_str()).map(|t| t.eq_ignore_ascii_case("food")).unwrap_or(false) {
        return json_to_food(&v).map(InboxItem::Food);
    }
    if v.get("@type").is_some() || v.get("@graph").is_some() || v.is_array() {
        let html = format!("<html><head><script type=\"application/ld+json\">{}</script></head><body></body></html>", text);
        let url = v.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
        return Some(InboxItem::Html { html, url });
    }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let list = |k: &str| -> Vec<String> {
        match v.get(k) {
            Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
            Some(serde_json::Value::String(s)) => s.lines().map(|l| l.to_string()).collect(),
            _ => vec![],
        }
    };
    let name = s("name");
    if name.is_empty() && list("ingredients").is_empty() {
        if let Some(u) = v.get("url").and_then(|u| u.as_str()) {
            return Some(InboxItem::Url(u.to_string()));
        }
        return None;
    }
    let mut txt = format!("{}\n", name);
    if !s("servings").is_empty() {
        txt.push_str(&format!("Servings: {}\n", s("servings")));
    }
    txt.push_str("Ingredients\n");
    for l in list("ingredients") {
        txt.push_str(&l);
        txt.push('\n');
    }
    txt.push_str("Directions\n");
    for l in list("directions").into_iter().chain(list("steps")) {
        txt.push_str(&l);
        txt.push('\n');
    }
    if !s("notes").is_empty() {
        txt.push_str("Notes\n");
        txt.push_str(&s("notes"));
        txt.push('\n');
    }
    // carry extra fields through a marker line the app reads back after import
    let extra = serde_json::json!({
        "prep_time": s("prep_time"), "cook_time": s("cook_time"), "source_url": s("source_url"),
        "categories": list("categories"), "video_url": s("video_url"),
    });
    txt.push_str(&format!("\u{1}{}", extra));
    Some(InboxItem::Text(txt))
}

/// {"type": "food", "name": ..., "aliases": [...], "serving": "1 scoop", "serving_g": 31,
///  "calories": 120, "protein_g": 24, "fat_g": 1.5, "carbs_g": 3, "fiber_g": 0, "sugar_g": 1, "sodium_mg": 130}
fn json_to_food(v: &serde_json::Value) -> Option<crate::model::MyFood> {
    // a nutrient the file didn't give (or gave as text that isn't a number) is unknown, not 0
    let nutrient = |k: &str| v.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok()))).map(|x| x as f32).filter(|x| x.is_finite()); // "nan", "inf" and 1e39 are unknown: a non-finite number saves as null and the library then won't load
    let num = |k: &str| nutrient(k).unwrap_or(0.0);
    let name = v.get("name")?.as_str()?.trim().to_string();
    let aliases = match v.get("aliases") {
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "),
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => String::new(),
    };
    let serving_g = num("serving_g");
    if name.is_empty() || serving_g <= 0.0 { // nutrient() never returns NaN or infinity, so this also rejects them
        return None;
    }
    Some(crate::model::MyFood {
        name,
        aliases,
        serving: v.get("serving").and_then(|x| x.as_str()).unwrap_or("1 serving").to_string(),
        serving_g,
        serving_ml: num("serving_ml"),
        per_serving: crate::model::Nutrients {
            calories: nutrient("calories"),
            protein_g: nutrient("protein_g"),
            fat_g: nutrient("fat_g"),
            carbs_g: nutrient("carbs_g"),
            fiber_g: nutrient("fiber_g"),
            sugar_g: nutrient("sugar_g"),
            sodium_mg: nutrient("sodium_mg"),
        },
        ..Default::default()
    })
}

pub fn read_item(path: &Path) -> Option<InboxItem> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let text = fs::read_to_string(path).ok()?;
    match ext.as_str() {
        "url" => looks_like_url(&text).map(InboxItem::Url),
        "json" => json_to_item(&text),
        "html" | "htm" => Some(InboxItem::Html { html: text, url: String::new() }),
        "txt" | "md" | "text" => Some(looks_like_url(&text).map(InboxItem::Url).unwrap_or(InboxItem::Text(text))),
        _ => None,
    }
}

/// Collect waiting items and move their files to Imported/.
pub fn take_items(dir: &Path) -> Vec<(String, InboxItem)> {
    let mut out = vec![];
    let Ok(rd) = fs::read_dir(dir) else { return out };
    let done = dir.join("Imported");
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_file() || p.file_name().map(|n| n == README_NAME).unwrap_or(false) {
            continue;
        }
        if let Some(item) = read_item(&p) {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let _ = fs::create_dir_all(&done);
            if fs::rename(&p, done.join(&name)).is_ok() {
                out.push((name, item));
            }
        }
    }
    out
}

/// `--import <file-or-url>`: copy into the inbox so the (possibly already running) app picks it up.
pub fn send_to_inbox(arg: &str) -> Result<PathBuf, String> {
    let dir = inbox_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = crate::model::now_secs();
    if arg.starts_with("http://") || arg.starts_with("https://") {
        let p = dir.join(format!("link-{}.url", stamp));
        fs::write(&p, format!("[InternetShortcut]\nURL={}\n", arg)).map_err(|e| e.to_string())?;
        return Ok(p);
    }
    let src = Path::new(arg);
    let name = src.file_name().ok_or("Not a file")?.to_string_lossy().to_string();
    let p = dir.join(format!("{}-{}", stamp, name));
    fs::copy(src, &p).map_err(|e| format!("Can't read {}: {}", arg, e))?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inbox_skips_its_note_and_imports_claude_json() {
        let dir = std::env::temp_dir().join(format!("rb-inbox-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(README_NAME), claude_instructions()).unwrap();
        fs::write(dir.join("chili.json"), r#"{"name":"Chili","ingredients":["1 lb beef"],"directions":["Cook."]}"#).unwrap();
        let items = take_items(&dir);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].0, "chili.json");
        assert!(dir.join(README_NAME).exists());
        assert!(claude_instructions().contains("\"ingredients\""));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn simple_json_imports() {
        let j = r#"{"name":"Claude's Chili","servings":"6","ingredients":["1 lb ground beef","1 can kidney beans","2 tbsp chili powder"],"directions":["Brown the beef.","Simmer everything for 30 minutes."],"categories":["Meat"]}"#;
        let Some(InboxItem::Text(t)) = json_to_item(j) else { panic!() };
        let r = crate::importer::import_text(t.split('\u{1}').next().unwrap(), &crate::model::Library::default());
        assert_eq!(r.recipe.name, "Claude's Chili");
        assert_eq!(r.recipe.ingredient_lines().len(), 3);
        assert_eq!(r.recipe.steps().len(), 2);
        assert!(matches!(json_to_item(r#"{"@type":"Recipe","name":"X","recipeIngredient":["1 egg"]}"#), Some(InboxItem::Html { .. })));
        assert_eq!(looks_like_url("[InternetShortcut]\nURL=https://a.com/r\n").as_deref(), Some("https://a.com/r"));
    }

    #[test]
    fn label_foods_come_through_the_inbox() {
        let j = r#"{"type":"food","name":"Gold Standard Whey","aliases":["protein powder","whey"],"serving":"1 scoop","serving_g":31,"calories":120,"protein_g":24,"fat_g":1.5,"carbs_g":3,"sodium_mg":"130"}"#;
        let Some(InboxItem::Food(f)) = json_to_item(j) else { panic!() };
        assert_eq!(f.aliases, "protein powder, whey");
        assert_eq!(f.serving_g, 31.0);
        assert_eq!((f.per_serving.sodium_mg, f.per_serving.fiber_g, f.per_serving.sugar_g), (Some(130.0), None, None));
        assert!(json_to_item(r#"{"type":"food","name":"No weight"}"#).is_none());
        assert!(claude_instructions().contains("\"type\": \"food\""));
    }

    #[test]
    fn non_finite_food_numbers_are_unknown() {
        for w in ["\"nan\"", "\"inf\"", "1e39"] {
            assert!(json_to_item(&format!(r#"{{"type":"food","name":"X","serving_g":{w}}}"#)).is_none(), "{w} as a weight");
        }
        let Some(InboxItem::Food(f)) = json_to_item(r#"{"type":"food","name":"X","serving_g":30,"calories":1e39,"fat_g":"infinity"}"#) else { panic!() };
        assert_eq!((f.per_serving.calories, f.per_serving.fat_g), (None, None));
    }
}
