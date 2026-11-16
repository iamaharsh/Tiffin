//! Sending recipes to health trackers.
//! Cronometer and similar services don't offer a public recipe-upload service for desktop apps,
//! so the app writes a ready-to-import file per recipe into a "tracker outbox" folder (estimated
//! format: per-serving custom food CSV plus the ingredient list for the tracker's recipe importer).

use crate::model::{now_secs, Nutrients, Recipe};
use std::path::PathBuf;

pub const SERVICES: &[&str] = &["Cronometer", "MyFitnessPal", "Other (JSON file)"];

/// The tracker exports folder in use now: the old one while it still holds files, else the Tiffin one.
fn exports_root() -> PathBuf {
    let (_, old, new) = crate::storage::MOVED_FOLDERS[1];
    crate::storage::pick_folder(&crate::storage::documents_dir(), old, new)
}

pub fn default_folder(service: &str) -> PathBuf {
    exports_root().join(service.split(' ').next().unwrap_or("Other"))
}

/// The folder to export into: the default when `folder` is empty, a saved folder inside the old default
/// folder follows the move, and any other saved folder is used as it is.
pub fn folder_for(service: &str, folder: &str) -> PathBuf {
    if folder.trim().is_empty() {
        return default_folder(service);
    }
    let old = crate::storage::documents_dir().join(crate::storage::MOVED_FOLDERS[1].1);
    remap(folder, &old, &exports_root())
}

/// `folder` with the `old_root` prefix swapped for `root`. Windows paths ignore case and slash direction.
fn remap(folder: &str, old_root: &std::path::Path, root: &std::path::Path) -> PathBuf {
    let norm = |s: &str| s.to_lowercase().replace('\\', "/");
    let old_text = old_root.display().to_string();
    let (f, o) = (norm(folder), norm(&old_text));
    if f == o || f.starts_with(&format!("{}/", o.trim_end_matches('/'))) {
        let rest: String = folder.chars().skip(old_text.chars().count()).collect();
        let rest = rest.trim_start_matches(['\\', '/']);
        return if rest.is_empty() { root.to_path_buf() } else { root.join(rest) };
    }
    PathBuf::from(folder)
}

fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
    n.trim().chars().take(60).collect()
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

pub fn per_serving(r: &Recipe) -> Option<Nutrients> {
    r.nutrition.as_ref().map(|n| n.scaled(1.0 / r.servings_count()))
}

pub fn export(r: &Recipe, service: &str, folder: &str) -> Result<PathBuf, String> {
    let n = per_serving(r).ok_or("Calculate nutrition for this recipe first.")?;
    let dir = folder_for(service, folder);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = safe_name(&r.name);
    let serving = format!("1 serving (1/{} of recipe)", crate::ingredients::format_qty(r.servings_count() as f64));
    match service {
        "Cronometer" | "MyFitnessPal" => {
            let header = "Name,Serving Size,Energy (kcal),Protein (g),Carbs (g),Fat (g),Fiber (g),Sugars (g),Sodium (mg)";
            // an unknown value is an empty cell, never 0
            let cell = |v: Option<f32>, digits: usize| v.map(|v| format!("{v:.digits$}")).unwrap_or_default();
            let row = format!(
                "{},{},{},{},{},{},{},{},{}",
                csv_escape(&r.name), csv_escape(&serving), cell(n.calories, 0), cell(n.protein_g, 1), cell(n.carbs_g, 1), cell(n.fat_g, 1), cell(n.fiber_g, 1), cell(n.sugar_g, 1), cell(n.sodium_mg, 0)
            );
            let csv = dir.join(format!("{}.csv", base));
            std::fs::write(&csv, format!("{}\n{}\n", header, row)).map_err(|e| e.to_string())?;
            // Ingredient list for the tracker's own "import recipe" box.
            let txt = format!("{}\nServings: {}\n\n{}\n", r.name, r.servings_count(), r.ingredients);
            std::fs::write(dir.join(format!("{} - ingredients.txt", base)), txt).map_err(|e| e.to_string())?;
            Ok(csv)
        }
        _ => {
            let v = serde_json::json!({
                "name": r.name,
                "servings": r.servings_count(),
                "per_serving": n,
                "ingredients": r.ingredient_lines(),
                "source": r.source_url,
                "exported": now_secs(),
            });
            let p = dir.join(format!("{}.json", base));
            std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| e.to_string())?;
            Ok(p)
        }
    }
}

pub fn service_url(service: &str) -> &'static str {
    match service {
        "Cronometer" => "https://cronometer.com/",
        "MyFitnessPal" => "https://www.myfitnesspal.com/",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn saved_old_default_folder_follows_the_move() {
        let (old, new) = (Path::new(r"C:\Users\a\Documents\Recipe Box Tracker Exports"), Path::new(r"C:\Users\a\Documents\Tiffin Tracker Exports"));
        assert_eq!(remap(r"C:\Users\a\Documents\Recipe Box Tracker Exports\Cronometer", old, new), new.join("Cronometer"));
        assert_eq!(remap(r"c:/users/a/documents/recipe box tracker exports/Cronometer", old, new), new.join("Cronometer"));
        assert_eq!(remap(r"C:\Users\a\Documents\Recipe Box Tracker Exports", old, new), new);
        assert_eq!(remap(r"D:\Exports\Cronometer", old, new), PathBuf::from(r"D:\Exports\Cronometer"));
        assert_eq!(remap(r"C:\Users\a\Documents\Recipe Box Tracker Exports Extra", old, new), PathBuf::from(r"C:\Users\a\Documents\Recipe Box Tracker Exports Extra"));
        assert_eq!(folder_for("Cronometer", " "), default_folder("Cronometer"));
    }

    #[test]
    fn export_leaves_unknown_values_blank() {
        let r = Recipe {
            name: "Test stew".into(),
            servings: "2".into(),
            nutrition: Some(Nutrients { calories: Some(400.0), protein_g: Some(20.0), carbs_g: Some(50.0), fat_g: Some(10.0), fiber_g: None, sugar_g: Some(0.0), sodium_mg: Some(800.0) }),
            ..Default::default()
        };
        let dir = std::env::temp_dir().join(format!("rb-health-export-{}", std::process::id()));
        let folder = dir.to_string_lossy().to_string();
        let csv = export(&r, "Cronometer", &folder).unwrap();
        let text = std::fs::read_to_string(&csv).unwrap();
        let cells: Vec<&str> = text.lines().nth(1).unwrap().split(',').collect();
        assert_eq!(&cells[2..], ["200", "10.0", "25.0", "5.0", "", "0.0", "400"]);
        let json = export(&r, "Other (JSON file)", &folder).unwrap();
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
        assert!(v["per_serving"].get("fiber_g").is_none());
        assert_eq!(v["per_serving"]["sugar_g"], 0.0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
