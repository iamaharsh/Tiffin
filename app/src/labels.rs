//! Filling in a "My Foods" entry from a product link, a barcode or a photo of the label.
//!
//!   * Barcode (or an Open Food Facts link): the free Open Food Facts product database.
//!   * Any other product link: the page's schema.org nutrition data, or failing that the
//!     nutrition panel text on the page.
//!   * Photo of the label: read with the text recognition built into Windows (works offline),
//!     then the same label-text reader.
//! Whatever is found fills the form; the person checks it against the label before saving.

use crate::model::{MyFood, Nutrients};

#[derive(Clone, Debug, Default)]
pub struct LabelGuess {
    pub food: MyFood,
    /// Where the numbers came from, shown next to the form and kept with the food.
    pub source: String,
    /// Label lines that weren't found (so the person knows what to type in).
    pub missing: Vec<&'static str>,
}

// ---------------------------------------------------------------------------------------------
// Label text ("Serving size 1 scoop (31g) Calories 120 Total Fat 1.5g …")

fn norm(text: &str) -> String {
    // lower case, decimal commas to points ("1,5 g"), one line
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        let c = c.to_ascii_lowercase();
        if c == ',' && i > 0 && i + 1 < chars.len() && chars[i - 1].is_ascii_digit() && chars[i + 1].is_ascii_digit() {
            out.push('.');
        } else if c == '\n' || c == '\r' || c == '\t' || c == '|' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    // photos: text recognition reads "1g" as "lg" and "0g" as "og"
    out.split_whitespace()
        .map(|w| {
            let (head, unit) = if let Some(h) = w.strip_suffix("mg") { (h, "mg") } else if let Some(h) = w.strip_suffix('g') { (h, "g") } else { (w, "") };
            if !unit.is_empty() && !head.is_empty() && head.chars().all(|c| matches!(c, 'l' | 'i' | 'o' | '|')) {
                let digits: String = head.chars().map(|c| if c == 'o' { '0' } else { '1' }).collect();
                format!("{}{}", digits, unit)
            } else {
                w.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// First number after position `at` (within a short distance), with the unit word that follows.
fn number_after(t: &str, at: usize, within: usize) -> Option<(f32, String)> {
    let s = &t[at..];
    let lim = s.char_indices().nth(within).map(|(i, _)| i).unwrap_or(s.len());
    let s = &s[..lim];
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let leads = c.is_ascii_digit() || (c == b'.' && i + 1 < b.len() && b[i + 1].is_ascii_digit());
        // "<1 g" counts as 0.5
        if leads {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let mut v: f32 = s[start..i].trim_end_matches('.').parse().ok()?;
            if start > 0 && b[start - 1] == b'<' {
                v /= 2.0;
            }
            let rest = s[i..].trim_start();
            let unit: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            return Some((v, unit));
        }
        i += 1;
    }
    None
}

const KNOWN_UNITS: &[&str] = &["g", "gr", "grams", "gram", "mg", "mcg", "kcal", "kj", "cal", "calories", "ml", "oz", "l"];

/// Find a nutrient line: the first of `labels` not preceded by one of `not_after`
/// ("fat" but not "saturated fat"), then the amount after it.
fn amount_of(t: &str, labels: &[&str], not_after: &[&str], unit_ok: &[&str]) -> Option<f32> {
    for l in labels {
        let mut from = 0;
        while let Some(pos) = t[from..].find(l) {
            let p = from + pos;
            from = p + l.len();
            let before = t[..p].trim_end();
            if not_after.iter().any(|n| before.ends_with(n)) {
                continue;
            }
            // whole word start
            if p > 0 && t.as_bytes()[p - 1].is_ascii_alphabetic() {
                continue;
            }
            if let Some((v, u)) = number_after(t, p + l.len(), 28) {
                // a following word that isn't a unit ("calories 130 total fat") counts as no unit
                let u = if KNOWN_UNITS.contains(&u.as_str()) { u } else { String::new() };
                if unit_ok.is_empty() || unit_ok.iter().any(|x| u == *x) || u.is_empty() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// Read the numbers off nutrition-label text (US, Canadian or European style).
pub fn parse_label_text(text: &str) -> Option<LabelGuess> {
    let t = norm(text);
    let g = &["g", "gr", "grams", "gram"];
    let mut n = Nutrients::default();
    let mut missing = vec![];
    let mut found = 0;
    // a value the label didn't give stays unknown (None), never 0
    let mut take = |v: Option<f32>, slot: &mut Option<f32>, name: &'static str, missing: &mut Vec<&'static str>| match v {
        Some(x) => {
            *slot = Some(x);
            found += 1;
        }
        None => missing.push(name),
    };
    let kcal = amount_of(&t, &["calories", "calorie", "energy", "énergie", "kcal"], &["from fat", "from"], &["kcal", "cal", "calories", ""])
        .or_else(|| {
            // "1590 kj / 380 kcal"
            let p = t.find("kcal")?;
            let before = &t[..p];
            let num: String = before.trim_end().chars().rev().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>().chars().rev().collect();
            num.parse().ok()
        });
    // "energy 1590 kj" without kcal: convert
    let kcal = match kcal {
        Some(v) if t.contains(" kj") && !t.contains("kcal") && !t.contains("calories") => Some(v / 4.184),
        other => other,
    };
    take(kcal, &mut n.calories, "Calories", &mut missing);
    take(amount_of(&t, &["total fat", "fat", "lipides", "lipids"], &["saturated", "trans", "sat.", "sat", "poly", "mono", "polyunsaturated", "monounsaturated", "from"], g), &mut n.fat_g, "Fat", &mut missing);
    take(amount_of(&t, &["total carbohydrate", "carbohydrate", "carbohydrates", "total carbs", "carbs", "glucides"], &[], g), &mut n.carbs_g, "Carbohydrates", &mut missing);
    take(amount_of(&t, &["dietary fiber", "dietary fibre", "fibre", "fiber", "fibres"], &["soluble", "insoluble"], g), &mut n.fiber_g, "Fiber", &mut missing);
    take(amount_of(&t, &["total sugars", "sugars", "sugar", "sucres"], &["added", "incl.", "includes", "alcohol"], g), &mut n.sugar_g, "Sugar", &mut missing);
    take(amount_of(&t, &["protein", "proteins", "protéines"], &[], g), &mut n.protein_g, "Protein", &mut missing);
    let sodium = amount_of(&t, &["sodium"], &[], &["mg", "g"]).map(|v| {
        // "sodium 0.13 g"
        let after = t.find("sodium").and_then(|p| number_after(&t, p + 6, 28)).map(|x| x.1).unwrap_or_default();
        if after == "g" { v * 1000.0 } else { v }
    });
    let sodium = sodium.or_else(|| amount_of(&t, &["salt", "sel"], &[], g).map(|salt| salt * 400.0));
    take(sodium, &mut n.sodium_mg, "Sodium", &mut missing);
    if found < 3 || n.calories.unwrap_or(0.0) <= 0.0 && n.protein_g.unwrap_or(0.0) <= 0.0 && n.carbs_g.unwrap_or(0.0) <= 0.0 {
        return None;
    }
    let (serving, serving_g, serving_ml) = serving_of(&t);
    Some(LabelGuess {
        food: MyFood { serving, serving_g, serving_ml, per_serving: n, ..Default::default() },
        source: String::new(),
        missing,
    })
}

/// "serving size 1 scoop (31g)", "per 1 scoop (31 g)", "2 tbsp (32g)", "per 100 g".
fn serving_of(t: &str) -> (String, f32, f32) {
    for key in ["serving size", "portion", "per ", "pour "] {
        let mut from = 0;
        while let Some(pos) = t[from..].find(key) {
            let p = from + pos + key.len();
            from = p;
            let rest = t[p..].trim_start_matches([':', ' ']);
            let window: String = rest.chars().take(40).collect();
            // "1 scoop (31 g)"
            if let Some(open) = window.find('(') {
                let what = window[..open].trim().to_string();
                if let Some((v, u)) = number_after(&window, open, 12) {
                    match u.as_str() {
                        "g" | "gr" | "grams" => return (if what.is_empty() { format!("{} g", v) } else { what }, v, 0.0),
                        "ml" => return (if what.is_empty() { format!("{} ml", v) } else { what }, v, v),
                        _ => {}
                    }
                }
            }
            // "31 g", "100 g", "250 ml"
            if let Some((v, u)) = number_after(&window, 0, 6) {
                match u.as_str() {
                    "g" | "gr" | "grams" => return (format!("{} g", v), v, 0.0),
                    "ml" => return (format!("{} ml", v), v, v),
                    _ => {}
                }
            }
        }
    }
    ("1 serving".into(), 0.0, 0.0)
}

// ---------------------------------------------------------------------------------------------
// Links and barcodes

fn is_barcode(s: &str) -> bool {
    let d = s.trim();
    (8..=14).contains(&d.len()) && d.chars().all(|c| c.is_ascii_digit())
}

fn off_barcode_from_url(url: &str) -> Option<String> {
    if !url.contains("openfoodfacts.") {
        return None;
    }
    let after = url.split("/product/").nth(1)?;
    let code: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if is_barcode(&code) { Some(code) } else { None }
}

/// Look up a link or a barcode number. Runs on a background thread (network). A barcode (or Open Food Facts link) needs the
/// person's contact `email`; without a valid one nothing is sent and the error is `foods::off::NEEDS_EMAIL`.
pub fn lookup(input: &str, email: &str) -> Result<LabelGuess, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Paste a product link or type the barcode number.".into());
    }
    if let Some(code) = if is_barcode(s) { Some(s.to_string()) } else { off_barcode_from_url(s) } {
        return open_food_facts(&code, email);
    }
    if !(s.starts_with("http://") || s.starts_with("https://") || s.contains('.')) {
        return Err("That doesn't look like a link or a barcode (8–14 digits).".into());
    }
    let html = crate::importer::fetch(s)?;
    from_html(&html, s).ok_or_else(|| {
        "No nutrition facts found on that page. Try a photo of the label, or the barcode number.".to_string()
    })
}

fn open_food_facts(code: &str, email: &str) -> Result<LabelGuess, String> {
    let agent_name = crate::foods::off::user_agent(email).ok_or_else(|| crate::foods::off::NEEDS_EMAIL.to_string())?;
    let url = format!(
        "https://world.openfoodfacts.org/api/v2/product/{}.json?fields=product_name,brands,serving_size,serving_quantity,nutriments",
        code
    );
    let tls = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
    let agent = ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).timeout(std::time::Duration::from_secs(20)).build();
    let text = agent
        .get(&url)
        // Open Food Facts asks apps to name themselves
        .set("User-Agent", &agent_name)
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "That barcode isn't in Open Food Facts. Try a photo of the label.".to_string(),
            other => format!("Couldn't reach Open Food Facts: {}", other),
        })?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if v.get("status").and_then(|s| s.as_i64()) == Some(0) {
        return Err("That barcode isn't in Open Food Facts. Try a photo of the label.".into());
    }
    off_product(&v["product"], code).ok_or_else(|| "Open Food Facts has this product but no nutrition facts for it. Try a photo of the label.".into())
}

fn off_product(p: &serde_json::Value, code: &str) -> Option<LabelGuess> {
    let n = &p["nutriments"];
    let num = |k: &str| n.get(k).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).map(|x| x as f32);
    let sq = p.get("serving_quantity").and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|s| s.parse().ok()))).map(|x| x as f32).filter(|x| *x > 0.0);
    let (serving, grams) = match sq {
        Some(g) => (p.get("serving_size").and_then(|x| x.as_str()).unwrap_or("1 serving").to_string(), g),
        None => ("100 g".to_string(), 100.0),
    };
    let k = grams / 100.0;
    let per100 = |key: &str| num(&format!("{}_100g", key));
    let kcal = per100("energy-kcal").or_else(|| per100("energy").map(|kj| kj / 4.184))?;
    let mut missing = vec![];
    let mut get = |key: &str, label: &'static str, scale: f32| match per100(key) {
        Some(v) => Some(v * k * scale),
        None => {
            missing.push(label);
            None
        }
    };
    let per_serving = Nutrients {
        calories: Some(kcal * k),
        protein_g: get("proteins", "Protein", 1.0),
        fat_g: get("fat", "Fat", 1.0),
        carbs_g: get("carbohydrates", "Carbohydrates", 1.0),
        fiber_g: get("fiber", "Fiber", 1.0),
        sugar_g: get("sugars", "Sugar", 1.0),
        sodium_mg: get("sodium", "Sodium", 1000.0),
    };
    let name = [p.get("brands").and_then(|x| x.as_str()).unwrap_or("").split(',').next().unwrap_or("").trim(), p.get("product_name").and_then(|x| x.as_str()).unwrap_or("").trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    Some(LabelGuess {
        food: MyFood { name, serving, serving_g: grams, per_serving, ..Default::default() },
        source: format!("Open Food Facts, barcode {}", code),
        missing,
    })
}

/// A product page: schema.org nutrition (JSON-LD), else the nutrition panel's text.
pub fn from_html(html: &str, url: &str) -> Option<LabelGuess> {
    let doc = scraper::Html::parse_document(html);
    let title = scraper::Selector::parse("meta[property='og:title'], title").ok().and_then(|s| {
        doc.select(&s).next().map(|e| e.value().attr("content").map(|c| c.to_string()).unwrap_or_else(|| e.text().collect::<String>()))
    });
    let title = title.map(|t| crate::importer::decode_entities(t.split(" | ").next().unwrap_or(&t).split(" - ").next().unwrap_or(&t).trim()));
    let host = url.split("//").nth(1).unwrap_or(url).split('/').next().unwrap_or("").trim_start_matches("www.").to_string();

    if let Ok(sel) = scraper::Selector::parse("script[type='application/ld+json']") {
        for s in doc.select(&sel) {
            let txt: String = s.text().collect();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) {
                if let Some(mut g) = find_nutrition(&v) {
                    if g.food.name.is_empty() {
                        g.food.name = title.clone().unwrap_or_default();
                    }
                    g.source = format!("Product page ({})", host);
                    return Some(g);
                }
            }
        }
    }
    // the page text: keep only the part around the nutrition panel when there is one
    let body = scraper::Selector::parse("body").ok().and_then(|s| doc.select(&s).next().map(|b| b.text().collect::<Vec<_>>().join(" ")))?;
    let lc = body.to_lowercase();
    let start = ["nutrition facts", "nutrition information", "nutritional information", "valeur nutritive", "nutrition"].iter().find_map(|k| lc.find(k)).unwrap_or(0);
    let slice: String = body[start..].chars().take(1500).collect();
    let mut g = parse_label_text(&slice)?;
    g.food.name = title.unwrap_or_default();
    g.source = format!("Product page ({})", host);
    Some(g)
}

fn find_nutrition(v: &serde_json::Value) -> Option<LabelGuess> {
    match v {
        serde_json::Value::Array(a) => a.iter().find_map(find_nutrition),
        serde_json::Value::Object(o) => {
            if let Some(n) = o.get("nutrition").filter(|n| n.is_object()) {
                let s = |k: &str| n.get(k).map(|x| match x {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
                let text = format!(
                    "serving size {} calories {} total fat {} carbohydrate {} fiber {} sugars {} protein {} sodium {}",
                    s("servingSize").unwrap_or_default(),
                    s("calories").unwrap_or_default(),
                    s("fatContent").unwrap_or_default(),
                    s("carbohydrateContent").unwrap_or_default(),
                    s("fiberContent").unwrap_or_default(),
                    s("sugarContent").unwrap_or_default(),
                    s("proteinContent").unwrap_or_default(),
                    s("sodiumContent").unwrap_or_default(),
                );
                if let Some(mut g) = parse_label_text(&text) {
                    g.food.name = o.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    return Some(g);
                }
            }
            o.values().find_map(find_nutrition)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Photos

/// Read the text in a photo with Windows' own text recognition (offline).
pub fn read_photo(path: &std::path::Path) -> Result<String, String> {
    // Windows reads pictures up to about 2600 px; phone photos are bigger, so shrink a copy.
    // by content, not file extension (pictures can arrive as extensionless temp files)
    let img = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| format!("Couldn't open the picture: {}", e))?
        .decode()
        .map_err(|e| format!("Couldn't open the picture: {}", e))?;
    let img = if img.width().max(img.height()) > 2400 { img.resize(2400, 2400, image::imageops::FilterType::Lanczos3) } else { img };
    let tmp = std::env::temp_dir().join(format!("recipebox-label-{}.png", crate::importer::unique_stamp()));
    let saved = img.save(&tmp).map_err(|e| e.to_string());
    let text = saved.and_then(|_| ocr_png(&tmp));
    let _ = std::fs::remove_file(&tmp);
    text
}

/// Windows text recognition. The picture's path travels in an environment variable, so no file or
/// folder name can ever become PowerShell script text.
const OCR_SCRIPT: &str = r#"
$path = $env:RECIPEBOX_OCR_IMAGE
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
try {
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0]
function Await($op, $type) { $t = $asTask.MakeGenericMethod($type).Invoke($null, @($op)); $t.Wait(-1) | Out-Null; $t.Result }
[Windows.Storage.StorageFile,Windows.Storage,ContentType=WindowsRuntime] | Out-Null
[Windows.Media.Ocr.OcrEngine,Windows.Foundation,ContentType=WindowsRuntime] | Out-Null
[Windows.Graphics.Imaging.BitmapDecoder,Windows.Graphics,ContentType=WindowsRuntime] | Out-Null
$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync($path)) ([Windows.Storage.StorageFile])
$stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream])
$decoder = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
$bitmap = Await ($decoder.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap])
$engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages()
$result = Await ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
$stream.Dispose()
foreach ($l in $result.Lines) { [Console]::Out.WriteLine($l.Text) }
} catch { [Console]::Error.WriteLine($_.Exception.GetBaseException().Message); exit 1 }
"#;

fn ocr_command(png: &std::path::Path) -> std::process::Command {
    use base64::Engine;
    // -EncodedCommand takes the script as base64 of UTF-16LE: no script file, no execution-policy override.
    // (The script catches its own errors: with an encoded command PowerShell would put them on stderr as CLIXML.)
    let utf16: Vec<u8> = OCR_SCRIPT.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded]).env("RECIPEBOX_OCR_IMAGE", png);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

fn ocr_png(png: &std::path::Path) -> Result<String, String> {
    let out = ocr_command(png).output().map_err(|e| format!("Couldn't start Windows text recognition: {}", e))?;
    if !out.status.success() {
        return Err(format!("Windows couldn't read the picture: {}", String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Photo of a nutrition label → numbers.
pub fn from_photo(path: &std::path::Path) -> Result<LabelGuess, String> {
    let text = read_photo(path)?;
    let mut g = parse_label_text(&text).ok_or_else(|| {
        "Couldn't find the nutrition numbers in that photo. Try a sharper, straight-on photo of just the Nutrition Facts panel.".to_string()
    })?;
    g.source = format!("Photo of the label ({})", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    Ok(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn us_label() {
        let t = "Nutrition Facts\n30 servings per container\nServing size 1 scoop (31g)\nAmount per serving\nCalories 120\n% Daily Value*\nTotal Fat 1.5g 2%\nSaturated Fat 1g 5%\nTrans Fat 0g\nCholesterol 35mg 12%\nSodium 130mg 6%\nTotal Carbohydrate 3g 1%\nDietary Fiber 0g 0%\nTotal Sugars 1g\nIncludes 0g Added Sugars 0%\nProtein 24g";
        let g = parse_label_text(t).unwrap();
        let n = &g.food.per_serving;
        assert_eq!((n.calories, n.fat_g, n.carbs_g, n.fiber_g, n.sugar_g, n.protein_g, n.sodium_mg), (Some(120.0), Some(1.5), Some(3.0), Some(0.0), Some(1.0), Some(24.0), Some(130.0)));
        assert_eq!(g.food.serving, "1 scoop");
        assert_eq!(g.food.serving_g, 31.0);
        assert!(g.missing.is_empty());
    }

    #[test]
    fn a_value_the_label_lacks_stays_unknown() {
        let g = parse_label_text("Nutrition Facts\nServing size 1 bar (40g)\nCalories 150\nTotal Fat 5g\nTotal Carbohydrate 20g\nProtein 6g\nSodium 90mg").unwrap();
        assert_eq!((g.food.per_serving.sugar_g, g.food.per_serving.fiber_g), (None, None));
        assert!(g.missing.contains(&"Sugar"));
        assert_eq!(g.food.per_serving.sodium_mg, Some(90.0));
    }

    #[test]
    fn text_read_from_a_photo() {
        // as Windows text recognition returned it for a test picture
        let t = "Nutrition Facts
28 servings per container
Serving size 1 scoop (32g)
Calories 130
Total Fat 2g
Saturated Fat lg
Trans Fat Og
5%
Cholesterol 40mg 13%
Sodium 150mg 7%
Total Carbohydrate 4g 1%
Dietary Fiber lg 4%
Total Sugars 2g
Protein 25g";
        let g = parse_label_text(t).unwrap();
        let n = &g.food.per_serving;
        assert_eq!((n.calories, n.fat_g, n.carbs_g, n.fiber_g, n.sugar_g, n.protein_g, n.sodium_mg), (Some(130.0), Some(2.0), Some(4.0), Some(1.0), Some(2.0), Some(25.0), Some(150.0)));
        assert_eq!(g.food.serving_g, 32.0);
        assert!(g.missing.is_empty());
    }

    #[test]
    fn canadian_bilingual_label() {
        let t = "Nutrition Facts Valeur nutritive Per 1/2 cup (85 g) pour 1/2 tasse (85 g) Calories 150 % Daily Value* Fat / Lipides 2 g 3 % Carbohydrate / Glucides 6 g Fibre / Fibres 1 g 4 % Sugars / Sucres 1 g Protein / Protéines 26 g Cholesterol / Cholestérol 0 mg Sodium 370 mg 16 %";
        let g = parse_label_text(t).unwrap();
        let n = &g.food.per_serving;
        assert_eq!((n.calories, n.fat_g, n.carbs_g, n.fiber_g, n.protein_g, n.sodium_mg), (Some(150.0), Some(2.0), Some(6.0), Some(1.0), Some(26.0), Some(370.0)));
        assert_eq!(g.food.serving_g, 85.0);
        assert_eq!(g.food.serving, "1/2 cup");
    }

    #[test]
    fn european_per_100g_label() {
        let t = "Nutrition per 100g: Energy 1590kJ / 380kcal, Fat 6,5g of which saturates 1,2g, Carbohydrate 60g of which sugars 1,1g, Fibre 10g, Protein 13g, Salt 0,02g";
        let g = parse_label_text(t).unwrap();
        let n = &g.food.per_serving;
        assert_eq!((n.calories, n.fat_g, n.sugar_g), (Some(380.0), Some(6.5), Some(1.1)));
        assert!((n.sodium_mg.unwrap() - 8.0).abs() < 0.01);
        assert_eq!(g.food.serving_g, 100.0);
    }

    #[test]
    fn product_page_json_ld() {
        let html = r#"<html><head><title>Seitan Strips | Shop</title><script type="application/ld+json">{"@type":"Product","name":"Original Seitan Strips","nutrition":{"@type":"NutritionInformation","servingSize":"3 oz (85 g)","calories":"130 calories","fatContent":"2 g","carbohydrateContent":"4 g","fiberContent":"1 g","sugarContent":"1 g","proteinContent":"24 g","sodiumContent":"380 mg"}}</script></head><body></body></html>"#;
        let g = from_html(html, "https://www.example.com/seitan").unwrap();
        assert_eq!(g.food.name, "Original Seitan Strips");
        assert_eq!(g.food.per_serving.protein_g, Some(24.0));
        assert_eq!(g.food.serving_g, 85.0);
        assert!(g.source.contains("example.com"));
    }

    #[test]
    fn open_food_facts_answer() {
        let v: serde_json::Value = serde_json::from_str(r#"{"product_name":"Vital Wheat Gluten","brands":"Bob's Red Mill","serving_size":"1/4 cup (30 g)","serving_quantity":30,"nutriments":{"energy-kcal_100g":400,"proteins_100g":76.7,"fat_100g":3.3,"carbohydrates_100g":13.3,"fiber_100g":3.3,"sugars_100g":0,"sodium_100g":0.133}}"#).unwrap();
        let g = off_product(&v, "039978003850").unwrap();
        assert_eq!(g.food.name, "Bob's Red Mill Vital Wheat Gluten");
        assert!((g.food.per_serving.calories.unwrap() - 120.0).abs() < 0.1);
        assert!((g.food.per_serving.sodium_mg.unwrap() - 39.9).abs() < 0.1);
        assert!(is_barcode("039978003850"));
        assert_eq!(off_barcode_from_url("https://world.openfoodfacts.org/product/3017620422003/nutella").as_deref(), Some("3017620422003"));
    }

    #[test]
    fn barcode_lookup_sends_nothing_without_an_email() {
        assert_eq!(lookup("039978003850", "").unwrap_err(), crate::foods::off::NEEDS_EMAIL);
        assert_eq!(lookup("https://world.openfoodfacts.org/product/3017620422003/nutella", "nope").unwrap_err(), crate::foods::off::NEEDS_EMAIL);
    }

    #[test]
    fn ocr_path_never_enters_the_script() {
        use base64::Engine;
        let path = std::path::Path::new("C:\\Users\\a b\\it's \u{2019}x\\label.png");
        let cmd = ocr_command(path);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().to_string()).collect();
        assert!(!args.iter().any(|a| a.contains("label.png")), "{args:?}");
        let env = cmd.get_envs().find(|(k, _)| k.to_str() == Some("RECIPEBOX_OCR_IMAGE")).and_then(|(_, v)| v).expect("env var");
        assert_eq!(std::path::Path::new(env), path);
        let at = args.iter().position(|a| a == "-EncodedCommand").expect("-EncodedCommand");
        let bytes = base64::engine::general_purpose::STANDARD.decode(&args[at + 1]).unwrap();
        let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let script = String::from_utf16(&units).unwrap();
        assert_eq!(script, OCR_SCRIPT);
        assert!(script.contains("$env:RECIPEBOX_OCR_IMAGE") && !script.contains("label.png"));
        assert_eq!(args[..3], ["-NoProfile", "-NonInteractive", "-EncodedCommand"]); // no script file, no policy override
        assert_eq!(args.len(), 4);
    }

    #[cfg(windows)]
    #[test]
    fn ocr_reads_a_picture_whose_path_has_spaces_and_quotes() {
        let dir = std::env::temp_dir().join(format!("rb ocr it's \u{2019}[x] {}", crate::importer::unique_stamp()));
        std::fs::create_dir_all(&dir).unwrap();
        let png = dir.join("label one.png");
        image::RgbImage::from_pixel(200, 100, image::Rgb([255, 255, 255])).save(&png).unwrap();
        let direct = ocr_png(&png);
        let full = read_photo(&png);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(direct.is_ok(), "{direct:?}");
        assert!(full.is_ok(), "{full:?}");
    }
}
