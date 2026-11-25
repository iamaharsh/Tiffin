//! Timers, step-by-step cook mode and recipe videos (logic only; drawn in ui.rs).

/// A new timer starts at 5 minutes (300 s).
pub const NEW_TIMER_SECS: u32 = 300;

/// Hours/minutes/seconds steppers → seconds: h×3600 + m×60 + s.
pub fn timer_seconds(h: u32, m: u32, s: u32) -> u32 {
    h * 3600 + m * 60 + s
}

/// Seconds → hours/minutes/seconds steppers.
pub fn timer_parts(secs: u32) -> (u32, u32, u32) {
    (secs / 3600, (secs % 3600) / 60, secs % 60)
}

#[derive(Clone, Debug)]
pub struct Timer {
    pub label: String,
    pub total: f64,
    pub remaining: f64,
    pub running: bool,
    pub done: bool,
}

impl Timer {
    pub fn new(label: &str, seconds: u32) -> Timer {
        Timer { label: label.into(), total: seconds as f64, remaining: seconds as f64, running: true, done: false }
    }
    /// Advance by `dt` seconds. Returns true when it just finished.
    pub fn tick(&mut self, dt: f64) -> bool {
        if !self.running || self.done {
            return false;
        }
        self.remaining -= dt;
        if self.remaining <= 0.0 {
            self.remaining = 0.0;
            self.running = false;
            self.done = true;
            return true;
        }
        false
    }
    pub fn toggle(&mut self) {
        if !self.done {
            self.running = !self.running;
        }
    }
    pub fn reset(&mut self) {
        self.remaining = self.total;
        self.done = false;
        self.running = false;
    }
    pub fn display(&self) -> String {
        let s = self.remaining.ceil() as u64;
        if s >= 3600 {
            format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
        } else {
            format!("{}:{:02}", s / 60, s % 60)
        }
    }
}

/// Step-by-step mode state.
#[derive(Clone, Debug, Default)]
pub struct StepMode {
    pub recipe_id: u64,
    pub steps: Vec<String>,
    pub index: usize,
    pub show_ingredients: bool,
}

impl StepMode {
    pub fn new(recipe_id: u64, steps: Vec<String>) -> StepMode {
        StepMode { recipe_id, steps, index: 0, show_ingredients: false }
    }
    pub fn next(&mut self) {
        if self.index + 1 < self.steps.len() {
            self.index += 1;
        }
    }
    pub fn prev(&mut self) {
        self.index = self.index.saturating_sub(1);
    }
    pub fn current(&self) -> &str {
        self.steps.get(self.index).map(|s| s.as_str()).unwrap_or("")
    }
    pub fn progress(&self) -> f32 {
        if self.steps.is_empty() { 0.0 } else { (self.index + 1) as f32 / self.steps.len() as f32 }
    }
    pub fn timers_in_step(&self) -> Vec<crate::ingredients::TimeSpan> {
        crate::ingredients::find_times(self.current())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum VideoKind {
    YouTube(String),
    Vimeo(String),
    File,
    Page,
}

pub fn video_kind(url: &str) -> Option<VideoKind> {
    let u = url.trim();
    if !(u.starts_with("http://") || u.starts_with("https://")) {
        return None;
    }
    let lc = u.to_lowercase();
    if lc.contains("youtube.com/watch") {
        let id = u.split("v=").nth(1)?.split(['&', '#']).next()?.to_string();
        return Some(VideoKind::YouTube(id));
    }
    if lc.contains("youtu.be/") || lc.contains("youtube.com/embed/") || lc.contains("youtube.com/shorts/") {
        let id = u.rsplit('/').next()?.split(['?', '&']).next()?.to_string();
        return Some(VideoKind::YouTube(id));
    }
    if lc.contains("vimeo.com/") {
        let id = u.rsplit('/').next()?.split('?').next()?.to_string();
        if id.chars().all(|c| c.is_ascii_digit()) {
            return Some(VideoKind::Vimeo(id));
        }
    }
    if [".mp4", ".webm", ".mov", ".m4v"].iter().any(|e| lc.split('?').next().unwrap_or("").ends_with(e)) {
        return Some(VideoKind::File);
    }
    Some(VideoKind::Page)
}

/// The address the video player opens, starting at `start_secs`.
pub fn video_play_url(url: &str, start_secs: u32) -> Option<String> {
    Some(match video_kind(url)? {
        VideoKind::YouTube(id) => format!("https://www.youtube.com/watch?v={}&t={}s", id, start_secs),
        VideoKind::Vimeo(id) => format!("https://vimeo.com/{}#t={}s", id, start_secs),
        VideoKind::File => format!("{}#t={}", url.trim(), start_secs),
        VideoKind::Page => url.trim().to_string(),
    })
}

/// Open an address with the system's default program (browser / video player).
pub fn open_external(target: &str) -> bool {
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(target).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(target).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(target).spawn();
    #[cfg(not(any(windows, unix)))]
    let r: Result<(), ()> = { let _ = target; Err(()) }; // the browser build opens links itself
    r.is_ok()
}

/// Amounts being tried out on one recipe (ingredient line index → amount). Never saved unless
/// the person presses Save.
#[derive(Clone, Debug)]
pub struct Trial {
    pub recipe_id: u64,
    pub amounts: std::collections::BTreeMap<usize, f64>,
}

impl Trial {
    /// A copy of the recipe with the trial amounts written in (for nutrition).
    pub fn apply(&self, r: &crate::model::Recipe) -> crate::model::Recipe {
        let lines: Vec<String> = r
            .ingredient_lines()
            .iter()
            .enumerate()
            .map(|(i, l)| match self.amounts.get(&i) {
                Some(q) => crate::ingredients::replace_amount(l, *q),
                None => l.to_string(),
            })
            .collect();
        crate::model::Recipe { ingredients: lines.join("\n"), ..r.clone() }
    }
}
