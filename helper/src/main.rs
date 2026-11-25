//! tiffin-helper: the one server behind Tiffin on the web. Browsers can't read other sites' pages
//! (CORS), so the web app asks here. It keeps nothing and logs no addresses or content: every
//! request is a single fetch through the importer's own download rules, passed straight back.

use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use recipe_box::foods::off::{self, Limiter, OffAnswer};
use recipe_box::importer;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Semaphore;

/// A download that runs longer than this is answered as too slow (the importer allows 25 s a hop).
const FETCH_LIMIT: Duration = Duration::from_secs(60);
/// Downloads running at once; more wait for none and get BUSY.
const AT_ONCE: usize = 16;

const NOT_TIFFIN: &str = "This helper answers only the Tiffin web app.";
const NO_ADDRESS: &str = "Send the page address as ?url=…";
const BUSY: &str = "Tiffin's helper is busy. Try again in a moment.";
const SLOW: &str = "The site took too long to answer.";
const BROKE: &str = "Tiffin's helper hit a problem with that page.";
/// Longest search text passed on; a food name is far shorter.
const MAX_QUERY_CHARS: usize = 200;
const BAD_QUERY: &str = "Search for a food name between 2 and 200 letters long.";
/// Clients remembered for their limits.
const MAX_CLIENTS: usize = 50_000;

#[derive(Clone)]
struct Helper {
    /// Web addresses of the Tiffin web app allowed to read the answers, e.g. https://tiffin.fyi
    origins: Arc<Vec<String>>,
    slots: Arc<Semaphore>,
    /// Each client's sends, by address: the desktop's own rule, `off::Limiter` (8 a minute).
    clients: Arc<Mutex<HashMap<IpAddr, Sends>>>,
    /// Open Food Facts said stop (429/503): nobody's search is sent before this second.
    off_stopped_until: Arc<Mutex<u64>>,
    /// Behind a reverse proxy, the client is the last X-Forwarded-For entry (the one the proxy added).
    behind_proxy: bool,
}

#[derive(Default)]
struct Sends {
    off: Limiter,
    fetch: Limiter,
}

#[cfg(test)]
fn app(origins: Vec<String>, at_once: usize) -> Router {
    router(origins, at_once, false)
}

fn router(origins: Vec<String>, at_once: usize, behind_proxy: bool) -> Router {
    let h = Helper {
        origins: Arc::new(origins),
        slots: Arc::new(Semaphore::new(at_once)),
        clients: Arc::default(),
        off_stopped_until: Arc::default(),
        behind_proxy,
    };
    Router::new()
        .route("/page", get(page))
        .route("/photo", get(photo))
        .route("/off", get(off_search))
        .layer(middleware::from_fn_with_state(h.clone(), harden))
        .with_state(h)
}

/// A recipe page, as text. The browser runs the importer on it.
async fn page(State(h): State<Helper>, ConnectInfo(peer): ConnectInfo<SocketAddr>, hd: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let who = client(&h, peer, &hd);
    relay(h, who, &hd, q, |url| importer::fetch(url).map(String::into_bytes), "text/plain; charset=utf-8").await
}

/// A recipe picture, as bytes.
async fn photo(State(h): State<Helper>, ConnectInfo(peer): ConnectInfo<SocketAddr>, hd: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    let who = client(&h, peer, &hd);
    relay(h, who, &hd, q, importer::fetch_bytes, "application/octet-stream").await
}

async fn relay(h: Helper, who: IpAddr, hd: &HeaderMap, q: HashMap<String, String>, fetch: fn(&str) -> Result<Vec<u8>, String>, kind: &'static str) -> Response {
    if let Err(no) = from_tiffin(&h, hd) {
        return no;
    }
    let Some(url) = q.get("url").filter(|u| !u.trim().is_empty()).cloned() else {
        return (StatusCode::BAD_REQUEST, NO_ADDRESS).into_response();
    };
    if let Err(wait) = allow(&h, who, |s| &mut s.fetch) {
        let msg = format!("Too many downloads from here. Try again in {wait} seconds.");
        return (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, wait.to_string())], msg).into_response();
    }
    match blocking(&h, move || fetch(&url)).await {
        Ok(Ok(body)) => ([(header::CONTENT_TYPE, kind)], body).into_response(),
        // the importer's own message: refused link, site error, too big, too many redirects
        Ok(Err(why)) => (StatusCode::UNPROCESSABLE_ENTITY, why).into_response(),
        Err(no) => no,
    }
}

/// An Open Food Facts name search, sent with the person's own contact email. The answer is the
/// same `OffAnswer` the desktop app gets, as JSON. Barcode lookups don't come here: OFF lets the
/// browser ask for those itself.
async fn off_search(State(h): State<Helper>, ConnectInfo(peer): ConnectInfo<SocketAddr>, hd: HeaderMap, Query(q): Query<HashMap<String, String>>) -> Response {
    if let Err(no) = from_tiffin(&h, &hd) {
        return no;
    }
    let text = q.get("q").map(|t| t.trim().to_string()).unwrap_or_default();
    if !off::query_ready(&text) || text.chars().count() > MAX_QUERY_CHARS {
        return (StatusCode::BAD_REQUEST, BAD_QUERY).into_response();
    }
    let email = q.get("email").cloned().unwrap_or_default();
    let now = now_secs();
    let stopped = *h.off_stopped_until.lock().unwrap();
    if now < stopped {
        return off_answer(&OffAnswer::Limited(stopped - now));
    }
    if let Err(wait) = allow(&h, client(&h, peer, &hd), |s| &mut s.off) {
        return off_answer(&OffAnswer::Limited(wait));
    }
    match blocking(&h, move || off::search(&text, &email)).await {
        Ok(answer) => {
            if let OffAnswer::Limited(secs) = answer {
                // one helper address for everyone: when OFF says wait, all of us wait
                let mut until = h.off_stopped_until.lock().unwrap();
                *until = (*until).max(now_secs().saturating_add(secs));
            }
            off_answer(&answer)
        }
        Err(no) => no,
    }
}

fn off_answer(answer: &OffAnswer) -> Response {
    match serde_json::to_vec(answer) {
        Ok(body) => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, BROKE).into_response(),
    }
}

/// Who is asking, for their limits. Behind a proxy, the last X-Forwarded-For entry (the proxy adds it;
/// earlier ones are whatever the client claimed). An IPv6 client is its /64, since one home gets a whole /64.
fn client(h: &Helper, peer: SocketAddr, hd: &HeaderMap) -> IpAddr {
    let forwarded = || {
        let last = hd.get_all("x-forwarded-for").iter().last()?.to_str().ok()?.rsplit(',').next()?.trim();
        // some proxies add the port: "1.2.3.4:567", "[2001:db8::1]:80"
        last.parse::<SocketAddr>().map(|a| a.ip()).or_else(|_| last.parse::<IpAddr>()).ok()
    };
    let ip = if h.behind_proxy { forwarded().unwrap_or(peer.ip()) } else { peer.ip() };
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

/// Count one send for this client, or say how many seconds they must wait.
fn allow(h: &Helper, who: IpAddr, which: fn(&mut Sends) -> &mut Limiter) -> Result<(), u64> {
    let mut clients = h.clients.lock().unwrap();
    if clients.len() >= MAX_CLIENTS && !clients.contains_key(&who) {
        // shortcut: a flood of new addresses resets everyone's count; keep per-client ages if that ever matters.
        clients.clear();
    }
    which(clients.entry(who).or_default()).allow(now_secs())
}

/// Seconds since the helper started, on a clock that never runs backwards: a changed system clock
/// can't stretch anyone's wait.
fn now_secs() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_secs()
}

/// Browsers send Origin on the web app's fetch but never on a typed or clicked link, so nobody
/// can hand out a helper address that downloads a file under the helper's name.
fn from_tiffin(h: &Helper, hd: &HeaderMap) -> Result<(), Response> {
    allowed_origin(h, hd).map(|_| ()).ok_or_else(|| (StatusCode::FORBIDDEN, NOT_TIFFIN).into_response())
}

/// Run one outside request on a worker thread: at most AT_ONCE at a time, answered within FETCH_LIMIT.
async fn blocking<T: Send + 'static>(h: &Helper, work: impl FnOnce() -> T + Send + 'static) -> Result<T, Response> {
    let Ok(slot) = h.slots.clone().try_acquire_owned() else {
        return Err((StatusCode::SERVICE_UNAVAILABLE, BUSY).into_response());
    };
    // The slot travels with the work, so a request that outlives FETCH_LIMIT still counts against AT_ONCE.
    let job = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        work()
    });
    match tokio::time::timeout(FETCH_LIMIT, job).await {
        Ok(Ok(done)) => Ok(done),
        Ok(Err(_)) => Err((StatusCode::INTERNAL_SERVER_ERROR, BROKE).into_response()),
        Err(_) => Err((StatusCode::GATEWAY_TIMEOUT, SLOW).into_response()),
    }
}

/// Every answer: never run, framed, sniffed, cached or embedded by other sites, and readable
/// only by the Tiffin web app's own origin.
async fn harden(State(h): State<Helper>, req: Request, next: Next) -> Response {
    let origin = allowed_origin(&h, req.headers());
    let mut res = next.run(req).await;
    let hd = res.headers_mut();
    hd.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    hd.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'; sandbox"));
    hd.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    hd.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    hd.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    hd.insert(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(o) = origin {
        hd.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, o);
    }
    res
}

/// The request's Origin when it is exactly one of the web app's.
fn allowed_origin(h: &Helper, hd: &HeaderMap) -> Option<HeaderValue> {
    hd.get(header::ORIGIN).filter(|o| o.to_str().is_ok_and(|o| h.origins.iter().any(|a| a == o))).cloned()
}

/// TIFFIN_ORIGIN: comma-separated, e.g. "https://tiffin.fyi,http://localhost:5173".
fn origins_from(v: Option<String>) -> Vec<String> {
    v.unwrap_or_else(|| "https://tiffin.fyi".into()).split(',').map(|o| o.trim().trim_end_matches('/').to_string()).filter(|o| !o.is_empty()).collect()
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("TIFFIN_HELPER_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".into());
    let origins = origins_from(std::env::var("TIFFIN_ORIGIN").ok());
    let behind_proxy = std::env::var("TIFFIN_BEHIND_PROXY").is_ok_and(|v| v == "1");
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Can't listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("tiffin-helper listening on {addr} for {}", origins.join(", "));
    let service = router(origins, AT_ONCE, behind_proxy).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service).await {
        eprintln!("tiffin-helper stopped: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::HeaderName;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn ask(app: Router, method: &str, uri: &str, origin: Option<&str>) -> (StatusCode, axum::http::HeaderMap, String) {
        ask_from(app, method, uri, origin, "203.0.113.9:5000", None).await
    }

    async fn ask_from(app: Router, method: &str, uri: &str, origin: Option<&str>, peer: &str, forwarded: Option<&str>) -> (StatusCode, axum::http::HeaderMap, String) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        if let Some(f) = forwarded {
            req = req.header("x-forwarded-for", f);
        }
        let mut req = req.body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(peer.parse::<SocketAddr>().unwrap()));
        let res = app.oneshot(req).await.unwrap();
        let (parts, body) = res.into_parts();
        let text = String::from_utf8_lossy(&body.collect().await.unwrap().to_bytes()).to_string();
        (parts.status, parts.headers, text)
    }

    fn helper() -> Router {
        app(origins_from(None), AT_ONCE)
    }

    const APP: Option<&str> = Some("https://tiffin.fyi");

    #[tokio::test]
    async fn only_the_web_app_gets_downloads() {
        for origin in [None, Some("https://evil.example"), Some("null")] {
            for uri in ["/page?url=https://example.com/", "/photo?url=https://example.com/a.exe"] {
                let (status, h, text) = ask(helper(), "GET", uri, origin).await;
                assert_eq!((status, text.as_str()), (StatusCode::FORBIDDEN, NOT_TIFFIN), "{uri} {origin:?}");
                assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
            }
        }
    }

    #[tokio::test]
    async fn refusals_carry_the_importer_message() {
        let cases = [
            ("/page?url=ftp://example.com/x", importer::ONLY_WEB),
            ("/page?url=file:///C:/Windows/win.ini", importer::ONLY_WEB),
            ("/page?url=http://localhost/", importer::NOT_LOCAL),
            ("/page?url=http://127.0.0.1:1/", importer::NOT_LOCAL),
            ("/page?url=http://%5B::1%5D/", importer::NOT_LOCAL),
            ("/page?url=http://%5B::ffff:127.0.0.1%5D/", importer::NOT_LOCAL),
            ("/photo?url=http://192.168.1.1/a.jpg", importer::NOT_LOCAL),
            ("/photo?url=http://169.254.169.254/latest/meta-data/", importer::NOT_LOCAL),
            ("/page?url=https://a:b@example.com/", importer::USER_IN_LINK),
        ];
        for (uri, why) in cases {
            let (status, _, text) = ask(helper(), "GET", uri, APP).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{uri}");
            assert!(text.starts_with(why), "{uri}: {text}");
        }
    }

    #[tokio::test]
    async fn a_missing_address_is_a_bad_request() {
        for uri in ["/page", "/page?url=", "/photo?url=%20", "/page?link=https://example.com/"] {
            let (status, _, text) = ask(helper(), "GET", uri, APP).await;
            assert_eq!((status, text.as_str()), (StatusCode::BAD_REQUEST, NO_ADDRESS), "{uri}");
        }
    }

    #[tokio::test]
    async fn only_get_and_only_its_routes() {
        let (status, h, _) = ask(helper(), "POST", "/page?url=https://example.com/", None).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let (status, h, _) = ask(helper(), "GET", "/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(h[header::CONTENT_SECURITY_POLICY], "default-src 'none'; frame-ancestors 'none'; sandbox");
    }

    #[tokio::test]
    async fn every_answer_is_inert_and_uncached() {
        let (_, h, _) = ask(helper(), "GET", "/page?url=ftp://x", APP).await;
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::CONTENT_SECURITY_POLICY], "default-src 'none'; frame-ancestors 'none'; sandbox");
        assert_eq!(h[header::CACHE_CONTROL], "no-store");
        assert_eq!(h[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(h["cross-origin-resource-policy"], "same-origin");
        assert!(h[header::CONTENT_TYPE].to_str().unwrap().starts_with("text/plain"));
    }

    #[tokio::test]
    async fn only_the_web_app_may_read_answers() {
        let (_, h, _) = ask(helper(), "GET", "/page?url=ftp://x", Some("https://tiffin.fyi")).await;
        assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "https://tiffin.fyi");
        assert_eq!(h[header::VARY], "Origin");
        for other in ["https://evil.example", "https://tiffin.fyi.evil.example", "null", "https://TIFFIN.FYI"] {
            let (_, h, _) = ask(helper(), "GET", "/page?url=ftp://x", Some(other)).await;
            assert!(h.get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none(), "{other}");
            assert_eq!(h[header::VARY], "Origin");
        }
        let dev = app(origins_from(Some(" https://tiffin.fyi/ , http://localhost:5173,,".into())), 1);
        let (_, h, _) = ask(dev, "GET", "/page?url=ftp://x", Some("http://localhost:5173")).await;
        assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "http://localhost:5173");
    }

    #[tokio::test]
    async fn off_search_takes_only_a_food_name_from_the_web_app() {
        let (status, _, text) = ask(helper(), "GET", "/off?q=nutella&email=a@b.co", None).await;
        assert_eq!((status, text.as_str()), (StatusCode::FORBIDDEN, NOT_TIFFIN));
        let long = format!("/off?q={}&email=a@b.co", "a".repeat(MAX_QUERY_CHARS + 1));
        for uri in ["/off", "/off?q=a&email=a@b.co", "/off?q=%20%20b%20&email=a@b.co", long.as_str()] {
            let (status, _, text) = ask(helper(), "GET", uri, APP).await;
            assert_eq!((status, text.as_str()), (StatusCode::BAD_REQUEST, BAD_QUERY), "{uri}");
        }
    }

    #[tokio::test]
    async fn off_search_without_a_contact_sends_nothing() {
        for uri in ["/off?q=nutella", "/off?q=nutella&email=nope", "/off?q=nutella&email=a@b.co%0d%0aX-Evil:%201"] {
            let (status, h, text) = ask(helper(), "GET", uri, APP).await;
            assert_eq!(status, StatusCode::OK, "{uri}");
            assert_eq!(h[header::CONTENT_TYPE], "application/json");
            assert_eq!(serde_json::from_str::<off::OffAnswer>(&text).unwrap(), off::OffAnswer::NeedsEmail, "{uri}");
            assert_eq!(h[header::CONTENT_SECURITY_POLICY], "default-src 'none'; frame-ancestors 'none'; sandbox");
            assert_eq!(h[header::ACCESS_CONTROL_ALLOW_ORIGIN], "https://tiffin.fyi");
        }
        // exactly the longest text allowed still gets an answer
        let edge = format!("/off?q={}", "a".repeat(MAX_QUERY_CHARS));
        let (status, _, _) = ask(helper(), "GET", &edge, APP).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn each_client_gets_eight_downloads_a_minute() {
        let h = helper();
        // refused links still count: each one is a request the helper had to judge
        for i in 0..off::OFF_PER_MINUTE {
            let (status, _, _) = ask(h.clone(), "GET", "/page?url=http://127.0.0.1/", APP).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "send {i}");
        }
        let (status, hd, text) = ask(h.clone(), "GET", "/photo?url=http://127.0.0.1/", APP).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        let wait: u64 = hd[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
        assert!((1..=60).contains(&wait), "{wait}");
        assert!(text.starts_with("Too many downloads from here."));
        // someone else is not held up, and neither is the same client's food search
        let (status, _, _) = ask_from(h.clone(), "GET", "/page?url=http://127.0.0.1/", APP, "198.51.100.4:1", None).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _, _) = ask(h, "GET", "/off?q=nutella", APP).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn each_client_gets_eight_searches_a_minute() {
        let h = helper();
        for _ in 0..off::OFF_PER_MINUTE {
            let (_, _, text) = ask(h.clone(), "GET", "/off?q=nutella", APP).await;
            assert_eq!(serde_json::from_str::<OffAnswer>(&text).unwrap(), OffAnswer::NeedsEmail);
        }
        let (status, _, text) = ask(h, "GET", "/off?q=nutella", APP).await;
        assert_eq!(status, StatusCode::OK);
        assert!(matches!(serde_json::from_str::<OffAnswer>(&text).unwrap(), OffAnswer::Limited(1..=60)), "{text}");
    }

    #[tokio::test]
    async fn when_off_says_stop_nobody_is_sent() {
        let h = Helper { origins: Arc::new(origins_from(None)), slots: Arc::new(Semaphore::new(1)), clients: Arc::default(), off_stopped_until: Arc::default(), behind_proxy: false };
        *h.off_stopped_until.lock().unwrap() = now_secs() + 120;
        let r = off_search(State(h.clone()), ConnectInfo("203.0.113.9:1".parse().unwrap()), [(header::ORIGIN, HeaderValue::from_static("https://tiffin.fyi"))].into_iter().collect(), Query(HashMap::from([("q".into(), "nutella".into())]))).await;
        let body = r.into_body().collect().await.unwrap().to_bytes();
        assert!(matches!(serde_json::from_slice::<OffAnswer>(&body).unwrap(), OffAnswer::Limited(119..=120)));
        assert!(h.clients.lock().unwrap().is_empty(), "nothing counted, nothing sent");
    }

    #[test]
    fn a_client_is_who_connected_unless_a_trusted_proxy_says() {
        let direct = Helper { origins: Arc::default(), slots: Arc::new(Semaphore::new(1)), clients: Arc::default(), off_stopped_until: Arc::default(), behind_proxy: false };
        let proxied = Helper { behind_proxy: true, ..direct.clone() };
        let peer: SocketAddr = "10.0.0.2:1".parse().unwrap();
        let fwd = |v: &str| -> HeaderMap { [(HeaderName::from_static("x-forwarded-for"), HeaderValue::from_str(v).unwrap())].into_iter().collect() };
        // without a proxy the header is the client's own claim, so it is ignored
        assert_eq!(client(&direct, peer, &fwd("8.8.8.8")), "10.0.0.2".parse::<IpAddr>().unwrap());
        // behind one, only the entry the proxy added counts; a client can't pick its own
        assert_eq!(client(&proxied, peer, &fwd("1.1.1.1, 198.51.100.7")), "198.51.100.7".parse::<IpAddr>().unwrap());
        assert_eq!(client(&proxied, peer, &fwd("not an address")), "10.0.0.2".parse::<IpAddr>().unwrap());
        assert_eq!(client(&proxied, peer, &fwd("1.1.1.1, 198.51.100.7:567")), "198.51.100.7".parse::<IpAddr>().unwrap());
        assert_eq!(client(&proxied, peer, &fwd("[2001:db8:5::1]:80")), "2001:db8:5::".parse::<IpAddr>().unwrap());
        // two header lines: the proxy's is the last one
        let two: HeaderMap = [("x-forwarded-for", "8.8.8.8"), ("x-forwarded-for", "198.51.100.7")].into_iter().map(|(k, v)| (HeaderName::from_static(k), HeaderValue::from_static(v))).collect();
        assert_eq!(client(&proxied, peer, &two), "198.51.100.7".parse::<IpAddr>().unwrap());
        assert_eq!(client(&proxied, peer, &HeaderMap::new()), "10.0.0.2".parse::<IpAddr>().unwrap());
        // one home's IPv6 /64 is one client; a mapped IPv4 address is that IPv4 client
        let six = |a: &str| client(&direct, SocketAddr::new(a.parse().unwrap(), 1), &HeaderMap::new());
        assert_eq!(six("2001:db8:1:2:aaaa::1"), six("2001:db8:1:2:bbbb::9"));
        assert_ne!(six("2001:db8:1:2::1"), six("2001:db8:1:3::1"));
        assert_eq!(six("::ffff:203.0.113.9"), "203.0.113.9".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn a_full_helper_says_busy_without_fetching() {
        let (status, _, text) = ask(app(origins_from(None), 0), "GET", "/page?url=https://example.com/", APP).await;
        assert_eq!((status, text.as_str()), (StatusCode::SERVICE_UNAVAILABLE, BUSY));
        let (status, _, text) = ask(app(origins_from(None), 0), "GET", "/off?q=nutella&email=a@b.co", APP).await;
        assert_eq!((status, text.as_str()), (StatusCode::SERVICE_UNAVAILABLE, BUSY));
    }
}
