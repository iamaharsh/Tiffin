//! Web & text importer.
//!
//! Order of methods (most reliable first), all run locally:
//!  1. schema.org Recipe data embedded in the page (JSON-LD)
//!  2. the tested site rule for the page's domain (rules.rs)
//!  3. common recipe-plugin layouts and schema.org microdata
//!  4. general reading: the list under an "Ingredients" heading and under "Directions/Method"
//! Results are cross-checked; disagreements and low-confidence lines are flagged for review.

use crate::ingredients::{self, check_step, iso_duration, parse_ingredient, split_steps};
use crate::model::{Library, Nutrients, Recipe};
use crate::rules::{self, SiteRule};
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Draft {
    pub title: String,
    pub ingredients: Vec<String>,
    pub steps: Vec<String>,
    pub servings: String,
    pub prep: String,
    pub cook: String,
    pub total: String,
    pub description: String,
    pub video_url: String,
    pub categories: Vec<String>,
    pub nutrition: Option<Nutrients>,
    pub cuisines: Vec<String>,
    pub image_url: String,
}

#[derive(Clone, Debug, Default)]
pub struct ImportResult {
    pub recipe: Recipe,
    pub method: String,
    pub tried: Vec<(String, String)>,
    pub field_issues: Vec<String>,
    pub rule_domain: Option<String>,
    pub rule_worked: Option<bool>,
    pub image_url: String,
}

// ---------------- fetching ----------------

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

const PAGE_MAX_BYTES: usize = 10 * 1024 * 1024;
const PICTURE_MAX_BYTES: usize = 15 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

/// Refusals for links the importer won't open. Full sentences: the UI shows them as they are.
pub const ONLY_WEB: &str = "Tiffin only imports from web pages (http or https).";
pub const DASH_LINK: &str = "That link starts with a dash, so it isn't a web address.";
pub const USER_IN_LINK: &str = "Links with a user name or password in them aren't imported.";
pub const TOO_MANY_REDIRECTS: &str = "The page redirected more than 5 times, so Tiffin stopped.";
pub const NOT_LOCAL: &str = "Tiffin imports only from web pages on the internet, not from this computer or your home network.";
const NOT_AN_ADDRESS: &str = "That isn't a web address.";
const PAGE_TOO_BIG: &str = "The page is larger than 10 MB, so it wasn't imported.";
const PICTURE_TOO_BIG: &str = "The picture is larger than 15 MB, so it wasn't downloaded.";
const PICTURE_FAILED: &str = "Couldn't download the picture";

/// Pick the picture address out of schema.org "image" (text, list, or ImageObject).
fn image_url_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Array(a) => a.iter().map(image_url_of).find(|s| !s.is_empty()).unwrap_or_default(),
        Value::Object(o) => o.get("url").or(o.get("contentUrl")).map(image_url_of).unwrap_or_default(),
        _ => String::new(),
    }
}

/// File a recipe under "Cuisines/…": the page's own cuisine first, otherwise a guess from its
/// name and ingredients.
pub fn file_under_cuisine(r: &mut Recipe, hints: &[String]) {
    if r.categories.iter().any(|c| c.starts_with("Cuisines/")) {
        return;
    }
    let from_page = hints.iter().find_map(|h| {
        let h = h.trim();
        crate::model::CUISINES.iter().find(|c| c.eq_ignore_ascii_case(h)).map(|c| c.to_string()).or_else(|| {
            // an unlisted cuisine named by the page becomes a new sub-category
            if !h.is_empty() && h.len() < 30 && h.chars().all(|c| c.is_alphabetic() || c == ' ') {
                let mut t = h.to_string();
                if let Some(f) = t.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                Some(t)
            } else {
                None
            }
        })
    });
    let c = from_page.or_else(|| crate::model::guess_cuisine(&r.name, &r.ingredients).map(|s| s.to_string()));
    if let Some(c) = c {
        r.categories.push(format!("{}/{}", crate::model::CUISINES_PARENT, c));
    }
}

/// The scheme a link starts with ("file" in "file:///x"). "localhost:8080/x" and "example.com:443/a"
/// are host and port, not a scheme, so a digit after the colon means no scheme.
fn scheme_of(s: &str) -> Option<&str> {
    let end = s.find(':')?;
    let name = &s[..end];
    let mut chars = name.chars();
    let named = chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
    let port = s[end + 1..].chars().next().is_some_and(|c| c.is_ascii_digit());
    (named && !port).then_some(name)
}

/// The one place an imported or typed link is judged: web pages only, no leading dash, no user name
/// or password. A bare "example.com/soup" gets https://. Returns the link in its normalised form.
pub fn check_web_url(input: &str) -> Result<String, String> {
    let s = input.trim();
    if s.starts_with('-') {
        return Err(DASH_LINK.into());
    }
    let s = match scheme_of(s) {
        Some(sc) if !sc.eq_ignore_ascii_case("http") && !sc.eq_ignore_ascii_case("https") => {
            let slashes = if s[sc.len() + 1..].starts_with("//") { "//" } else { "" };
            return Err(format!("{ONLY_WEB} This link starts with {sc}:{slashes}."));
        }
        Some(_) => s.to_string(),
        None => format!("https://{s}"),
    };
    // ureq's own parser, so the check and the request can never read a link two ways
    let req = ureq::get(&s);
    let parsed = req.request_url().map_err(|_| NOT_AN_ADDRESS.to_string())?;
    let url = parsed.as_url();
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{ONLY_WEB} This link starts with {}:.", url.scheme()));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(USER_IN_LINK.into());
    }
    Ok(url.to_string())
}

/// Where a redirect leads, judged exactly like a typed link.
fn redirect_target(current: &str, location: &str) -> Result<String, String> {
    let base = ureq::get(current).request_url().map_err(|_| NOT_AN_ADDRESS.to_string())?;
    let next = base.as_url().join(location.trim()).map_err(|_| NOT_AN_ADDRESS.to_string())?;
    check_web_url(next.as_str())
}

/// This computer or the home network: loopback, private, link-local, shared (100.64/10) and
/// unspecified addresses, and IPv6 addresses that wrap an IPv4 one.
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => {
            let o = a.octets();
            a.is_loopback() || a.is_private() || a.is_link_local() || a.is_unspecified() || a.is_broadcast() || o[0] == 0 || (o[0] == 100 && o[1] & 0xc0 == 64)
        }
        IpAddr::V6(a) => {
            let first = a.segments()[0];
            a.is_loopback() || a.is_unspecified() || first & 0xffc0 == 0xfe80 || first & 0xfe00 == 0xfc00 || a.to_ipv4().is_some_and(|v| is_local(IpAddr::V4(v)))
        }
    }
}

/// Look the host up once and refuse it when any answer is local. The caller connects to the
/// returned address and nothing else, so a name can't pass here and then resolve elsewhere.
fn vet_host(host: &str, port: u16) -> Result<SocketAddr, String> {
    let all: Vec<SocketAddr> = format!("{host}:{port}").to_socket_addrs().map_err(|_| format!("Couldn't find the site {host}."))?.collect();
    if all.iter().any(|a| is_local(a.ip())) {
        return Err(NOT_LOCAL.into());
    }
    all.iter().find(|a| a.is_ipv4()).or(all.first()).copied().ok_or_else(|| format!("Couldn't find the site {host}."))
}

/// One answer: the body, or the next address to try.
enum Hop {
    Body(Vec<u8>),
    Redirect(String),
}

fn too_big(page: bool) -> &'static str {
    if page { PAGE_TOO_BIG } else { PICTURE_TOO_BIG }
}

/// A name part that two calls never share (process id plus a nanosecond stamp).
pub(crate) fn unique_stamp() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos)
}

/// The only download path (pages and pictures): checks the link, then follows redirects one hop
/// at a time, checking every target again. Built-in client first, then the system's curl, which
/// some sites accept when they refuse other clients.
fn download(url: &str, max: usize, page: bool) -> Result<Vec<u8>, String> {
    let mut url = check_web_url(url)?;
    for _ in 0..=MAX_REDIRECTS {
        let (host, port) = host_port(&url)?;
        let addr = vet_host(&host, port)?;
        let hop = match ureq_hop(&url, max, page, addr) {
            Ok(h) => h,
            Err(e) if e == too_big(page) => return Err(e),
            Err(first) => curl_hop(&url, max, page, &host, port, addr.ip()).map_err(|_| if page { first } else { PICTURE_FAILED.to_string() })?,
        };
        match hop {
            Hop::Body(b) => return Ok(b),
            Hop::Redirect(location) => url = redirect_target(&url, &location)?,
        }
    }
    Err(TOO_MANY_REDIRECTS.into())
}

fn host_port(url: &str) -> Result<(String, u16), String> {
    let req = ureq::get(url);
    let parsed = req.request_url().map_err(|_| NOT_AN_ADDRESS.to_string())?;
    let u = parsed.as_url();
    Ok((u.host_str().unwrap_or_default().to_string(), u.port_or_known_default().unwrap_or(80)))
}

fn ureq_hop(url: &str, max: usize, page: bool, addr: SocketAddr) -> Result<Hop, String> {
    use std::io::Read;
    let tls = native_tls::TlsConnector::new().map_err(|e| format!("Secure connection unavailable: {}", e))?;
    // redirects(0): a 3xx comes back as an answer, and download() checks where it leads.
    // The resolver hands back only the address vet_host checked, so ureq never looks the name up again.
    let agent = ureq::AgentBuilder::new().tls_connector(std::sync::Arc::new(tls)).timeout(std::time::Duration::from_secs(25)).redirects(0).resolver(move |_: &str| Ok(vec![addr])).build();
    // A standard browser identity: several big recipe sites refuse requests that name an app.
    let mut req = agent.get(url).set("User-Agent", BROWSER_UA);
    if page {
        req = req.set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8").set("Accept-Language", "en-US,en;q=0.9");
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(401 | 402 | 403 | 429, _) => {
            "This site blocks recipe apps from downloading its pages. Open it in your browser, then either copy the recipe and use \"Paste recipe text\", or save the page (Ctrl+S) into the Recipe Inbox folder.".to_string()
        }
        ureq::Error::Status(code, _) => format!("The site answered with error {}", code),
        other => format!("Couldn't reach the page: {}", other),
    })?;
    if (300..400).contains(&resp.status()) {
        return resp.header("Location").map(|l| Hop::Redirect(l.to_string())).ok_or_else(|| "The site sent a redirect without an address.".to_string());
    }
    let mut buf = Vec::new();
    resp.into_reader().take(max as u64 + 1).read_to_end(&mut buf).map_err(|e| format!("Couldn't read the page: {}", e))?;
    if buf.len() > max {
        return Err(too_big(page).into());
    }
    if buf.is_empty() {
        return Err("The page was empty.".into());
    }
    Ok(Hop::Body(buf))
}

/// Arguments for one curl hop. The link comes last, after "--", so it can never be read as an
/// option; "-q" first ignores the user's curlrc; only http(s), and curl never follows a redirect
/// itself (download() does, and checks each one).
fn curl_args(url: &str, max: usize, page: bool, out: &Path, host: &str, port: u16, ip: IpAddr) -> Vec<String> {
    let mut a: Vec<String> = ["-q", "-sS", "--proto", "=http,https", "--proto-redir", "=http,https", "--max-redirs", "0", "--max-time", "25", "--max-filesize"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    a.push(max.to_string());
    a.extend(["--noproxy", "*", "-A", BROWSER_UA].iter().map(|s| s.to_string())); // ureq uses no proxy either
    if page {
        a.extend(["--compressed", "-H", "Accept-Language: en-US,en;q=0.9"].iter().map(|s| s.to_string()));
    }
    if host.trim_matches(['[', ']']).parse::<IpAddr>().is_err() {
        // connect to the address that was checked, not to a fresh lookup of the name
        let ip = if ip.is_ipv6() { format!("[{ip}]") } else { ip.to_string() };
        a.extend(["--resolve".to_string(), format!("{host}:{port}:{ip}")]);
    }
    a.extend(["-o".to_string(), out.to_string_lossy().to_string(), "-w".into(), "%{http_code} %{redirect_url}".into(), "--".into(), url.to_string()]);
    a
}

fn curl_hop(url: &str, max: usize, page: bool, host: &str, port: u16, ip: IpAddr) -> Result<Hop, String> {
    let out = std::env::temp_dir().join(format!("recipebox-download-{}.tmp", unique_stamp()));
    let res = run_curl(url, max, page, &out, host, port, ip);
    let _ = std::fs::remove_file(&out);
    res
}

fn run_curl(url: &str, max: usize, page: bool, out: &Path, host: &str, port: u16, ip: IpAddr) -> Result<Hop, String> {
    // Windows ships curl in System32; prefer it over whatever else is on the PATH.
    let system = std::env::var_os("SystemRoot").map(|r| Path::new(&r).join("System32").join("curl.exe")).filter(|p| p.is_file());
    let mut cmd = match system {
        Some(p) => std::process::Command::new(p),
        None => std::process::Command::new("curl"),
    };
    cmd.args(curl_args(url, max, page, out, host, port, ip));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let o = cmd.output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
        // 63: curl's "maximum file size exceeded"
        return Err(if o.status.code() == Some(63) { too_big(page).into() } else if err.is_empty() { "curl failed".into() } else { err });
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let (code, redirect) = text.trim().split_once(' ').unwrap_or((text.trim(), ""));
    let code: u16 = code.parse().unwrap_or(0);
    match code {
        300..=399 if !redirect.is_empty() => Ok(Hop::Redirect(redirect.to_string())),
        200..=299 => {
            if std::fs::metadata(out).map_err(|e| e.to_string())?.len() > max as u64 {
                return Err(too_big(page).into());
            }
            let body = std::fs::read(out).map_err(|e| e.to_string())?;
            if body.is_empty() { Err("The page was empty.".into()) } else { Ok(Hop::Body(body)) }
        }
        _ => Err(format!("The site answered with error {}", code)),
    }
}

/// Download a page: http(s) only, every redirect re-checked, at most 10 MB.
pub fn fetch(url: &str) -> Result<String, String> {
    download(url, PAGE_MAX_BYTES, true).map(|b| String::from_utf8_lossy(&b).to_string())
}

/// Download a file (pictures) the same way; at most 15 MB.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, String> {
    download(url, PICTURE_MAX_BYTES, false)
}


// ---------------- text helpers ----------------

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        if let Some(end) = after[..after.len().min(12)].find(';') {
            let ent = &after[1..end];
            let rep = match ent {
                "amp" => Some("&".to_string()),
                "lt" => Some("<".into()),
                "gt" => Some(">".into()),
                "quot" => Some("\"".into()),
                "apos" | "#39" => Some("'".into()),
                "nbsp" => Some(" ".into()),
                "frac12" => Some("½".into()),
                "frac14" => Some("¼".into()),
                "frac34" => Some("¾".into()),
                "deg" => Some("°".into()),
                "ndash" => Some("-".into()),
                "mdash" => Some("-".into()),
                "rsquo" | "lsquo" => Some("'".into()),
                "rdquo" | "ldquo" => Some("\"".into()),
                "hellip" => Some("...".into()),
                e if e.starts_with("#x") || e.starts_with("#X") => {
                    u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32).map(|c| c.to_string())
                }
                e if e.starts_with('#') => e[1..].parse::<u32>().ok().and_then(char::from_u32).map(|c| c.to_string()),
                _ => None,
            };
            if let Some(r) = rep {
                out.push_str(&r);
                rest = &after[end + 1..];
                continue;
            }
        }
        out.push('&');
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

pub fn clean_text(s: &str) -> String {
    let t = decode_entities(&strip_tags(&decode_entities(s)));
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn el_text(el: &ElementRef) -> String {
    let t: Vec<&str> = el.text().collect();
    clean_text(&t.join(" "))
}

fn select_texts(doc: &Html, selectors: &[String]) -> Vec<String> {
    for s in selectors {
        let Ok(sel) = Selector::parse(s) else { continue };
        let items: Vec<String> = doc.select(&sel).map(|e| el_text(&e)).filter(|t| !t.is_empty()).collect();
        if !items.is_empty() {
            return items;
        }
    }
    vec![]
}

fn select_texts_str(doc: &Html, selector: &str) -> Vec<String> {
    select_texts(doc, &[selector.to_string()])
}

fn meta(doc: &Html, prop: &str) -> Option<String> {
    let sel = Selector::parse(&format!("meta[property='{0}'], meta[name='{0}']", prop)).ok()?;
    doc.select(&sel).next().and_then(|e| e.value().attr("content")).map(clean_text).filter(|s| !s.is_empty())
}

fn page_title(doc: &Html) -> String {
    if let Some(t) = meta(doc, "og:title") {
        return t;
    }
    let h1 = select_texts_str(doc, "h1");
    if let Some(t) = h1.first() {
        return t.clone();
    }
    select_texts_str(doc, "title").first().cloned().unwrap_or_default()
}

// ---------------- 1. JSON-LD ----------------

fn is_recipe(v: &Value) -> bool {
    match v.get("@type") {
        Some(Value::String(s)) => s.eq_ignore_ascii_case("recipe") || s.ends_with("/Recipe"),
        Some(Value::Array(a)) => a.iter().any(|x| x.as_str().map(|s| s.eq_ignore_ascii_case("recipe")).unwrap_or(false)),
        _ => false,
    }
}

fn find_recipe(v: &Value) -> Option<&Value> {
    if is_recipe(v) {
        return Some(v);
    }
    match v {
        Value::Array(a) => a.iter().find_map(find_recipe),
        Value::Object(o) => {
            if let Some(g) = o.get("@graph") {
                if let Some(r) = find_recipe(g) {
                    return Some(r);
                }
            }
            if let Some(m) = o.get("mainEntity") {
                return find_recipe(m);
            }
            None
        }
        _ => None,
    }
}

fn val_text(v: &Value) -> String {
    match v {
        Value::String(s) => clean_text(s),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(val_text).filter(|s| !s.is_empty()).collect::<Vec<_>>().first().cloned().unwrap_or_default(),
        Value::Object(o) => o.get("name").or(o.get("text")).map(val_text).unwrap_or_default(),
        _ => String::new(),
    }
}

fn collect_instructions(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => {
            // Some sites put all steps in one string, sometimes with <p> or <li> markup.
            let marked = s.replace("</p>", "\n").replace("</li>", "\n").replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
            for st in split_steps(&decode_entities(&strip_tags(&decode_entities(&marked)))) {
                let c = clean_text(&st);
                if !c.is_empty() {
                    out.push(c);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_instructions(x, out)),
        Value::Object(o) => {
            let ty = o.get("@type").and_then(|t| t.as_str()).unwrap_or("");
            if ty == "HowToSection" {
                if let Some(n) = o.get("name").map(val_text).filter(|n| !n.is_empty()) {
                    out.push(format!("{}:", n.trim_end_matches(':')));
                }
                if let Some(items) = o.get("itemListElement") {
                    collect_instructions(items, out);
                }
            } else if let Some(t) = o.get("text") {
                collect_instructions(t, out);
            } else if let Some(items) = o.get("itemListElement") {
                collect_instructions(items, out);
            } else if let Some(n) = o.get("name") {
                collect_instructions(n, out);
            }
        }
        _ => {}
    }
}

/// The number a page value starts with; none when it has no leading number (unknown, never 0).
fn num_prefix(s: &str) -> Option<f32> {
    let n: String = s.trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    n.parse::<f32>().ok().filter(|v| v.is_finite())
}

pub fn extract_jsonld(doc: &Html) -> Option<Draft> {
    let sel = Selector::parse("script[type='application/ld+json']").ok()?;
    for s in doc.select(&sel) {
        let raw: String = s.text().collect();
        let Ok(v) = serde_json::from_str::<Value>(raw.trim()) else { continue };
        let Some(r) = find_recipe(&v) else { continue };
        let mut d = Draft { title: r.get("name").map(val_text).unwrap_or_default(), ..Default::default() };
        if let Some(Value::Array(a)) = r.get("recipeIngredient").or(r.get("ingredients")) {
            d.ingredients = a.iter().map(val_text).filter(|s| !s.is_empty()).collect();
        }
        if let Some(i) = r.get("recipeInstructions") {
            collect_instructions(i, &mut d.steps);
        }
        d.servings = r.get("recipeYield").map(val_text).unwrap_or_default();
        if !d.servings.is_empty() && d.servings.chars().all(|c| c.is_ascii_digit()) {
            d.servings = format!("{} servings", d.servings);
        }
        d.prep = r.get("prepTime").map(|v| iso_duration(&val_text(v))).unwrap_or_default();
        d.cook = r.get("cookTime").map(|v| iso_duration(&val_text(v))).unwrap_or_default();
        d.total = r.get("totalTime").map(|v| iso_duration(&val_text(v))).unwrap_or_default();
        d.description = r.get("description").map(val_text).unwrap_or_default();
        match r.get("recipeCategory") {
            Some(Value::Array(a)) => d.categories = a.iter().map(val_text).filter(|s| !s.is_empty()).collect(),
            Some(v) => {
                let t = val_text(v);
                if !t.is_empty() {
                    d.categories = t.split(',').map(|s| s.trim().to_string()).collect();
                }
            }
            None => {}
        }
        match r.get("recipeCuisine") {
            Some(Value::Array(a)) => d.cuisines = a.iter().map(val_text).filter(|s| !s.is_empty()).collect(),
            Some(v) => d.cuisines = val_text(v).split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            None => {}
        }
        d.image_url = r.get("image").map(image_url_of).unwrap_or_default();
        if let Some(vid) = r.get("video") {
            let vid = if let Value::Array(a) = vid { a.first().unwrap_or(vid) } else { vid };
            d.video_url = vid.get("contentUrl").or(vid.get("embedUrl")).or(vid.get("url")).map(val_text).unwrap_or_default();
        }
        if let Some(n) = r.get("nutrition") {
            let g = |k: &str| n.get(k).and_then(|x| num_prefix(&val_text(x)));
            let cal = g("calories");
            if cal.is_some_and(|c| c > 0.0) {
                d.nutrition = Some(Nutrients {
                    calories: cal,
                    protein_g: g("proteinContent"),
                    fat_g: g("fatContent"),
                    carbs_g: g("carbohydrateContent"),
                    fiber_g: g("fiberContent"),
                    sugar_g: g("sugarContent"),
                    sodium_mg: g("sodiumContent"),
                });
            }
        }
        return Some(d);
    }
    None
}

// ---------------- 2/3. site rules and plugins ----------------

pub fn apply_rule_doc(doc: &Html, r: &SiteRule) -> Option<Draft> {
    let ingredients = select_texts(doc, &r.ingredients);
    let steps = select_texts(doc, &r.steps);
    if ingredients.is_empty() && steps.is_empty() {
        return None;
    }
    let title = select_texts(doc, &r.title).first().cloned().unwrap_or_else(|| page_title(doc));
    let servings = select_texts(doc, &r.servings).first().cloned().unwrap_or_default();
    Some(Draft { title, ingredients, steps, servings, ..Default::default() })
}

pub fn apply_rule_html(html: &str, r: &SiteRule) -> Option<Draft> {
    let doc = Html::parse_document(html);
    apply_rule_doc(&doc, r)
}

fn extract_plugins(doc: &Html) -> Option<(String, Draft)> {
    for (name, ing, steps) in rules::PLUGIN_RULES {
        let ingredients = select_texts_str(doc, ing);
        if ingredients.len() < 2 {
            continue;
        }
        let mut st = select_texts_str(doc, steps);
        // A single instructions block: split it into steps.
        if st.len() == 1 {
            st = split_steps(&st[0]);
        }
        return Some((name.to_string(), Draft { title: page_title(doc), ingredients, steps: st, ..Default::default() }));
    }
    None
}

// ---------------- 4. general reading ----------------

fn heading_matches(t: &str, words: &[&str]) -> bool {
    let lc = t.trim().trim_end_matches(':').to_lowercase();
    lc.len() < 40 && words.iter().any(|w| lc == *w || lc.ends_with(&format!(" {}", w)) || lc.starts_with(&format!("{} ", w)))
}

fn list_after(el: ElementRef) -> Vec<String> {
    // Walk following siblings (and the parent's following siblings) to the next list.
    let li = Selector::parse("li").unwrap();
    let mut node = Some(el);
    for _ in 0..3 {
        let Some(cur) = node else { break };
        for sib in cur.next_siblings() {
            if let Some(e) = ElementRef::wrap(sib) {
                let tag = e.value().name();
                if matches!(tag, "h1" | "h2" | "h3" | "h4") {
                    break;
                }
                let items: Vec<String> = if tag == "ul" || tag == "ol" {
                    e.select(&li).map(|x| el_text(&x)).filter(|t| !t.is_empty()).collect()
                } else {
                    e.select(&li).map(|x| el_text(&x)).filter(|t| !t.is_empty()).collect()
                };
                if !items.is_empty() {
                    return items;
                }
                if tag == "p" {
                    let t = el_text(&e);
                    if !t.is_empty() {
                        // paragraph list: lines separated by <br>
                        let html = e.inner_html().replace("<br>", "\n").replace("<br/>", "\n").replace("<br />", "\n");
                        let lines: Vec<String> = clean_lines(&html);
                        if lines.len() >= 2 {
                            return lines;
                        }
                    }
                }
            }
        }
        node = cur.parent().and_then(ElementRef::wrap);
    }
    vec![]
}

fn clean_lines(html: &str) -> Vec<String> {
    html.split('\n').map(clean_text).filter(|s| !s.is_empty()).collect()
}

fn extract_heuristic(doc: &Html) -> Option<Draft> {
    let hs = Selector::parse("h1, h2, h3, h4, h5, strong, b, p, span, div").ok()?;
    let mut ingredients = vec![];
    let mut steps = vec![];
    for h in doc.select(&hs) {
        // only short leaf-ish headings
        if h.children().count() > 3 {
            continue;
        }
        let t = el_text(&h);
        if ingredients.is_empty() && heading_matches(&t, &["ingredients", "ingredient list", "you will need", "what you need"]) {
            ingredients = list_after(h);
        } else if steps.is_empty() && heading_matches(&t, &["directions", "instructions", "method", "preparation", "steps", "how to make it"]) {
            steps = list_after(h);
            if steps.len() == 1 {
                steps = split_steps(&steps[0]);
            }
        }
        if !ingredients.is_empty() && !steps.is_empty() {
            break;
        }
    }
    if ingredients.is_empty() && steps.is_empty() {
        return None;
    }
    Some(Draft { title: page_title(doc), ingredients, steps, ..Default::default() })
}

// ---------------- assembling ----------------

fn draft_score(d: &Draft) -> f32 {
    if d.ingredients.is_empty() {
        return 0.0;
    }
    let conf: f32 = d.ingredients.iter().map(|l| parse_ingredient(l).confidence).sum::<f32>() / d.ingredients.len() as f32;
    let steps = if d.steps.is_empty() { 0.0 } else { 1.0 };
    conf * 2.0 + steps + (d.ingredients.len().min(6) as f32) * 0.1
}

fn tidy_title(t: &str, host: &str) -> String {
    let mut s = clean_text(t);
    for sep in [" | ", " - ", " – ", " — "] {
        if let Some((a, b)) = s.split_once(sep) {
            let b_lc = b.to_lowercase();
            let site = host.split('.').next().unwrap_or("");
            if !site.is_empty() && b_lc.replace(' ', "").contains(site) {
                s = a.to_string();
            }
        }
    }
    let s = s.trim();
    let s = s.strip_suffix(" Recipe").or(s.strip_suffix(" recipe")).unwrap_or(s);
    s.trim().to_string()
}

pub fn import_html(html: &str, url: &str, lib: &Library) -> ImportResult {
    let doc = Html::parse_document(html);
    let host = rules::host_of(url);
    let builtin = rules::builtin_rules();
    let mut res = ImportResult::default();

    let mut candidates: Vec<(String, Draft)> = vec![];

    match extract_jsonld(&doc) {
        Some(d) => {
            res.tried.push(("Recipe data on the page".into(), format!("{} ingredients, {} steps", d.ingredients.len(), d.steps.len())));
            candidates.push(("Recipe data on the page".into(), d));
        }
        None => res.tried.push(("Recipe data on the page".into(), "none found".into())),
    }
    if let Some(rule) = rules::rule_for(&host, &lib.rule_overrides, &builtin) {
        res.rule_domain = Some(rule.domain.clone());
        let label = format!("Site rule: {} v{}", rule.name, rule.version);
        match apply_rule_doc(&doc, rule) {
            Some(d) if !d.ingredients.is_empty() => {
                res.rule_worked = Some(true);
                res.tried.push((label.clone(), format!("{} ingredients, {} steps", d.ingredients.len(), d.steps.len())));
                candidates.push((label, d));
            }
            _ => {
                res.rule_worked = Some(false);
                res.tried.push((label, "found nothing - the site's layout may have changed".into()));
            }
        }
    }
    match extract_plugins(&doc) {
        Some((name, d)) => {
            res.tried.push((format!("Recipe plugin layout ({})", name), format!("{} ingredients, {} steps", d.ingredients.len(), d.steps.len())));
            candidates.push((format!("Recipe plugin layout ({})", name), d));
        }
        None => res.tried.push(("Recipe plugin layouts".into(), "none found".into())),
    }
    if candidates.iter().all(|c| c.1.ingredients.is_empty() || c.1.steps.is_empty()) {
        match extract_heuristic(&doc) {
            Some(d) => {
                res.tried.push(("General page reading".into(), format!("{} ingredients, {} steps", d.ingredients.len(), d.steps.len())));
                candidates.push(("General page reading".into(), d));
            }
            None => res.tried.push(("General page reading".into(), "no Ingredients heading found".into())),
        }
    }

    if candidates.is_empty() {
        res.field_issues.push("No recipe found on this page. Paste the recipe text instead, or check the address.".into());
        res.recipe.name = tidy_title(&page_title(&doc), &host);
        res.recipe.source_url = url.to_string();
        res.method = "Nothing found".into();
        return res;
    }

    // Best candidate: the structured data wins when it is complete; otherwise the highest score.
    let best_idx = candidates
        .iter()
        .position(|(n, d)| n.starts_with("Recipe data") && d.ingredients.len() >= 2 && !d.steps.is_empty())
        .unwrap_or_else(|| {
            let mut bi = 0;
            for (i, c) in candidates.iter().enumerate() {
                if draft_score(&c.1) > draft_score(&candidates[bi].1) {
                    bi = i;
                }
            }
            bi
        });
    let (method, mut best) = candidates[best_idx].clone();
    // Fill gaps from the other methods.
    for (i, (_, d)) in candidates.iter().enumerate() {
        if i == best_idx {
            continue;
        }
        if best.steps.is_empty() && !d.steps.is_empty() {
            best.steps = d.steps.clone();
        }
        if best.ingredients.is_empty() && !d.ingredients.is_empty() {
            best.ingredients = d.ingredients.clone();
        }
        if best.title.is_empty() {
            best.title = d.title.clone();
        }
        if best.servings.is_empty() {
            best.servings = d.servings.clone();
        }
        // Cross-check counts between independent methods.
        let a = best.ingredients.len() as i64;
        let b = d.ingredients.len() as i64;
        if a > 0 && b > 0 && (a - b).abs() > 2 {
            res.field_issues.push(format!(
                "Two reading methods disagree on the ingredient count ({} vs {}). Check nothing is missing.",
                a, b
            ));
        }
    }
    res.method = method;
    if best.title.is_empty() {
        best.title = page_title(&doc);
    }

    let mut r = Recipe {
        name: tidy_title(&best.title, &host),
        ingredients: best.ingredients.iter().map(|s| clean_text(s)).collect::<Vec<_>>().join("\n"),
        directions: best.steps.iter().map(|s| clean_text(s)).collect::<Vec<_>>().join("\n"),
        servings: best.servings.clone(),
        prep_time: best.prep.clone(),
        cook_time: best.cook.clone(),
        total_time: best.total.clone(),
        description: best.description.clone(),
        video_url: best.video_url.clone(),
        source: host.clone(),
        source_url: url.to_string(),
        nutrition: best.nutrition.clone(),
        nutrition_edited: best.nutrition.is_some(),
        ..Default::default()
    };
    r.categories = best.categories.iter().filter(|c| lib.categories.iter().any(|k| k.eq_ignore_ascii_case(c))).cloned().collect();
    file_under_cuisine(&mut r, &best.cuisines);
    res.image_url = if best.image_url.is_empty() { meta(&doc, "og:image").unwrap_or_default() } else { best.image_url.clone() };
    if r.total_time.is_empty() && (!r.prep_time.is_empty() || !r.cook_time.is_empty()) {
        r.total_time = ingredients::sum_times(&r.prep_time, &r.cook_time);
    }
    add_field_issues(&r, &mut res.field_issues);
    res.recipe = r;
    res
}

fn add_field_issues(r: &Recipe, out: &mut Vec<String>) {
    if r.name.trim().is_empty() {
        out.push("No title found.".into());
    }
    if r.ingredients.trim().is_empty() {
        out.push("No ingredients found.".into());
    }
    if r.directions.trim().is_empty() {
        out.push("No directions found.".into());
    }
    if r.servings.trim().is_empty() {
        out.push("Servings not found (needed for per-serving nutrition).".into());
    }
}

/// Import from pasted text (a recipe copied from anywhere).
pub fn import_text(text: &str, lib: &Library) -> ImportResult {
    let trimmed = text.trim_start();
    if trimmed.starts_with('<') {
        return import_html(text, "", lib);
    }
    let mut res = ImportResult { method: "Pasted text".into(), ..Default::default() };
    let mut title = String::new();
    let mut ing = vec![];
    let mut steps = vec![];
    let mut servings = String::new();
    #[derive(PartialEq)]
    enum Sec {
        Head,
        Ing,
        Dir,
        Notes,
    }
    let mut sec = Sec::Head;
    let mut notes = vec![];
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let lc = t.to_lowercase();
        let h = lc.trim_end_matches(':');
        if heading_matches(t, &["ingredients", "you will need", "what you need"]) {
            sec = Sec::Ing;
            continue;
        }
        if heading_matches(t, &["directions", "instructions", "method", "preparation", "steps"]) {
            sec = Sec::Dir;
            continue;
        }
        if heading_matches(t, &["notes", "tips"]) {
            sec = Sec::Notes;
            continue;
        }
        if (h.starts_with("serves") || h.starts_with("servings") || h.starts_with("yield") || h.starts_with("makes")) && t.len() < 40 {
            servings = t.split_once([':', ' ']).map(|x| x.1.trim().to_string()).unwrap_or_default();
            continue;
        }
        match sec {
            Sec::Head => {
                if title.is_empty() {
                    title = t.to_string();
                } else {
                    // No headings: guess by shape.
                    let p = parse_ingredient(t);
                    if p.quantity.is_some() && t.len() < 80 {
                        ing.push(t.to_string());
                    } else if t.len() > 25 {
                        steps.push(t.to_string());
                    } else {
                        ing.push(t.to_string());
                    }
                }
            }
            Sec::Ing => ing.push(t.to_string()),
            Sec::Dir => steps.extend(split_steps(t)),
            Sec::Notes => notes.push(t.to_string()),
        }
    }
    let r = Recipe {
        name: title,
        ingredients: ing.join("\n"),
        directions: steps.join("\n"),
        servings,
        notes: notes.join("\n"),
        source: "Pasted text".into(),
        ..Default::default()
    };
    let mut r = r;
    file_under_cuisine(&mut r, &[]);
    res.tried.push(("Pasted text".into(), format!("{} ingredients, {} steps", ing.len(), steps.len())));
    add_field_issues(&r, &mut res.field_issues);
    res.recipe = r;
    res
}

// ---------------- review flags ----------------

/// Fields an inbox item can carry beside its text (simple JSON: prep_time, cook_time,
/// source_url, video_url, categories).
pub fn apply_extra(r: &mut Recipe, extra: &str) {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(extra) {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        if !s("prep_time").is_empty() {
            r.prep_time = s("prep_time");
        }
        if !s("cook_time").is_empty() {
            r.cook_time = s("cook_time");
        }
        if !s("source_url").is_empty() {
            r.source_url = s("source_url");
        }
        if !s("video_url").is_empty() {
            r.video_url = s("video_url");
        }
        if let Some(serde_json::Value::Array(a)) = v.get("categories") {
            r.categories = a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect();
        }
        if r.total_time.is_empty() {
            r.total_time = crate::ingredients::sum_times(&r.prep_time, &r.cook_time);
        }
    }
}

pub fn review_summary(r: &Recipe) -> Vec<String> {
    let mut out = vec![];
    for line in r.ingredient_lines() {
        let p = parse_ingredient(line);
        if p.flagged() {
            out.push(format!("Ingredient \"{}\": {}", line, p.issues.join("; ")));
        }
    }
    let steps = r.steps();
    for (i, s) in steps.iter().enumerate() {
        let rep = check_step(s, if i > 0 { Some(steps[i - 1].as_str()) } else { None });
        if rep.flagged() {
            out.push(format!("Step {}: {}", i + 1, rep.issues.join("; ")));
        }
    }
    out
}

// ---------------- duplicates ----------------

#[derive(Clone, Debug)]
pub struct DuplicateMatch {
    pub id: u64,
    pub name: String,
    pub score: f32,
    pub reason: String,
}

pub fn normalize_url(u: &str) -> String {
    let u = u.trim().to_lowercase();
    let u = u.split("://").nth(1).unwrap_or(&u).to_string();
    let u = u.trim_start_matches("www.");
    let u = u.split(['?', '#']).next().unwrap_or(u);
    u.trim_end_matches('/').to_string()
}

const STOP: &[&str] = &["the", "a", "an", "and", "with", "of", "my", "best", "easy", "recipe", "simple", "homemade", "perfect"];

fn word_set(s: &str) -> Vec<String> {
    let mut v: Vec<String> = ingredients::match_key(s).split(' ').filter(|w| !w.is_empty() && !STOP.contains(w)).map(|w| w.to_string()).collect();
    v.sort();
    v.dedup();
    v
}

fn jaccard(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.iter().filter(|x| b.contains(x)).count() as f32;
    let uni = (a.len() + b.len()) as f32 - inter;
    if uni == 0.0 { 0.0 } else { inter / uni }
}

fn ingredient_names(r: &Recipe) -> Vec<String> {
    let mut v: Vec<String> = r
        .ingredient_lines()
        .iter()
        .map(|l| parse_ingredient(l))
        .filter(|p| !p.is_heading && !p.name.is_empty())
        .map(|p| ingredients::match_key(&p.name))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Same recipe already in the library? Same source page, or very similar title and ingredients.
pub fn find_duplicate(lib: &Library, r: &Recipe, skip_id: Option<u64>) -> Option<DuplicateMatch> {
    let url = normalize_url(&r.source_url);
    let tw = word_set(&r.name);
    let iw = ingredient_names(r);
    let mut best: Option<DuplicateMatch> = None;
    for e in lib.recipes.iter().filter(|e| !e.in_trash && Some(e.id) != skip_id) {
        let (score, reason) = if !url.is_empty() && normalize_url(&e.source_url) == url {
            (1.0, "Imported from the same page".to_string())
        } else {
            let t = jaccard(&tw, &word_set(&e.name));
            let i = jaccard(&iw, &ingredient_names(e));
            let s = if iw.is_empty() { t * 0.8 } else { 0.55 * t + 0.45 * i };
            (s, format!("Title {}% alike, ingredients {}% alike", (t * 100.0).round(), (i * 100.0).round()))
        };
        if score >= DUPLICATE_THRESHOLD && best.as_ref().map(|b| score > b.score).unwrap_or(true) {
            best = Some(DuplicateMatch { id: e.id, name: e.name.clone(), score, reason });
        }
    }
    best
}

pub const DUPLICATE_THRESHOLD: f32 = 0.72;

/// Merge an incoming copy into the existing recipe. Keeps the person's own data (favorite, rating,
/// categories, notes, edits) and fills anything missing from the new copy.
/// Merge every pair of duplicate recipes (not in Trash) into the older one; the newer copy is
/// removed with a tombstone so sync doesn't bring it back. Returns how many were merged.
pub fn merge_all_duplicates(lib: &mut Library) -> usize {
    let mut merged = 0;
    loop {
        let pair = lib.recipes.iter().filter(|r| !r.in_trash).find_map(|r| {
            find_duplicate(lib, r, Some(r.id)).map(|d| if d.id < r.id { (d.id, r.id) } else { (r.id, d.id) })
        });
        let Some((keep, drop)) = pair else { break };
        let incoming = lib.recipe(drop).unwrap().clone();
        if let Some(k) = lib.recipe_mut(keep) {
            merge_into(k, &incoming);
            k.modified = crate::model::now_secs();
        }
        lib.recipes.retain(|r| r.id != drop);
        lib.tombstones.push((incoming.uid.clone(), crate::model::now_secs()));
        merged += 1;
    }
    merged
}

pub fn merge_into(existing: &mut Recipe, incoming: &Recipe) -> Vec<String> {
    let mut changes = vec![];
    macro_rules! fill {
        ($f:ident, $label:expr) => {
            if existing.$f.trim().is_empty() && !incoming.$f.trim().is_empty() {
                existing.$f = incoming.$f.clone();
                changes.push(format!("Added missing {}", $label));
            }
        };
    }
    fill!(ingredients, "ingredients");
    fill!(directions, "directions");
    fill!(description, "description");
    fill!(servings, "servings");
    fill!(prep_time, "prep time");
    fill!(cook_time, "cook time");
    fill!(total_time, "total time");
    fill!(source_url, "source link");
    fill!(video_url, "video link");
    fill!(difficulty, "difficulty");
    fill!(photo, "photo");
    for c in &incoming.categories {
        if !existing.categories.contains(c) {
            existing.categories.push(c.clone());
            changes.push(format!("Added category {}", c));
        }
    }
    if incoming.rating > existing.rating {
        existing.rating = incoming.rating;
    }
    existing.favorite |= incoming.favorite;
    if !incoming.notes.trim().is_empty() && !existing.notes.contains(incoming.notes.trim()) {
        if !existing.notes.is_empty() {
            existing.notes.push_str("\n\n");
        }
        existing.notes.push_str(incoming.notes.trim());
        changes.push("Kept notes from both copies".into());
    }
    if existing.nutrition.is_none() && incoming.nutrition.is_some() {
        existing.nutrition = incoming.nutrition.clone();
    }
    let a = ingredient_names(existing);
    let b = ingredient_names(incoming);
    if !a.is_empty() && !b.is_empty() && jaccard(&a, &b) < 0.95 {
        existing.notes.push_str(&format!(
            "{}Ingredients from the newer copy:\n{}",
            if existing.notes.is_empty() { "" } else { "\n\n" },
            incoming.ingredients
        ));
        changes.push("Ingredient lists differed: newer list saved in Notes".into());
    }
    existing.merged_count += 1;
    existing.modified = crate::model::now_secs();
    if changes.is_empty() {
        changes.push("Nothing new; your copy was already complete".into());
    }
    changes
}

// ---------------- one-pass cleanup ----------------

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn title_case(s: &str) -> String {
    let small = ["a", "an", "and", "of", "with", "the", "in", "on", "for", "to", "or"];
    s.split_whitespace()
        .enumerate()
        .map(|(i, w)| {
            let lw = w.to_lowercase();
            if i > 0 && small.contains(&lw.as_str()) { lw } else { capitalize(&lw) }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

const CLUTTER_LINES: &[&str] = &[
    "advertisement", "jump to recipe", "print recipe", "pin recipe", "rate this recipe", "save recipe", "cook mode",
    "prevent your screen from going dark", "watch the video", "deselect all", "subscribe",
];

/// Fix formatting across the whole recipe in one pass. Returns what changed.
pub fn tidy_recipe(r: &mut Recipe, unit_short: bool) -> Vec<String> {
    let mut changes: Vec<String> = vec![];
    // Title
    let t = clean_text(&r.name);
    let letters: Vec<char> = t.chars().filter(|c| c.is_alphabetic()).collect();
    let t2 = if !letters.is_empty() && (letters.iter().all(|c| c.is_uppercase()) || letters.iter().all(|c| c.is_lowercase())) {
        title_case(&t)
    } else {
        t
    };
    if t2 != r.name {
        r.name = t2;
        changes.push("Tidied the title".into());
    }

    // Ingredients
    let mut out: Vec<String> = vec![];
    let mut removed = 0;
    let mut rewritten = 0;
    for line in r.ingredients.lines() {
        let l = clean_text(line);
        if l.is_empty() {
            continue;
        }
        let lc = l.to_lowercase();
        if CLUTTER_LINES.iter().any(|c| lc == *c || lc.starts_with(c)) {
            removed += 1;
            continue;
        }
        let p = parse_ingredient(&l);
        let mut tidy = if p.confidence >= 0.85 || p.is_heading { p.tidy() } else { ingredients::normalize_text(ingredients::strip_bullet(&l)) };
        if unit_short && !p.is_heading && p.confidence >= 0.85 {
            if let Some(u) = &p.unit {
                let long = p.unit_text();
                let short = ingredients::short_unit(u);
                if short != u.as_str() {
                    tidy = tidy.replacen(&long, short, 1);
                }
            }
        }
        if out.last().map(|prev: &String| prev.eq_ignore_ascii_case(&tidy)).unwrap_or(false) {
            removed += 1;
            continue;
        }
        if tidy != l {
            rewritten += 1;
        }
        out.push(tidy);
    }
    let new_ing = out.join("\n");
    if new_ing != r.ingredients {
        r.ingredients = new_ing;
        if rewritten > 0 {
            changes.push(format!("Standardised {} ingredient line(s): amounts, fractions and units", rewritten));
        }
        if removed > 0 {
            changes.push(format!("Removed {} clutter or repeated ingredient line(s)", removed));
        }
    }

    // Steps
    let steps = r.steps();
    let mut out: Vec<String> = vec![];
    let mut merged = 0;
    let mut dropped = 0;
    // A step that started lowercase is only a broken piece when the one before it stopped
    // mid-sentence; "Melt the butter." then "add the salmon." are two real steps.
    let mut prev_open = false;
    for s in steps {
        let s = clean_text(&ingredients::normalize_text(&s));
        let lc = s.to_lowercase();
        if CLUTTER_LINES.iter().any(|c| lc == *c || lc.starts_with(c)) {
            dropped += 1;
            continue;
        }
        if out.last().map(|p| *p == s).unwrap_or(false) {
            dropped += 1;
            continue;
        }
        if prev_open && s.chars().next().map(|c| c.is_lowercase()).unwrap_or(false) && !out.is_empty() {
            let last = out.last_mut().unwrap();
            if last.ends_with('.') {
                last.pop(); // the full stop added when it was pushed (it had none of its own)
            }
            last.push(' ');
            last.push_str(&s);
            if !last.ends_with(['.', '!', '?', ':', ')']) {
                last.push('.');
            }
            prev_open = !s.trim_end().ends_with(['.', '!', '?']);
            merged += 1;
            continue;
        }
        prev_open = !s.trim_end().ends_with(['.', '!', '?']);
        let mut s = capitalize(&s);
        if !s.ends_with(['.', '!', '?', ':', ')']) {
            s.push('.');
        }
        out.push(s);
    }
    let new_dir = out.join("\n");
    if new_dir != r.directions {
        r.directions = new_dir;
        changes.push("Split steps, removed numbering and fixed capitals/punctuation".into());
        if merged > 0 {
            changes.push(format!("Joined {} broken step(s) back together", merged));
        }
        if dropped > 0 {
            changes.push(format!("Removed {} clutter or repeated step(s)", dropped));
        }
    }
    for (f, label) in [(&mut r.description, "description"), (&mut r.notes, "notes")] {
        let c = f.lines().map(clean_text).collect::<Vec<_>>().join("\n").trim().to_string();
        if c != *f {
            *f = c;
            changes.push(format!("Cleaned the {}", label));
        }
    }
    if r.total_time.is_empty() && (!r.prep_time.is_empty() || !r.cook_time.is_empty()) {
        r.total_time = ingredients::sum_times(&r.prep_time, &r.cook_time);
        if !r.total_time.is_empty() {
            changes.push("Filled in the total time".into());
        }
    }
    if changes.is_empty() {
        changes.push("Everything already looked tidy".into());
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    const LD: &str = r#"<html><head><title>Best Banana Bread Recipe | Example Kitchen</title>
<script type="application/ld+json">{"@context":"https://schema.org","@graph":[{"@type":"WebPage"},{"@type":"Recipe","name":"Banana Bread","recipeIngredient":["3 ripe bananas","1/3 cup melted butter","3/4 cup sugar","1 egg, beaten","1 teaspoon baking soda","1 1/2 cups all-purpose flour"],"recipeInstructions":[{"@type":"HowToStep","text":"Heat the oven to 350&deg;F."},{"@type":"HowToStep","text":"Mash the bananas and mix in the butter."},{"@type":"HowToStep","text":"Bake for 1 hour."}],"recipeYield":["8","8 slices"],"prepTime":"PT10M","cookTime":"PT1H"}]}</script>
</head><body><h1>Banana Bread</h1></body></html>"#;

    #[test]
    fn jsonld_import() {
        let lib = Library::default();
        let r = import_html(LD, "https://example.com/banana-bread", &lib);
        assert_eq!(r.method, "Recipe data on the page");
        assert_eq!(r.recipe.name, "Banana Bread");
        assert_eq!(r.recipe.ingredient_lines().len(), 6);
        assert_eq!(r.recipe.steps().len(), 3);
        assert_eq!(r.recipe.steps()[0], "Heat the oven to 350°F.");
        assert_eq!(r.recipe.servings, "8 servings");
        assert_eq!(r.recipe.total_time, "1 hr 10 min");
    }

    #[test]
    fn page_nutrition_leaves_missing_values_unknown() {
        let nutrition = r#""cookTime":"PT1H","nutrition":{"@type":"NutritionInformation","calories":"250 calories","proteinContent":"10 g","fatContent":"0 g","carbohydrateContent":"30 g","fiberContent":"n/a"}"#;
        let html = LD.replace(r#""cookTime":"PT1H""#, nutrition);
        let d = extract_jsonld(&Html::parse_document(&html)).expect("recipe on the page");
        let n = d.nutrition.expect("page has nutrition");
        assert_eq!((n.calories, n.protein_g, n.fat_g, n.carbs_g), (Some(250.0), Some(10.0), Some(0.0), Some(30.0)));
        assert_eq!((n.fiber_g, n.sugar_g, n.sodium_mg), (None, None, None));
    }

    #[test]
    fn cuisine_and_picture_from_page() {
        let html = r#"<html><head><script type="application/ld+json">{"@type":"Recipe","name":"Pork Adobo","recipeCuisine":"Filipino","image":[{"@type":"ImageObject","url":"https://example.com/adobo.jpg"}],"recipeIngredient":["2 lbs pork","1/2 cup soy sauce"],"recipeInstructions":"Simmer everything for 1 hour."}</script></head></html>"#;
        let r = import_html(html, "https://example.com/adobo", &Library::default());
        assert!(r.recipe.categories.contains(&"Cuisines/Filipino".to_string()));
        assert_eq!(r.image_url, "https://example.com/adobo.jpg");
        // no cuisine on the page: guessed from the ingredients
        let t = import_text("Weeknight Curry\nIngredients\n2 tbsp garam masala\n1 cup paneer\n1 cup peas\nDirections\nCook it all for 20 minutes.", &Library::default());
        assert!(t.recipe.categories.contains(&"Cuisines/Indian".to_string()), "{:?}", t.recipe.categories);
        // nothing distinctive: left alone
        let n = import_text("Toast\nIngredients\n1 slice bread\nDirections\nToast the bread for 2 minutes.", &Library::default());
        assert!(n.recipe.categories.is_empty());
    }

    #[test]
    fn heuristic_import() {
        let html = "<html><body><h1>Lemonade</h1><h2>Ingredients</h2><ul><li>1 cup lemon juice</li><li>1 cup sugar</li><li>5 cups water</li></ul><h2>Directions</h2><ol><li>Stir the sugar into the juice.</li><li>Add water and chill.</li></ol></body></html>";
        let r = import_html(html, "https://unknown.example/lemonade", &Library::default());
        assert_eq!(r.method, "General page reading");
        assert_eq!(r.recipe.ingredient_lines().len(), 3);
        assert_eq!(r.recipe.steps().len(), 2);
    }

    #[test]
    fn duplicates_and_merge() {
        let mut lib = Library::default();
        let mut a = Recipe { name: "Banana Bread".into(), ingredients: "3 bananas\n1 cup flour\n1 egg".into(), favorite: true, ..Default::default() };
        a.source_url = "https://www.example.com/banana-bread/".into();
        let id = lib.add_recipe(a);
        let b = Recipe { name: "Best Banana Bread".into(), ingredients: "3 ripe bananas\n1 cup flour\n1 egg".into(), directions: "Bake it.".into(), source_url: "http://example.com/banana-bread?utm=x".into(), ..Default::default() };
        let d = find_duplicate(&lib, &b, None).expect("dup");
        assert_eq!(d.id, id);
        let c = Recipe { name: "Banana Bread Muffins".into(), ingredients: "3 bananas\n1 cup flour\n1 egg".into(), ..Default::default() };
        assert!(find_duplicate(&lib, &c, None).is_some());
        let e = Recipe { name: "Beef Stew".into(), ingredients: "1 lb beef\n2 carrots".into(), ..Default::default() };
        assert!(find_duplicate(&lib, &e, None).is_none());
        let ex = lib.recipe_mut(id).unwrap();
        merge_into(ex, &b);
        assert!(ex.favorite);
        assert_eq!(ex.directions, "Bake it.");
        // merging everything keeps the older recipe, drops the copy and leaves the stew alone
        let bid = lib.add_recipe(b);
        lib.add_recipe(e);
        assert_eq!(merge_all_duplicates(&mut lib), 1);
        assert!(lib.recipe(id).is_some() && lib.recipe(bid).is_none());
        assert_eq!(lib.recipes.len(), 2);
        assert_eq!(lib.tombstones.len(), 1);
    }

    #[test]
    fn cleanup_pass() {
        let mut r = Recipe {
            name: "CHOCOLATE CHIP COOKIES".into(),
            ingredients: "▢ 1½ cups flour\nAdvertisement\n2 tablespoon butter\n2 tablespoon butter".into(),
            directions: "1. preheat oven. 2. Mix the dough well 3. Bake for 10 minutes".into(),
            ..Default::default()
        };
        let ch = tidy_recipe(&mut r, false);
        assert!(!ch.is_empty());
        assert_eq!(r.name, "Chocolate Chip Cookies");
        assert_eq!(r.ingredient_lines(), vec!["1 1/2 cups flour", "2 tablespoons butter"]);
        assert_eq!(r.steps()[0], "Preheat oven.");
    }

    #[test]
    fn only_unfinished_steps_are_joined() {
        // finished sentences that start lowercase stay separate steps
        let mut r = Recipe { directions: "1. melt the butter for 2 minutes.
2. add the salmon.
3. squeeze lemon over".into(), ..Default::default() };
        tidy_recipe(&mut r, false);
        assert_eq!(r.steps(), vec!["Melt the butter for 2 minutes.", "Add the salmon.", "Squeeze lemon over."]);
        // a step cut off mid-sentence is joined back, without a stray full stop in the middle
        let mut r = Recipe { directions: "Whisk the eggs with
the sugar until pale.
Fold in the flour (gently)
and bake.".into(), ..Default::default() };
        tidy_recipe(&mut r, false);
        assert_eq!(r.steps(), vec!["Whisk the eggs with the sugar until pale.", "Fold in the flour (gently) and bake."]);
    }

    /// Live check against real sites: RECIPE_URLS="url1 url2" cargo test live_sites -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_sites() {
        let urls = std::env::var("RECIPE_URLS").unwrap_or_default();
        let lib = Library::default();
        for u in urls.split_whitespace() {
            match fetch(u) {
                Ok(html) => {
                    let r = import_html(&html, u, &lib);
                    let flagged = review_summary(&r.recipe);
                    println!(
                        "{}\n  title: {}\n  method: {}\n  {} ingredients, {} steps, servings '{}', {} flagged\n  tried: {:?}\n  first: {:?}\n  flags: {:?}\n",
                        u, r.recipe.name, r.method, r.recipe.ingredient_lines().len(), r.recipe.steps().len(), r.recipe.servings,
                        flagged.len(), r.tried, r.recipe.ingredient_lines().first(), flagged.iter().take(4).collect::<Vec<_>>()
                    );
                    if !r.image_url.is_empty() {
                        match fetch_bytes(&r.image_url) {
                            Ok(b) => println!("  picture: {} bytes\n", b.len()),
                            Err(e) => println!("  PICTURE FAILED: {}\n", e),
                        }
                    }
                }
                Err(e) => println!("{}\n  FETCH FAILED: {}\n", u, e),
            }
        }
    }

    #[test]
    fn refuses_non_web_links() {
        for (link, scheme) in [
            ("file:///C:/Windows/win.ini", "file://"),
            ("FILE:///x", "FILE://"),
            ("ftp://x.com/a", "ftp://"),
            ("data:text/html,hi", "data:"),
            ("javascript:alert(1)", "javascript:"),
            ("gopher://x.com", "gopher://"),
            ("mailto:a@b.com", "mailto:"),
        ] {
            let e = check_web_url(link).unwrap_err();
            assert!(e.contains(ONLY_WEB) && e.contains(&format!("starts with {scheme}")), "{link}: {e}");
        }
        assert_eq!(check_web_url("-o x"), Err(DASH_LINK.to_string()));
        assert_eq!(check_web_url("--config x"), Err(DASH_LINK.to_string()));
        assert_eq!(check_web_url("https://u:p@example.com/"), Err(USER_IN_LINK.to_string()));
        assert_eq!(check_web_url("example.com/soup").unwrap(), "https://example.com/soup");
        assert_eq!(check_web_url("example.com:8443/a").unwrap(), "https://example.com:8443/a"); // host and port, not a scheme
        assert!(check_web_url("HTTPS://Example.com/a").is_ok());
        // refused before any download, by either entry point
        for link in ["file:///C:/Windows/win.ini", "-o x", "--config x"] {
            assert_eq!(fetch(link).unwrap_err(), check_web_url(link).unwrap_err());
            assert_eq!(fetch_bytes(link).unwrap_err(), check_web_url(link).unwrap_err());
        }
    }

    #[test]
    fn redirect_targets_are_checked() {
        assert_eq!(redirect_target("https://a.com/x", "/y").unwrap(), "https://a.com/y");
        for bad in ["file:///etc/passwd", "ftp://b.com/"] {
            assert!(redirect_target("https://a.com/x", bad).unwrap_err().contains(ONLY_WEB), "{bad}");
        }
    }

    #[test]
    fn curl_reads_the_link_as_a_link() {
        let url = "https://example.com/--config";
        let ip: IpAddr = "93.184.216.34".parse().unwrap();
        let a = curl_args(url, PAGE_MAX_BYTES, true, Path::new("out.tmp"), "example.com", 443, ip);
        assert_eq!(a[0], "-q");
        for (k, v) in [("--proto", "=http,https"), ("--proto-redir", "=http,https"), ("--max-redirs", "0")] {
            assert!(a.windows(2).any(|w| w[0] == k && w[1] == v), "{k} {v}");
        }
        assert_eq!(a[a.len() - 2], "--");
        assert_eq!(a[a.len() - 1], url);
        assert!(!a.iter().any(|s| s == "-L" || s == "-sSL" || s == "--location"));
        // a host name is pinned to the address that was checked; an address needs no pin
        let pin = a.iter().position(|s| s == "--resolve").expect("--resolve");
        assert_eq!(a[pin + 1], "example.com:443:93.184.216.34");
        assert!(pin < a.len() - 2);
        let lit = curl_args("http://93.184.216.34/x", PICTURE_MAX_BYTES, false, Path::new("out.tmp"), "93.184.216.34", 80, ip);
        assert!(!lit.iter().any(|s| s == "--resolve"));
        let six: IpAddr = "2001:4860:4860::8888".parse().unwrap();
        let a6 = curl_args(url, PAGE_MAX_BYTES, true, Path::new("out.tmp"), "example.com", 443, six);
        assert!(a6.iter().any(|s| s == "example.com:443:[2001:4860:4860::8888]"));
    }

    #[test]
    fn local_and_private_hosts_are_refused() {
        for ip in [
            "127.0.0.1", "10.1.2.3", "172.16.0.1", "172.31.255.255", "192.168.1.1", "169.254.169.254", "0.0.0.0", "255.255.255.255", "::1", "::", "fe80::1", "fc00::1",
            "fd12:3456::1", "::ffff:127.0.0.1", "::ffff:192.168.0.1",
        ] {
            assert!(is_local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "93.184.216.34", "2001:4860:4860::8888"] {
            assert!(!is_local(ip.parse().unwrap()), "{ip}");
        }
        // refused before any connection, by literal address, by name, in both entry points
        for link in ["http://localhost/", "http://127.0.0.1:1/", "http://[::1]/", "http://2130706433/"] {
            assert_eq!(fetch(link), Err(NOT_LOCAL.to_string()), "{link}");
        }
        assert_eq!(fetch_bytes("http://192.168.1.1/a.jpg"), Err(NOT_LOCAL.to_string()));
    }

    #[test]
    fn pasted_text() {
        let t = "Pancakes\nServes: 4\nIngredients\n1 cup flour\n1 cup milk\n1 egg\nDirections\nMix everything.\nCook on a hot pan for 2 minutes.";
        let r = import_text(t, &Library::default());
        assert_eq!(r.recipe.name, "Pancakes");
        assert_eq!(r.recipe.servings, "4");
        assert_eq!(r.recipe.ingredient_lines().len(), 3);
        assert_eq!(r.recipe.steps().len(), 2);
    }
}
