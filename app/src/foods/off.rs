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

/// What came back from one online search. Serialized, it is also what tiffin-helper's /off sends
/// the web app: `{"answer":"found","value":[…]}`, `{"answer":"limited","value":60}`, `{"answer":"slow"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "answer", content = "value", rename_all = "snake_case")]
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
    let v = match get_json(url, email, &[("q", text), ("page_size", "20"), ("langs", "en"), ("fields", FIELDS)]) {
        Ok(v) => v,
        Err(answer) => return answer,
    };
    found_or_empty(parse_hits(&v))
}

const FIELDS: &str = "code,product_name,brands,serving_size,serving_quantity,nutriments";
pub const OFF_PRODUCT_FIELDS: &str = FIELDS;
pub const OFF_PRODUCT_URL: &str = "https://world.openfoodfacts.org/api/v2/product";

/// What the lookup says about a barcode whose check digit doesn't fit (so nothing was sent).
pub const BAD_BARCODE: &str = "That barcode number doesn't add up. Check the digits against the pack.";

/// Look up one product by its barcode, naming the person's `email` in the User-Agent. Sends only the code: one request,
/// 8 s timeout, at most 2 MB read. A code that fails its check digit is refused before anything is sent.
/// (The v2 endpoint: v3 is documented as still changing, and both share the 100-reads-a-minute limit.)
pub fn product(code: &str, email: &str) -> Result<OffAnswer, String> {
    product_at(OFF_PRODUCT_URL, code, email)
}

/// Turn one Open Food Facts v2 product JSON response into an [`OffAnswer`]. Shared by desktop `product()` and the web shell.
pub fn parse_product(code: &str, mut v: Value) -> OffAnswer {
    let code = code.trim();
    if v.get("status").and_then(Value::as_i64) == Some(0) || !v["product"].is_object() {
        return OffAnswer::Empty;
    }
    let p = &mut v["product"];
    p["code"] = Value::from(code);
    // a product listed without a name is still the right product for this barcode
    if p.get("product_name").and_then(Value::as_str).is_none_or(|n| n.trim().is_empty()) {
        p["product_name"] = Value::from(format!("Barcode {code}"));
    }
    found_or_empty(parse_hits(&serde_json::json!({ "hits": [p.take()] })))
}

fn product_at(base: &str, code: &str, email: &str) -> Result<OffAnswer, String> {
    let code = code.trim();
    if !gtin_ok(code) {
        return Err(BAD_BARCODE.into());
    }
    let v = match get_json(&format!("{base}/{code}.json"), email, &[("fields", FIELDS)]) {
        Ok(v) => v,
        Err(answer) => return Ok(answer),
    };
    Ok(parse_product(code, v))
}

fn found_or_empty(hits: Vec<OnlineFood>) -> OffAnswer {
    if hits.is_empty() {
        OffAnswer::Empty
    } else {
        OffAnswer::Found(hits)
    }
}

/// One GET to Open Food Facts with the contact User-Agent and nothing else, no redirects (the host is fixed), 8 s, 2 MB.
/// Every way it can go wrong is an answer: not there (404), offline, slow, or told to wait.
fn get_json(url: &str, email: &str, query: &[(&str, &str)]) -> Result<Value, OffAnswer> {
    let agent_name = user_agent(email).ok_or(OffAnswer::NeedsEmail)?;
    let agent = crate::importer::secure_agent().map_err(|_| OffAnswer::Slow)?.timeout(Duration::from_secs(8)).redirects(0).build();
    let mut req = agent.get(url).set("User-Agent", &agent_name);
    for (k, v) in query {
        req = req.query(k, v);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(429 | 503, r)) => return Err(OffAnswer::Limited(retry_after(r.header("Retry-After")))),
        Err(ureq::Error::Status(404, _)) => return Err(OffAnswer::Empty),
        Err(ureq::Error::Status(..)) => return Err(OffAnswer::Slow),
        Err(ureq::Error::Transport(t)) => {
            return Err(if matches!(t.kind(), ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed) { OffAnswer::Offline } else { OffAnswer::Slow });
        }
    };
    if resp.status() != 200 {
        return Err(OffAnswer::Slow);
    }
    let mut body = Vec::new();
    resp.into_reader().take(2 * 1024 * 1024).read_to_end(&mut body).map_err(|_| OffAnswer::Slow)?;
    serde_json::from_slice::<Value>(&body).map_err(|_| OffAnswer::Slow)
}

/// GS1 check digit (EAN-8, UPC-A, EAN-13): weights 3, 1, 3… from the right, the check digit left out.
pub fn gtin_ok(code: &str) -> bool {
    if !matches!(code.len(), 8 | 12 | 13) || !code.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let d: Vec<u32> = code.bytes().map(|b| (b - b'0') as u32).collect();
    let (body, check) = d.split_at(d.len() - 1);
    let sum: u32 = body.iter().rev().enumerate().map(|(i, &x)| if i % 2 == 0 { x * 3 } else { x }).sum();
    (10 - sum % 10) % 10 == check[0]
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
pub(crate) fn is_suspect(n: &Nutrients) -> bool {
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

/// UPC-E (8 digits with zeros squeezed out) → the 12-digit UPC-A it stands for.
pub(crate) fn upc_e_to_a(e: &str) -> Option<String> {
    if e.len() != 8 || !e.bytes().all(|b| b.is_ascii_digit()) || !matches!(&e[0..1], "0" | "1") {
        return None;
    }
    let d = &e[1..7];
    let mid = match d.as_bytes()[5] {
        b'0'..=b'2' => format!("{}{}0000{}", &d[0..2], &d[5..6], &d[2..5]),
        b'3' => format!("{}00000{}", &d[0..3], &d[3..5]),
        b'4' => format!("{}00000{}", &d[0..4], &d[4..5]),
        _ => format!("{}0000{}", &d[0..5], &d[5..6]),
    };
    Some(format!("{}{}{}", &e[0..1], mid, &e[7..8]))
}

/// What a search box query says as a barcode: `None` when it isn't one (not 8, 12 or 13 digits), else the code to look up
/// (a typed UPC-E becomes its UPC-A) or [`BAD_BARCODE`] when the digits don't add up. Nothing is sent for a bad one.
pub fn barcode_in(q: &str) -> Option<Result<String, &'static str>> {
    let q = q.trim();
    if !matches!(q.len(), 8 | 12 | 13) || !q.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // shortcut: 8 digits that are valid both as EAN-8 and as a UPC-E (about 1 in 10 UPC-Es) are taken as EAN-8; a scan knows its format, typing doesn't. Look up both if it matters.
    if gtin_ok(q) {
        return Some(Ok(q.to_string()));
    }
    Some(upc_e_to_a(q).filter(|a| gtin_ok(a)).ok_or(BAD_BARCODE))
}

/// One product however its code is written: a UPC-A and the same code as EAN-13 ("0" in front) match.
pub(crate) fn same_code(a: &str, b: &str) -> bool {
    !code_key(a).is_empty() && code_key(a) == code_key(b)
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

    /// The kept product with this barcode, so a product scanned before is found offline. Leading zeros don't matter:
    /// a UPC-A and the same code as EAN-13 ("0" in front) are one product.
    pub fn get(&self, code: &str) -> Option<&OnlineFood> {
        self.items.iter().find(|f| same_code(code, &f.code))
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
    let food = MyFood { name, serving, serving_g, per_serving: f.per100.scaled(serving_g / 100.0), source: format!("Open Food Facts, product {}", f.code), barcode: if gtin_ok(f.code.trim()) { f.code.trim().to_string() } else { String::new() }, ..Default::default() };
    (food, f.per100.calories.is_some() && !f.suspect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// PRIV-02: an online search carries the typed text, the fixed search fields and the contact User-Agent, and nothing else.
    #[test]
    #[cfg(feature = "native")] // talks to a local server through the real client
    fn a_search_sends_only_the_text_and_the_contact() {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/search", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut lines = vec![];
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            loop {
                let mut l = String::new();
                if reader.read_line(&mut l).unwrap() == 0 || l == "\r\n" {
                    break;
                }
                lines.push(l.trim_end().to_string());
            }
            conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n{\"hits\":[]}").unwrap();
            lines
        });
        assert_eq!(search_at(&url, "oat milk", "me@example.com"), OffAnswer::Empty);
        let lines = server.join().unwrap();
        assert_eq!(lines[0], "GET /search?q=oat+milk&page_size=20&langs=en&fields=code%2Cproduct_name%2Cbrands%2Cserving_size%2Cserving_quantity%2Cnutriments HTTP/1.1");
        let mut headers: Vec<String> = lines[1..].iter().map(|h| h.split(':').next().unwrap().to_ascii_lowercase()).collect();
        headers.sort();
        assert_eq!(headers, ["accept", "accept-encoding", "host", "user-agent"], "no cookie, referrer or other header: {lines:?}");
        assert!(lines.contains(&format!("User-Agent: Tiffin/{} (me@example.com)", APP_VERSION)));
    }

    /// A one-request HTTP server on this machine: answers `reply` and hands back the request lines it received.
    #[cfg(feature = "native")]
    fn serve(reply: String) -> (String, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/api/v2/product", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut lines = vec![];
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            loop {
                let mut l = String::new();
                if reader.read_line(&mut l).unwrap() == 0 || l == "\r\n" {
                    break;
                }
                lines.push(l.trim_end().to_string());
            }
            conn.write_all(reply.as_bytes()).unwrap();
            lines
        });
        (base, server)
    }

    #[cfg(feature = "native")]
    fn http(status: &str, body: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}", body.len())
    }

    /// PRIV-02: a barcode lookup carries the code, the fixed fields and the contact User-Agent, and nothing else.
    #[test]
    #[cfg(feature = "native")] // talks to a local server through the real client
    fn a_barcode_lookup_sends_only_the_code_and_the_contact() {
        let body = json!({"status": 1, "product": {"product_name": "Extra Firm Tofu", "brands": "Sunrise", "serving_size": "85 g", "serving_quantity": 85,
            "nutriments": {"energy-kcal_100g": 141, "proteins_100g": 15.3, "fat_100g": 8.2, "carbohydrates_100g": 2.4, "sodium_100g": 0.012}}}).to_string();
        let (base, server) = serve(http("200 OK", &body));
        let answer = product_at(&base, " 196633823717 ", "me@example.com").unwrap();
        let lines = server.join().unwrap();
        assert_eq!(lines[0], "GET /api/v2/product/196633823717.json?fields=code%2Cproduct_name%2Cbrands%2Cserving_size%2Cserving_quantity%2Cnutriments HTTP/1.1");
        let mut headers: Vec<String> = lines[1..].iter().map(|h| h.split(':').next().unwrap().to_ascii_lowercase()).collect();
        headers.sort();
        assert_eq!(headers, ["accept", "accept-encoding", "host", "user-agent"], "no cookie, referrer or other header: {lines:?}");
        assert!(lines.contains(&format!("User-Agent: Tiffin/{} (me@example.com)", APP_VERSION)));
        let OffAnswer::Found(hits) = answer else { panic!("{answer:?}") };
        let f = &hits[0];
        assert_eq!((f.code.as_str(), f.name.as_str(), f.brands.as_str(), f.serving_g), ("196633823717", "Extra Firm Tofu", "Sunrise", 85.0));
        assert_eq!((f.per100.calories, f.per100.protein_g, f.per100.sodium_mg, f.per100.fiber_g), (Some(141.0), Some(15.3), Some(12.0), None));
        assert!(!f.suspect);
    }

    #[test]
    #[cfg(feature = "native")] // talks to a local server through the real client
    fn barcode_answers_for_every_outcome() {
        // impossible values come back flagged, and a nameless product still comes back for its barcode
        let body = json!({"status": 1, "product": {"nutriments": {"energy-kcal_100g": 50, "proteins_100g": 130}}}).to_string();
        let (base, server) = serve(http("200 OK", &body));
        let OffAnswer::Found(hits) = product_at(&base, "4006381333931", "me@example.com").unwrap() else { panic!() };
        server.join().unwrap();
        assert!(hits[0].suspect);
        assert_eq!(hits[0].name, "Barcode 4006381333931");
        // not in the database: "status 0" or a 404
        let (base, server) = serve(http("200 OK", r#"{"status":0,"status_verbose":"product not found"}"#));
        assert_eq!(product_at(&base, "4006381333931", "me@example.com"), Ok(OffAnswer::Empty));
        server.join().unwrap();
        let (base, server) = serve(http("404 Not Found", "{}"));
        assert_eq!(product_at(&base, "4006381333931", "me@example.com"), Ok(OffAnswer::Empty));
        server.join().unwrap();
        // told to wait
        let (base, server) = serve("HTTP/1.1 429 Too Many Requests\r\nRetry-After: 30\r\nContent-Length: 0\r\n\r\n".into());
        assert_eq!(product_at(&base, "4006381333931", "me@example.com"), Ok(OffAnswer::Limited(30)));
        server.join().unwrap();
        let (base, server) = serve("HTTP/1.1 503 Service Unavailable\r\nRetry-After: 5\r\nContent-Length: 0\r\n\r\n".into());
        assert_eq!(product_at(&base, "4006381333931", "me@example.com"), Ok(OffAnswer::Limited(5)));
        server.join().unwrap();
        // an odd answer: a product that isn't an object is "not there", a body that isn't JSON is "slow"
        let (base, server) = serve(http("200 OK", r#"{"status":1,"product":[1,2]}"#));
        assert_eq!(product_at(&base, "4006381333931", "me@example.com"), Ok(OffAnswer::Empty));
        server.join().unwrap();
        let (base, server) = serve(http("200 OK", "<html>busy</html>"));
        assert_eq!(product_at(&base, "4006381333931", "me@example.com"), Ok(OffAnswer::Slow));
        server.join().unwrap();
        // the search shares the same handling: a 404 there is "no results"
        let (base, server) = serve(http("404 Not Found", "{}"));
        assert_eq!(search_at(&base, "oat milk", "me@example.com"), OffAnswer::Empty);
        server.join().unwrap();
        // nothing listens on port 9: a wrong check digit or a missing email is refused before any request is tried
        assert_eq!(product_at("http://127.0.0.1:9/p", "4006381333930", "me@example.com"), Err(BAD_BARCODE.to_string()));
        assert_eq!(product_at("http://127.0.0.1:9/p", "../../search?q=x", "me@example.com"), Err(BAD_BARCODE.to_string()));
        assert_eq!(product_at("http://127.0.0.1:9/p", "4006381333931", ""), Ok(OffAnswer::NeedsEmail));
        assert_eq!(product_at("http://127.0.0.1:9/p", "4006381333931", "me@example.com"), Ok(OffAnswer::Offline));
    }

    #[test]
    fn an_answer_crosses_the_wire_unchanged() {
        let food = OnlineFood { code: "3017620422003".into(), name: "Nutella".into(), ..Default::default() };
        for a in [OffAnswer::Found(vec![food]), OffAnswer::Empty, OffAnswer::Offline, OffAnswer::Slow, OffAnswer::Limited(60), OffAnswer::NeedsEmail] {
            let wire = serde_json::to_string(&a).unwrap();
            assert_eq!(serde_json::from_str::<OffAnswer>(&wire).unwrap(), a, "{wire}");
        }
        // the shape the web app reads
        assert_eq!(serde_json::to_string(&OffAnswer::Limited(60)).unwrap(), r#"{"answer":"limited","value":60}"#);
        assert_eq!(serde_json::to_string(&OffAnswer::NeedsEmail).unwrap(), r#"{"answer":"needs_email"}"#);
    }

    #[test]
    fn check_digits() {
        for good in ["815074022915", "196633823717", "4006381333931", "96385074", "042100005264"] {
            assert!(gtin_ok(good), "{good}");
        }
        for bad in ["815074022914", "4006381333930", "12345", "", "81507402291x", "12345678901231"] {
            assert!(!gtin_ok(bad), "{bad}");
        }
    }

    #[test]
    fn a_query_that_is_a_barcode() {
        assert_eq!(barcode_in(" 196633823717 "), Some(Ok("196633823717".into())));
        assert_eq!(barcode_in("96385074"), Some(Ok("96385074".into())));
        // a typed UPC-E (not a valid EAN-8) is looked up as its UPC-A
        assert_eq!(barcode_in("04252614"), Some(Ok("042100005264".into())));
        assert_eq!(barcode_in("196633823718"), Some(Err(BAD_BARCODE)));
        // valid both as EAN-8 and as UPC-E (012345000065): EAN-8 wins
        assert!(gtin_ok("01234565") && upc_e_to_a("01234565").is_some_and(|a| gtin_ok(&a)));
        assert_eq!(barcode_in("01234565"), Some(Ok("01234565".into())));
        // a date that happens to add up is looked up as a barcode, never text-searched
        assert_eq!(barcode_in("20261122"), Some(Ok("20261122".into())));
        assert_eq!(barcode_in("20261123"), Some(Err(BAD_BARCODE)));
        for text in ["greek yogurt", "1966338237", "19663382371x", "", "1 cup"] {
            assert_eq!(barcode_in(text), None, "{text}");
        }
    }

    #[test]
    fn a_product_scanned_before_is_found_offline() {
        let path = tmp("get");
        let mut cache = OffCache::open(path.clone());
        cache.remember(product("0196633823717", "Tofu"), 10).unwrap();
        let reopened = OffCache::open(path.clone());
        let _ = std::fs::remove_file(&path);
        assert_eq!(reopened.get("196633823717").map(|f| f.name.as_str()), Some("Tofu"));
        assert_eq!(reopened.get("00196633823717").map(|f| f.name.as_str()), Some("Tofu"));
        assert!(reopened.get("4006381333931").is_none());
        assert!(reopened.get("000").is_none());
    }

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
    #[cfg(feature = "native")] // talks to a local server through the real client
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
        let d = crate::scratch_dir().join(format!("rb-off-{}-{}", name, crate::test_pid()));
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
        // the product's barcode goes with it, so scanning it again finds the saved food; a code that isn't one is left out
        assert_eq!(as_my_food(&product("196633823717", "Tofu")).0.barcode, "196633823717");
        assert_eq!(as_my_food(&product("0123", "Greek Yogurt")).0.barcode, "");
        let mut odd = product("2", "Odd");
        odd.suspect = true;
        assert!(!as_my_food(&odd).1);
    }

    #[test]
    fn parse_product_handles_missing_unnamed_and_valid_responses() {
        assert_eq!(parse_product("4006381333931", json!({ "status": 0 })), OffAnswer::Empty);
        assert_eq!(parse_product("4006381333931", json!({ "status": 1 })), OffAnswer::Empty);
        let OffAnswer::Found(unnamed) = parse_product("4006381333931", json!({ "status": 1, "product": { "product_name": "  " } })) else {
            panic!("expected Found for unnamed product");
        };
        assert_eq!(unnamed.len(), 1);
        assert_eq!(unnamed[0].code, "4006381333931");
        assert_eq!(unnamed[0].name, "Barcode 4006381333931");
        let OffAnswer::Found(named) = parse_product(
            " 4006381333931 ",
            json!({ "status": 1, "product": { "product_name": "Oat Milk", "brands": "Oatly", "nutriments": { "energy-kcal_100g": 45 } } }),
        ) else {
            panic!("expected Found for named product");
        };
        assert_eq!(named[0].name, "Oat Milk");
        assert_eq!(named[0].brands, "Oatly");
        assert_eq!(named[0].per100.calories, Some(45.0));
    }
}
