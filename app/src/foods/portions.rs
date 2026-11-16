//! Which units a food can be measured in and what one of each weighs (FOOD-04, D-13 to D-15).

use serde::Serialize;

use super::num1;
use crate::ingredients;

/// One choice in the unit picker. `grams` is the weight of one of it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Unit {
    /// `g`, `ml`, `oz`, `cup`, `tbsp`, `serving` or `piece:<name>`
    pub id: String,
    pub label: String,
    pub grams: f32,
}

const METRIC: [&str; 7] = ["g", "ml", "serving", "piece", "tbsp", "cup", "oz"];
const US: [&str; 7] = ["oz", "cup", "tbsp", "g", "ml", "serving", "piece"];

/// The units a food can convert, in the Metric or US order (D-15). Unsupported units are left out, never disabled (D-14):
/// g and oz always; ml, cup and tbsp only with a density (g per ml); a serving only with its weight; each piece with its weight.
/// Grams for oz, cup and tbsp come from `ingredients::convert`, the table recipes use.
pub fn units(serving: Option<(&str, f32)>, density: Option<f32>, pieces: &[(String, f32)], us: bool) -> Vec<Unit> {
    let unit = |id: &str, label: &str, grams: f32| Unit { id: id.into(), label: label.into(), grams };
    let grams_per = |from: &str| ingredients::convert(1.0, from, "gram").unwrap_or(0.0) as f32;
    let ml_per = |from: &str| ingredients::convert(1.0, from, "milliliter").unwrap_or(0.0) as f32;
    let mut v = vec![unit("g", "g", 1.0), unit("oz", "oz", grams_per("ounce"))];
    if let Some(d) = density.filter(|d| d.is_finite() && *d > 0.0) {
        v.push(unit("ml", "ml", d));
        v.push(unit("cup", "cup", ml_per("cup") * d));
        v.push(unit("tbsp", "tbsp", ml_per("tablespoon") * d));
    }
    if let Some((name, g)) = serving.filter(|s| s.1.is_finite() && s.1 > 0.0) {
        v.push(unit("serving", &format!("{} ({} g)", name, num1(g)), g));
    }
    for (name, g) in pieces.iter().filter(|p| p.1.is_finite() && p.1 > 0.0) {
        v.push(unit(&format!("piece:{}", name), &format!("{} ({} g)", name, num1(*g)), *g));
    }
    let order = if us { US } else { METRIC };
    // stable, so pieces stay in the order given
    v.sort_by_key(|u| order.iter().position(|o| *o == u.id.split(':').next().unwrap_or("")));
    v
}
