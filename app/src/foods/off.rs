//! Open Food Facts search, only when the person presses Search Online; sends only the typed text.
//!
//! The service is an enhancement: every state it can be in (offline, slow, throttled) comes back
//! as an [`OffAnswer`], never as a failure of the app. Nothing here logs the query or the response.

use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::OnlineFood;
use crate::model::{MyFood, Nutrients, APP_VERSION};

pub const OFF_SEARCH_URL: &str = "https://search.openfoodfacts.org/search";

/// What a barcode lookup returns when no valid contact email is saved; the page recognises it and asks for one.
pub const NEEDS_EMAIL: &str = "needs_email";

/// The one rule for the contact email: trimmed, one `@`, a dot in the domain, no gaps in it. The value goes into a
/// request header, so only plain visible ASCII is allowed and header-breaking characters are refused. Returns the trimmed value.
pub fn valid_email(s: &str) -> Result<String, String> {
    let s = s.trim();
    let bad = || "That doesn't look like an email address. Use the form name@example.com.".to_string();
    let (local, domain) = s.split_once('@').ok_or_else(bad)?;
    let plain = s.len() <= 254 && s.chars().all(|c| c.is_ascii_graphic() && !"()<>[],;:\\\"".contains(c));
    if !plain || local.is_empty() || domain.contains('@') || !domain.contains('.') || domain.split('.').any(str::is_empty) {
        return Err(bad());
    }
    Ok(s.to_string())
}

/// The User-Agent for every Open Food Facts request: `Tiffin/<version> (<email>)`. `None` when the email isn't valid, so nothing is sent.
pub fn user_agent(email: &str) -> Option<String> {
    valid_email(email).ok().map(|e| format!("Tiffin/{} ({})", APP_VERSION, e))
}

pub const OFF_MIN_LETTERS: usize = 2;

/// The one rule for when a query may be sent online; the page shows it, the command enforces it.
pub fn query_ready(q: &str) -> bool {
    q.trim().chars().count() >= OFF_MIN_LETTERS
}

/// What came back from one online search.
#[derive(Debug, Clone, PartialEq)]
pub enum OffAnswer {
    Found(Vec<OnlineFood>),
    Empty,
    Offline,
    Slow,
    /// The service said to stop (429 or 503); the seconds to wait.
    Limited(u64),
    /// No valid contact email is saved, so nothing was sent.
    NeedsEmail,
}

/// Search Open Food Facts for `text`, naming the person's `email` in the User-Agent. One request, 8 s timeout, at most 2 MB read.
pub fn search(text: &str, email: &str) -> OffAnswer {
    search_at(OFF_SEARCH_URL, text, email)
}

/// Seconds to wait after a stop. The header is the server's word, so it is bounded: at least 1 second, at most an hour.
fn retry_after(header: Option<&str>) -> u64 {
    header.and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(60).clamp(1, 3600)
}

fn search_at(url: &str, text: &str, email: &str) -> OffAnswer {
    let Some(agent_name) = user_agent(email) else { return OffAnswer::NeedsEmail };
    let Ok(tls) = native_tls::TlsConnector::new() else { return OffAnswer::Slow };
    // no redirects: the host is fixed and a search never needs one
    let agent = ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).timeout(Duration::from_secs(8)).redirects(0).build();
    let sent = agent
        .get(url)
        .set("User-Agent", &agent_name)
        .query("q", text)
        .query("page_size", "20")
        .query("langs", "en")
        .query("fields", "code,product_name,brands,serving_size,serving_quantity,nutriments")
        .call();
    let resp = match sent {
        Ok(r) => r,
        Err(ureq::Error::Status(429 | 503, r)) => return OffAnswer::Limited(retry_after(r.header("Retry-After"))),
        Err(ureq::Error::Status(..)) => return OffAnswer::Slow,
        Err(ureq::Error::Transport(t)) => {
            return if matches!(t.kind(), ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed) { OffAnswer::Offline } else { OffAnswer::Slow };
        }
    };
    if resp.status() != 200 {
        return OffAnswer::Slow;
    }
    let mut body = Vec::new();
    if resp.into_reader().take(2 * 1024 * 1024).read_to_end(&mut body).is_err() {
        return OffAnswer::Slow;
    }
    let Ok(v) = serde_json::from_slice::<Value>(&body) else { return OffAnswer::Slow };
    let hits = parse_hits(&v);
    if hits.is_empty() {
        OffAnswer::Empty
    } else {
        OffAnswer::Found(hits)
    }
}

/// A JSON number, or a number written as text; a missing, non-numeric, non-finite or negative value is `None` (unknown).
fn num(v: &Value, key: &str) -> Option<f64> {
    let x = v.get(key)?;
    x.as_f64().or_else(|| x.as_str().and_then(|s| s.trim().parse().ok())).filter(|x| x.is_finite() && *x >= 0.0)
}

/// A bare weight such as "40 g" or "30ml".
fn bare_weight(s: &str) -> bool {
    let s = s.trim().to_lowercase();
    let n = ["ml", "oz", "g"].iter().find_map(|u| s.strip_suffix(u)).unwrap_or("");
    n.trim().parse::<f32>().is_ok()
}

/// The serving text without its gram weight: the unit list adds "(40 g)" itself, so "1 bar (40 g)" stays "1 bar"
/// and a bare "40 g" becomes "" (shown as "serving").
fn strip_weight(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else { break };
        let inner = &rest[open + 1..open + close];
        out.push_str(&rest[..open]);
        if !bare_weight(inner) {
            out.push_str(&rest[open..open + close + 1]);
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if bare_weight(&out) { String::new() } else { out }
}

/// The foods in a search response (`hits`): brands as an array, energy from kcal, then kJ, per 100 g, sodium in mg,
/// a missing value is `None`. A product whose figures look wrong is flagged `suspect`. Codes are deduplicated with
/// leading zeros stripped, keeping the first.
pub fn parse_hits(v: &Value) -> Vec<OnlineFood> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut out = vec![];
    for h in v.get("hits").and_then(|h| h.as_array()).into_iter().flatten() {
        let code = h.get("code").and_then(|c| c.as_str().map(String::from).or_else(|| c.as_i64().map(|n| n.to_string()))).unwrap_or_default();
        let name = h.get("product_name").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
        let key = code.trim().trim_start_matches('0').to_string();
        if key.is_empty() || name.is_empty() || seen.insert(key, ()).is_some() {
            continue;
        }
        let brands = match h.get("brands") {
            Some(Value::Array(a)) => a.iter().filter_map(|b| b.as_str()).map(str::trim).filter(|b| !b.is_empty()).collect::<Vec<_>>().join(", "),
            Some(Value::String(s)) => s.trim().to_string(),
            _ => String::new(),
        };
        let n = h.get("nutriments").unwrap_or(&Value::Null);
        let kcal = num(n, "energy-kcal_100g").or_else(|| num(n, "energy-kj_100g").map(|kj| kj / 4.184)).or_else(|| num(n, "energy_100g").map(|kj| kj / 4.184));
        let g = |k: &str| num(n, k).map(|x| x as f32);
        let per100 = Nutrients {
            calories: kcal.map(|x| x as f32),
            protein_g: g("proteins_100g"),
            fat_g: g("fat_100g"),
            carbs_g: g("carbohydrates_100g"),
            fiber_g: g("fiber_100g"),
            sugar_g: g("sugars_100g"),
            sodium_mg: num(n, "sodium_100g").map(|x| (x * 1000.0) as f32),
        };
        let serving_g = num(h, "serving_quantity").filter(|x| *x > 0.0).map(|x| x as f32).unwrap_or(0.0);
        let serving = if serving_g > 0.0 { h.get("serving_size").and_then(|s| s.as_str()).map(strip_weight).unwrap_or_default() } else { String::new() };
        out.push(OnlineFood { code, name, brands, suspect: is_suspect(&per100), per100, serving, serving_g, ..Default::default() });
    }
    out
}

/// Calories far from 4P + 4C + 9F (more than 25% and more than 10 kcal off), a macro above 100 g in 100 g,
/// more than 900 kcal in 100 g (pure fat is about 884), or sodium above 40,000 mg in 100 g (salt is about 39,000).
fn is_suspect(n: &Nutrients) -> bool {
    let macros = [n.protein_g, n.carbs_g, n.fat_g, n.fiber_g, n.sugar_g];
    if macros.iter().flatten().any(|m| *m > 100.0) || n.calories.is_some_and(|k| k > 900.0) || n.sodium_mg.is_some_and(|m| m > 40000.0) {
        return true;
    }
    match (n.calories, n.protein_g, n.carbs_g, n.fat_g) {
        (Some(k), Some(p), Some(c), Some(f)) => {
            let expected = 4.0 * p + 4.0 * c + 9.0 * f;
            (k - expected).abs() > 0.25 * expected.max(10.0 * 4.0)
        }
        _ => false,
    }
}

/// Online searches allowed in any rolling minute (OFF documents 10 a minute and a ban for more).
pub const OFF_PER_MINUTE: usize = 8;

/// Counts online sends so the command can refuse the next one before any request is made.
#[derive(Debug, Default)]
pub struct Limiter {
    sent: VecDeque<u64>,
    blocked_until: u64,
}

impl Limiter {
    /// Records a send at `now` (seconds) and says `Ok`, or says `Err(seconds to wait)` and records nothing.
    pub fn allow(&mut self, now: u64) -> Result<(), u64> {
        if now < self.blocked_until {
            return Err(self.blocked_until - now);
        }
        while self.sent.front().is_some_and(|t| t + 60 <= now) {
            self.sent.pop_front();
        }
        if self.sent.len() >= OFF_PER_MINUTE {
            return Err(self.sent[0] + 60 - now);
        }
        self.sent.push_back(now);
        Ok(())
    }

    /// The service said stop: refuse everything until `now + secs`.
    pub fn block(&mut self, now: u64, secs: u64) {
        self.blocked_until = now.saturating_add(secs);
    }
}

/// Most opened products kept for offline search.
pub const OFF_CACHE_CAP: usize = 5000;

/// `cache\off-products.json` in the data folder. Disposable: not in backups, not in the sync folder.
pub fn cache_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("cache").join("off-products.json")
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct CacheFile {
    items: Vec<OnlineFood>,
}

/// Products the person opened, kept so later searches find them offline; the least recently used go first past the cap.
pub struct OffCache {
    path: PathBuf,
    cap: usize,
    items: Vec<OnlineFood>,
}

fn code_key(code: &str) -> &str {
    code.trim().trim_start_matches('0')
}

impl OffCache {
    /// Missing is empty. A file that can't be read or parsed is deleted and the cache starts empty: it can be fetched again, so no damaged copy is kept.
    pub fn open(path: PathBuf) -> OffCache {
        let items = match std::fs::read(&path) {
            Ok(b) => match serde_json::from_slice::<CacheFile>(&b) {
                Ok(f) => f.items,
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                    vec![]
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(_) => {
                let _ = std::fs::remove_file(&path);
                vec![]
            }
        };
        OffCache { path, cap: OFF_CACHE_CAP, items }
    }

    pub fn items(&self) -> &[OnlineFood] {
        &self.items
    }

    /// Keep `food` (a product seen again replaces its entry and keeps its first `fetched` time), drop the least recently used past the cap, and save.
    pub fn remember(&mut self, mut food: OnlineFood, now: u64) -> Result<(), String> {
        food.last_used = now;
        match self.items.iter_mut().find(|f| code_key(&f.code) == code_key(&food.code)) {
            Some(old) => {
                food.fetched = old.fetched;
                *old = food;
            }
            None => {
                food.fetched = now;
                self.items.push(food);
            }
        }
        while self.items.len() > self.cap {
            let oldest = (0..self.items.len()).min_by_key(|&i| self.items[i].last_used).unwrap_or(0);
            self.items.remove(oldest);
        }
        let bytes = serde_json::to_vec(&CacheFile { items: self.items.clone() }).map_err(|e| e.to_string())?;
        crate::storage::write_atomic(&self.path, &bytes, false).map_err(|e| format!("Couldn't keep that product for offline use: {e}"))
    }
}

/// An online food as a My Food draft: serving "100 g" when the product has no serving weight. The flag says it can be saved as it is:
/// calories are known and the values don't look wrong. Otherwise the form opens for review.
pub fn as_my_food(f: &OnlineFood) -> (MyFood, bool) {
    let name = [f.brands.split(',').next().unwrap_or("").trim(), f.name.trim()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
    let (serving, serving_g) = if f.serving_g > 0.0 {
        (if f.serving.trim().is_empty() { "1 serving".to_string() } else { f.serving.trim().to_string() }, f.serving_g)
    } else {
        ("100 g".to_string(), 100.0)
    };
    let food = MyFood { name, serving, serving_g, per_serving: f.per100.scaled(serving_g / 100.0), source: format!("Open Food Facts, product {}", f.code), ..Default::default() };
    (food, f.per100.calories.is_some() && !f.suspect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn user_agent_names_tiffin_and_the_contact() {
        assert_eq!(user_agent(" a.b@example.com ").unwrap(), format!("Tiffin/{} (a.b@example.com)", APP_VERSION));
        assert_eq!(user_agent(""), None);
    }

    #[test]
    fn email_rule_keeps_header_breakers_out() {
        assert_eq!(valid_email("  me@example.co.uk "), Ok("me@example.co.uk".to_string()));
        for bad in ["", "   ", "me", "me@example", "@example.com", "me@", "me@@example.com", "a@b@example.com", "me@.com", "me@example.", "me@exa..mple.com", "me @example.com", "me@exam ple.com", "me@example.com\r\nX-Evil: 1", "me@exa\nmple.com", "me\t@example.com", "(me)@example.com", "me@example.com (x)", "me@exämple.com", "<me@example.com>", "\"me\"@example.com"] {
            assert!(valid_email(bad).is_err(), "{bad:?} must be refused");
        }
        assert_eq!(valid_email("me@example.com\n"), Ok("me@example.com".to_string()), "the edge whitespace is trimmed away, never sent");
        assert!(valid_email(&format!("{}@example.com", "a".repeat(250))).is_err(), "too long for a header contact");
    }

    #[test]
    fn short_queries_are_not_ready() {
        assert!(!query_ready("") && !query_ready("a") && !query_ready("  a  "));
        assert!(query_ready("ab") && query_ready("él") && query_ready(" ab "));
    }

    #[test]
    fn off_hit_parse_array_brands_and_kj() {
        let v = json!({"hits": [
            {"code": "0894700010137", "brands": ["Chobani"], "product_name": "Nonfat Greek Yogurt",
             "nutriments": {"carbohydrates_100g": 3.53, "energy-kcal_100g": 52.9, "fat_100g": 0, "proteins_100g": 9.41, "sodium_100g": 0.0382, "sugars_100g": 3.53}},
            {"code": "111", "product_name": "Kilojoule Bar", "nutriments": {"energy-kj_100g": 418.4}},
            {"code": "222", "product_name": "", "nutriments": {}},
            {"code": "333", "product_name": "Text numbers", "brands": "A, B", "serving_quantity": "40", "serving_size": "1 bar (40 g)", "nutriments": {"energy-kcal_100g": "120"}}
        ]});
        let f = parse_hits(&v);
        assert_eq!(f.len(), 3, "the hit without a name is skipped");
        assert_eq!(f[0].brands, "Chobani");
        assert!((f[0].per100.calories.unwrap() - 52.9).abs() < 0.01);
        assert!((f[0].per100.sodium_mg.unwrap() - 38.2).abs() < 0.01);
        assert_eq!((f[0].per100.fiber_g, f[0].serving_g, f[0].serving.as_str()), (None, 0.0, ""));
        assert_eq!(f[1].per100.calories, Some(100.0));
        // a serving that already holds its weight keeps only the name; the unit list adds the "(40 g)"
        assert_eq!((f[2].brands.as_str(), f[2].per100.calories, f[2].serving_g, f[2].serving.as_str()), ("A, B", Some(120.0), 40.0, "1 bar"));
    }

    #[test]
    fn serving_text_loses_its_weight() {
        assert_eq!(strip_weight("1 bar (40 g)"), "1 bar");
        assert_eq!(strip_weight("2 cookies (30g) (about)"), "2 cookies (about)");
        assert_eq!(strip_weight("40 g"), "");
        assert_eq!(strip_weight("1 scoop"), "1 scoop");
    }

    #[test]
    fn off_dedupes_leading_zero_codes() {
        let v = json!({"hits": [{"code": "00043963", "product_name": "Soup"}, {"code": "0043963", "product_name": "Soup again"}, {"code": "7", "product_name": "Other"}]});
        let f = parse_hits(&v);
        assert_eq!(f.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Soup", "Other"]);
    }

    #[test]
    fn off_suspect_values_are_flagged() {
        let v = json!({"hits": [
            {"code": "1", "product_name": "Bad", "nutriments": {"energy-kcal_100g": 0.39, "proteins_100g": 78.8, "carbohydrates_100g": 1, "fat_100g": 1}},
            {"code": "2", "product_name": "Over", "nutriments": {"proteins_100g": 150}},
            {"code": "3", "product_name": "Fine", "nutriments": {"energy-kcal_100g": 52.9, "proteins_100g": 9.41, "carbohydrates_100g": 3.53, "fat_100g": 0}},
            {"code": "4", "product_name": "Water", "nutriments": {"energy-kcal_100g": 0, "proteins_100g": 0, "carbohydrates_100g": 0, "fat_100g": 0}},
            {"code": "5", "product_name": "Only kcal", "nutriments": {"energy-kcal_100g": 300}},
            {"code": "6", "product_name": "Too many kcal", "nutriments": {"energy-kcal_100g": 5000}},
            {"code": "7", "product_name": "Too much sodium", "nutriments": {"energy-kcal_100g": 10, "sodium_100g": 80}},
            {"code": "8", "product_name": "Negative", "nutriments": {"energy-kcal_100g": -40, "fat_100g": -3, "proteins_100g": 5}}
        ]});
        let f = parse_hits(&v);
        let s: Vec<bool> = f.iter().map(|f| f.suspect).collect();
        assert_eq!(s, [true, true, false, false, false, true, true, false]);
        assert_eq!((f[7].per100.calories, f[7].per100.fat_g), (None, None), "a negative value is unknown, not saved");
        assert!(!as_my_food(&f[7]).1, "no calories, so not ready to save");
    }

    #[test]
    fn unreachable_service_reads_as_offline() {
        // nothing listens on port 9 of this machine: no real network is touched
        assert_eq!(search_at("http://127.0.0.1:9/search", "yogurt", "me@example.com"), OffAnswer::Offline);
        // without a valid email nothing is sent, so the answer is NeedsEmail even though nothing listens
        assert_eq!(search_at("http://127.0.0.1:9/search", "yogurt", ""), OffAnswer::NeedsEmail);
        assert_eq!(search_at("http://127.0.0.1:9/search", "yogurt", "not an email"), OffAnswer::NeedsEmail);
    }

    #[test]
    fn limiter_allows_8_then_reports_wait() {
        let mut l = Limiter::default();
        for i in 0..8 {
            assert_eq!(l.allow(1000 + i), Ok(()));
        }
        assert_eq!(l.allow(1010), Err(50), "oldest send at 1000 leaves the window at 1060");
        assert_eq!(l.allow(1060), Ok(()), "the oldest has left");
        assert_eq!(l.allow(1060), Err(1), "1001 leaves at 1061");
        let mut b = Limiter::default();
        b.block(2000, 30);
        assert_eq!(b.allow(2010), Err(20));
        assert_eq!(b.allow(2030), Ok(()));
        assert_eq!([None, Some("x"), Some("0"), Some("18446744073709551615"), Some(" 30 ")].map(retry_after), [60, 60, 1, 3600, 30]);
        b.block(u64::MAX - 1, u64::MAX); // saturates instead of overflowing
        assert!(b.allow(2040).is_err());
    }

    fn product(code: &str, name: &str) -> OnlineFood {
        OnlineFood { code: code.into(), name: name.into(), brands: "Fage".into(), per100: Nutrients { calories: Some(59.0), protein_g: Some(10.0), ..Default::default() }, ..Default::default() }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rb-off-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("cache").join("off-products.json")
    }

    #[test]
    fn cache_keeps_the_most_recent_and_drops_the_rest() {
        let path = tmp("cache");
        let mut c = OffCache::open(path.clone());
        assert!(c.items().is_empty());
        c.cap = 3;
        for (i, code) in ["1", "2", "3", "4"].iter().enumerate() {
            c.remember(product(code, "Yogurt"), 100 + i as u64).unwrap();
        }
        let codes = |c: &OffCache| c.items().iter().map(|f| f.code.clone()).collect::<Vec<_>>();
        assert_eq!(codes(&c), ["2", "3", "4"]);
        // opening 2 again (with a leading zero) makes it the newest and keeps one entry and its first fetched time
        c.remember(product("02", "Yogurt"), 200).unwrap();
        c.remember(product("5", "Yogurt"), 201).unwrap();
        assert_eq!(codes(&c), ["02", "4", "5"].map(String::from));
        assert_eq!(c.items()[0].fetched, 101);
        let back = OffCache::open(path.clone());
        assert_eq!(back.items(), c.items());
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn damaged_cache_is_deleted_not_quarantined() {
        let path = tmp("damaged");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"items": ["#).unwrap();
        let c = OffCache::open(path.clone());
        assert!(c.items().is_empty());
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(path.parent().unwrap()).unwrap().count(), 0, "no damaged copy is made");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn online_food_becomes_a_my_food_draft() {
        let (f, ready) = as_my_food(&product("0123", "Greek Yogurt"));
        assert_eq!((f.name.as_str(), f.serving.as_str(), f.serving_g, f.source.as_str(), ready), ("Fage Greek Yogurt", "100 g", 100.0, "Open Food Facts, product 0123", true));
        assert_eq!(f.per_serving, product("0123", "x").per100);
        let mut with_serving = product("9", "Bar");
        (with_serving.serving, with_serving.serving_g) = ("1 bar".into(), 40.0);
        let (f, _) = as_my_food(&with_serving);
        assert_eq!((f.serving.as_str(), f.serving_g), ("1 bar", 40.0));
        assert!((f.per_serving.calories.unwrap() - 23.6).abs() < 0.001);
        let mut no_kcal = product("1", "Mystery");
        no_kcal.per100.calories = None;
        assert!(!as_my_food(&no_kcal).1);
        let mut odd = product("2", "Odd");
        odd.suspect = true;
        assert!(!as_my_food(&odd).1);
    }
}
