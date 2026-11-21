//! The journal: diary month files (`journal/YYYY-MM.json`) and the profile (`journal/profile.json`)
//! kept beside the library. Tracker data never goes into library.json, so a diary change never
//! rewrites the recipes and a recipe change never rewrites the diary.

use crate::model::{now_secs, MealType, Nutrients};
use crate::storage::{self, Problem};
use crate::tracker::day::is_month_key;
use crate::tracker::{body, goals};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The number that decides whether a month file is too new for this build. Bump it
/// only for a change an older build would lose when it re-saves one of these files (for example a
/// new kind of record in month files), exactly as `LIBRARY_FORMAT` does for the library.
/// 2: month files hold the complete-day flags (03-02); a format-1 build would drop them when it re-saves or backs up a month, so it opens a format-2 file read-only.
/// 3: month files hold exercise and body records (04-01); a format-2 build would drop them when it re-saves or backs up a month, so it opens a format-3 file read-only.
pub const JOURNAL_FORMAT: u32 = 3;

/// profile.json has its own format number, so a month-file change never changes it.
/// 2: profile.json holds the body details, dated targets, unit choices, custom activities and custom measures (04-01).
pub const PROFILE_FORMAT: u32 = 2;

/// The format number a journal file is checked against: profile.json has its own.
fn format_of(key: &str) -> u32 {
    if key == "profile" {
        PROFILE_FORMAT
    } else {
        JOURNAL_FORMAT
    }
}

/// Where a diary entry's food came from. Recipes are referenced by uid, never by numeric id.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
pub enum FoodRef {
    /// A food in the bundled database, by its key.
    Bundled(String),
    /// One of the user's My Foods, by name.
    MyFood(String),
    /// An Open Food Facts product, by barcode.
    Off(String),
    /// One of the user's recipes, by uid.
    Recipe(String),
    /// Typed in on the spot.
    #[default]
    Custom,
}

/// One logged food. The nutrition is frozen at log time: later edits to a recipe, My Food or the
/// database never change an entry that is already in the diary.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct DiaryEntry {
    pub uid: String,
    /// A valid `YYYY-MM-DD` day (see `Day::parse`), the local day the food was eaten.
    pub day: String,
    pub slot: MealType,
    pub logged_at: u64,
    pub modified: u64,
    /// Set when the entry is deleted, so a deletion survives a sync merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<u64>,
    pub name: String,
    pub source: FoodRef,
    /// Grams eaten, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grams: Option<f32>,
    pub amount: f32,
    pub unit: String,
    /// The nutrition the entry was logged with: per 100 g when `per_100g`, otherwise for one `unit` (one serving), and the entry holds `amount` of them.
    pub basis: Nutrients,
    pub per_100g: bool,
}

/// What was exercised with. A source names where the activity came from; the numbers it was logged with
/// are in the entry's [`ExerciseBasis`], so a later change to a table or a custom activity never moves a logged one.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum ExerciseSource {
    /// A row of the bundled activity table (`mets.tsv`), by its id.
    Compendium(String),
    /// One of the user's own activities, by `CustomActivity.uid`.
    Custom(String),
    /// Reserved for the future Workouts tab (D-17d); nothing writes it yet.
    Workout(String),
}

impl Default for ExerciseSource {
    fn default() -> Self {
        ExerciseSource::Custom(String::new())
    }
}

/// How an activity turns minutes into energy.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub enum ExerciseBasis {
    /// A MET value; the energy above resting is (MET - 1) x kg x hours.
    Met(f64),
    /// Net kcal an hour, already above resting.
    KcalPerHour(f64),
}

impl Default for ExerciseBasis {
    fn default() -> Self {
        ExerciseBasis::Met(1.0)
    }
}

/// One logged workout. Frozen at log time (D-17b): the shown kcal is `kcal_override` or else `est_kcal`;
/// None is unknown, never 0.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct ExerciseEntry {
    pub uid: String,
    /// A valid `YYYY-MM-DD` day (see `Day::parse`).
    pub day: String,
    pub logged_at: u64,
    pub modified: u64,
    /// Set when the entry is deleted, so a deletion survives a sync merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<u64>,
    pub name: String,
    pub source: ExerciseSource,
    pub basis: ExerciseBasis,
    pub minutes: f64,
    /// The body weight the estimate was made with, when there was one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weight_kg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_kcal: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kcal_override: Option<f64>,
}

/// One body measurement. `value` is canonical (kg, cm, percent or a plain number) and converted only at the
/// edge; there is one live entry per (day, measure).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct BodyEntry {
    pub uid: String,
    pub day: String,
    pub logged_at: u64,
    pub modified: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<u64>,
    /// A `body::BUILT_IN` id or a `CustomMeasure.uid`.
    pub measure: String,
    pub value: f64,
}

/// An activity the user made up, kept apart from the cited table (D-15).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Default)]
#[serde(default)]
pub struct CustomActivity {
    pub uid: String,
    pub name: String,
    pub basis: ExerciseBasis,
}

/// What one diary change replaced, so it can be put back. `before` holds the entries as they were and
/// `added` the uids the change created. The exercise and body lists do the same for those records: one undo covers
/// food, exercise and body together, so Copy Yesterday is one step back (D-17c).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct DiaryUndo {
    pub day: String,
    pub before: Vec<DiaryEntry>,
    pub added: Vec<String>,
    pub before_exercise: Vec<ExerciseEntry>,
    pub added_exercise: Vec<String>,
    pub before_body: Vec<BodyEntry>,
    pub added_body: Vec<String>,
}

/// One month: the diary, the workouts and the body measurements logged in it (exercise and body are format 3).
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct JournalMonth {
    pub format: u32,
    /// `YYYY-MM`
    pub month: String,
    pub entries: Vec<DiaryEntry>,
    /// The days (`YYYY-MM-DD`) marked complete. A day stays editable.
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub complete: BTreeSet<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exercise: Vec<ExerciseEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<BodyEntry>,
}

impl Default for JournalMonth {
    fn default() -> Self {
        JournalMonth { format: JOURNAL_FORMAT, month: String::new(), entries: vec![], complete: BTreeSet::new(), exercise: vec![], body: vec![] }
    }
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

fn is_default_rate(v: &f64) -> bool {
    *v == DEFAULT_RATE_KG_WEEK
}

/// The weekly rate a new profile starts with.
const DEFAULT_RATE_KG_WEEK: f64 = 0.5;

/// The user's body details, goal, dated targets and own lists. There is no weight field: the body log is the
/// only source of weight (D-06), so a weight is never typed twice. Heights, weights and rates are kg or cm;
/// the unit is in the name or the type and a value is converted only at the edge.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub format: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sex: Option<goals::Sex>,
    /// `YYYY-MM-DD`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_cm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<goals::Activity>,
    #[serde(skip_serializing_if = "is_default")]
    pub goal: goals::GoalKind,
    /// The weekly change asked for, in kg a week, whatever unit the page shows.
    #[serde(skip_serializing_if = "is_default_rate")]
    pub rate_kg_week: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal_weight_kg: Option<f64>,
    #[serde(skip_serializing_if = "is_default")]
    pub units: goals::UnitChoice,
    /// Dated full snapshots, ascending by `effective_from`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<goals::TargetSet>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub activities: Vec<CustomActivity>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<body::CustomMeasure>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile {
            format: PROFILE_FORMAT,
            sex: None,
            birth_date: None,
            height_cm: None,
            activity: None,
            goal: goals::GoalKind::default(),
            rate_kg_week: DEFAULT_RATE_KG_WEEK,
            goal_weight_kg: None,
            units: goals::UnitChoice::default(),
            targets: vec![],
            activities: vec![],
            measures: vec![],
        }
    }
}

/// The journal folder inside a data folder.
pub fn journal_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("journal")
}

/// At most one history snapshot of a journal file per this many seconds (6 hours).
pub const JOURNAL_SNAPSHOT_GAP: u64 = 6 * 3600;
/// Snapshots kept per file in `journal/history`; the oldest go first.
pub const JOURNAL_SNAPSHOTS_KEPT: usize = 28;

/// Month files are read when first asked for and written when changed. Each file guards itself: a
/// damaged or newer one is locked alone (see [`Journal::problems`]), so the others keep working.
/// The journal never touches the library's own lock.
pub struct Journal {
    dir: PathBuf,
    months: BTreeMap<String, JournalMonth>,
    dirty: BTreeSet<String>,
    profile: Option<Profile>,
    profile_dirty: bool,
    /// Files that can't be changed, by key ("2026-11" or "profile").
    problems: Vec<(String, Problem)>,
    /// Files opened from their `.bak`, with the kept copy of the damaged file (None when it was missing).
    pub recovered: Vec<(String, Option<PathBuf>)>,
    /// When each file last got a history snapshot, by key.
    last_snapshot: BTreeMap<String, u64>,
    /// The pending one-level undo. It lives here, not on the app state, so opening a new journal (a backup restore) drops it.
    undo: Option<DiaryUndo>,
    /// The undo as of the last good save; [`Journal::discard_unsaved`] puts it back.
    saved_undo: Option<DiaryUndo>,
}

/// Only the format of a journal file, read without caring what else it holds.
#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    format: u32,
}

const NO_UPGRADE_COPY: &str = "Couldn't keep a copy of this diary month before updating it, so nothing was saved. Make sure the data folder isn't full, then try again.";

/// What reading one journal file found.
enum FileRead<T> {
    Missing,
    Ok(T),
    Bad,
    /// `format` above what this build reads; holds the version that wrote it.
    TooNew(String),
}

fn read_file<T: DeserializeOwned>(path: &Path, current: u32) -> FileRead<T> {
    match fs::read(path) {
        Ok(bytes) => match storage::newer_than(&bytes, current) {
            Some(found) => FileRead::TooNew(found),
            None => serde_json::from_slice(&bytes).map_or(FileRead::Bad, FileRead::Ok),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileRead::Missing,
        Err(_) => FileRead::Bad,
    }
}

impl Journal {
    /// A journal over a folder. Reads nothing.
    pub fn open(dir: PathBuf) -> Journal {
        Journal { dir, months: BTreeMap::new(), dirty: BTreeSet::new(), profile: None, profile_dirty: false, problems: vec![], recovered: vec![], last_snapshot: BTreeMap::new(), undo: None, saved_undo: None }
    }

    /// Keep `u` as the one pending undo. The slot changes only together with a successful save.
    pub fn set_undo(&mut self, u: DiaryUndo) {
        self.undo = Some(u);
    }

    /// Take the pending undo, leaving none.
    pub fn take_undo(&mut self) -> Option<DiaryUndo> {
        self.undo.take()
    }

    /// After a failed change, make memory match the files again: every changed month is dropped (the next read
    /// loads it from disk), the profile too, and the pending undo goes back to the one from the last good save.
    pub fn discard_unsaved(&mut self) {
        for m in std::mem::take(&mut self.dirty) {
            self.months.remove(&m);
        }
        if self.profile_dirty {
            self.profile = None;
            self.profile_dirty = false;
        }
        self.undo = self.saved_undo.clone();
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }

    /// The files that are locked, by key ("2026-11" or "profile"), with why.
    pub fn problems(&self) -> &[(String, Problem)] {
        &self.problems
    }

    fn locked(&self, key: &str) -> Option<&Problem> {
        self.problems.iter().find(|(k, _)| k == key).map(|(_, p)| p)
    }

    /// Lock one file and give the sentence for refusing a change to it.
    fn lock(&mut self, key: &str, p: Problem) -> String {
        let said = p.refusal().to_string();
        self.problems.push((key.into(), p));
        said
    }

    /// Read one file the way the library is read: a good file is used; a newer one locks; a damaged one is
    /// copied aside first and then the `.bak` is used if it reads; otherwise it locks. `fresh` is for a file that doesn't exist.
    fn load<T: DeserializeOwned>(&mut self, key: &str, fresh: T) -> Result<T, String> {
        let main = self.path(key);
        let bak = main.with_extension("json.bak");
        let too_new = |found| Problem::TooNew { found, partial: false };
        let cur = format_of(key);
        match read_file::<T>(&main, cur) {
            FileRead::Ok(v) => Ok(v),
            FileRead::TooNew(found) => Err(self.lock(key, too_new(found))), // never read or touch the .bak
            FileRead::Missing => match read_file::<T>(&bak, cur) {
                FileRead::Ok(v) => {
                    self.recovered.push((key.into(), None));
                    Ok(v)
                }
                FileRead::Missing => Ok(fresh),
                FileRead::TooNew(found) => Err(self.lock(key, too_new(found))),
                FileRead::Bad => {
                    let kept = storage::quarantine(&bak).ok();
                    Err(self.lock(key, Problem::Damaged { file: bak, kept }))
                }
            },
            FileRead::Bad => {
                let kept = storage::quarantine(&main).ok();
                match read_file::<T>(&bak, cur) {
                    FileRead::Ok(v) if kept.is_some() => {
                        self.recovered.push((key.into(), kept));
                        Ok(v)
                    }
                    FileRead::TooNew(found) => Err(self.lock(key, too_new(found))),
                    other => {
                        if matches!(other, FileRead::Bad) {
                            let _ = storage::quarantine(&bak);
                        }
                        Err(self.lock(key, Problem::Damaged { file: main, kept }))
                    }
                }
            }
        }
    }

    fn ensure_month(&mut self, m: &str) -> Result<(), String> {
        if !is_month_key(m) {
            return Err(format!("\"{m}\" isn't a valid month (use YYYY-MM)."));
        }
        if let Some(p) = self.locked(m) {
            return Err(p.refusal().into());
        }
        if !self.months.contains_key(m) {
            let month = self.load(m, JournalMonth { month: m.into(), ..Default::default() })?;
            self.months.insert(m.into(), month);
        }
        Ok(())
    }

    /// One month for reading; an empty month when there is no file yet. Err while that month's file is locked.
    pub fn month(&mut self, m: &str) -> Result<&JournalMonth, String> {
        self.ensure_month(m)?;
        Ok(&self.months[m])
    }

    /// One month for changing. It is marked changed and written by the next [`Journal::save_dirty`].
    pub fn month_mut(&mut self, m: &str) -> Result<&mut JournalMonth, String> {
        self.ensure_month(m)?;
        self.dirty.insert(m.into());
        let month = self.months.get_mut(m).unwrap();
        month.format = JOURNAL_FORMAT; // a month this build changes is written in this build's format
        Ok(month)
    }

    fn ensure_profile(&mut self) -> Result<(), String> {
        if let Some(p) = self.locked("profile") {
            return Err(p.refusal().into());
        }
        if self.profile.is_none() {
            self.profile = Some(self.load("profile", Profile::default())?);
        }
        Ok(())
    }

    /// The profile for reading. The diary reads it on every view, and a read must never rewrite profile.json,
    /// so this does not mark it changed. Err while profile.json is locked.
    pub fn profile(&mut self) -> Result<&Profile, String> {
        self.ensure_profile()?;
        Ok(self.profile.as_ref().unwrap())
    }

    /// The profile for changing; marked changed. Err while profile.json is locked.
    pub fn profile_mut(&mut self) -> Result<&mut Profile, String> {
        self.ensure_profile()?;
        self.profile_dirty = true;
        let p = self.profile.as_mut().unwrap();
        p.format = PROFILE_FORMAT; // a profile this build changes is written in this build's format
        Ok(p)
    }

    /// Write every changed month and the profile. Each file is read again first, so what is on disk
    /// now decides how it is replaced (see [`Journal::save_key`]). A file that fails stays changed and
    /// the others are still written; the first error is returned.
    pub fn save_dirty(&mut self) -> Result<(), String> {
        let mut first_err = None;
        let mut jobs = vec![];
        for m in &self.dirty {
            jobs.push((m.clone(), serde_json::to_string_pretty(&self.months[m]).map_err(|e| e.to_string())?));
        }
        for (key, text) in jobs {
            let r = self.save_key::<JournalMonth>(&key, &text);
            if r.is_ok() || self.locked(&key).is_some() {
                self.dirty.remove(&key);
            }
            first_err = first_err.or(r.err());
        }
        if self.profile_dirty {
            let text = serde_json::to_string_pretty(self.profile.as_ref().unwrap()).map_err(|e| e.to_string())?;
            let r = self.save_key::<Profile>("profile", &text);
            if r.is_ok() || self.locked("profile").is_some() {
                self.profile_dirty = false;
            }
            first_err = first_err.or(r.err());
        }
        if first_err.is_none() {
            self.saved_undo = self.undo.clone();
        }
        first_err.map_or(Ok(()), Err)
    }

    /// The gate in front of one file. A good file gets a history snapshot if one is due and is rotated into
    /// `.bak`; a file found unreadable is copied aside first and never rotated (or nothing is written);
    /// a file from a newer build is locked and left exactly as it is.
    fn save_key<T: DeserializeOwned>(&mut self, key: &str, text: &str) -> Result<(), String> {
        // what is about to be written must read back (a NaN or infinity in a number is written as null, and the file would then be damaged)
        if serde_json::from_str::<T>(text).is_err() {
            return Err("Couldn't save the diary: it holds a number that can't be stored. Your earlier entries are untouched.".into());
        }
        let path = self.path(key);
        let keep_bak = match read_file::<T>(&path, format_of(key)) {
            FileRead::Missing => false,
            FileRead::Ok(_) => {
                // a file in an older format is copied first whatever the snapshot window says, and a failed copy stops the write (snapshot before migrations)
                if fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Header>(&b).ok()).is_some_and(|h| h.format < format_of(key)) {
                    self.copy_to_history(key, &path).map_err(|_| NO_UPGRADE_COPY.to_string())?;
                } else {
                    self.snapshot_if_due(key, &path);
                }
                true
            }
            FileRead::Bad => {
                storage::quarantine(&path)?; // Err(NO_SAFETY_COPY): the file stays as it is and the change stays pending
                false
            }
            FileRead::TooNew(found) => return Err(self.lock(key, Problem::TooNew { found, partial: false })),
        };
        storage::write_atomic(&path, text.as_bytes(), keep_bak)
    }

    /// Copy the good file to `history/<key>-<secs>.json` once per [`JOURNAL_SNAPSHOT_GAP`], then drop the
    /// oldest beyond [`JOURNAL_SNAPSHOTS_KEPT`]. Best effort: a history folder that can't be written doesn't stop the diary.
    fn snapshot_if_due(&mut self, key: &str, path: &Path) {
        let now = now_secs();
        if self.last_snapshot.get(key).is_some_and(|t| now.saturating_sub(*t) < JOURNAL_SNAPSHOT_GAP) {
            return;
        }
        self.last_snapshot.insert(key.into(), now);
        let _ = self.copy_to_history(key, path);
    }

    /// Copy the file's exact bytes to `history/<key>-<secs>.json`, then drop the oldest beyond [`JOURNAL_SNAPSHOTS_KEPT`].
    fn copy_to_history(&mut self, key: &str, path: &Path) -> Result<(), String> {
        let now = now_secs();
        let hist = self.dir.join("history");
        let copied = fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| storage::write_atomic(&hist.join(format!("{key}-{now}.json")), &bytes, false));
        if copied.is_ok() {
            self.last_snapshot.insert(key.into(), now);
        }
        let prefix = format!("{key}-");
        let mut mine: Vec<(u64, PathBuf)> = fs::read_dir(&hist)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let secs = name.strip_prefix(&prefix)?.strip_suffix(".json")?;
                Some((secs.parse::<u64>().ok()?, e.path()))
            })
            .collect();
        mine.sort();
        for (_, old) in mine.iter().take(mine.len().saturating_sub(JOURNAL_SNAPSHOTS_KEPT)) {
            let _ = fs::remove_file(old);
        }
        copied
    }

    /// The months that have a file, sorted. Only `YYYY-MM.json` counts: backups, temp files, damaged copies, the profile and the history folder are ignored.
    pub fn months_on_disk(&self) -> Vec<String> {
        let mut out: Vec<String> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".json").filter(|m| is_month_key(m)).map(String::from))
            .collect();
        out.sort();
        out
    }

    /// Every month that has a file or is held in memory (a month changed but not yet saved counts), sorted.
    pub fn months_with_data(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = self.months_on_disk().into_iter().collect();
        out.extend(self.months.keys().cloned());
        out.into_iter().collect()
    }
}

/// The diary part of a one-file backup: every month file and the profile, as they are on disk.
/// Phase 4 adds exercise and body records here the same way (they live in the month files).
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
#[serde(default)]
pub struct JournalBundle {
    pub months: BTreeMap<String, JournalMonth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Profile>,
}

/// Read every month file and the profile straight from disk (not only the ones loaded). A file that can't
/// be read, or is from a newer build, is left out and its key returned. None when nothing was read, so a
/// backup of an empty install has no diary section and never offers to replace a real diary.
pub fn bundle_from_disk(dir: &Path) -> (Option<JournalBundle>, Vec<String>) {
    let mut b = JournalBundle::default();
    let mut left_out = vec![];
    for key in Journal::open(dir.to_path_buf()).months_on_disk() {
        match read_file::<JournalMonth>(&dir.join(format!("{key}.json")), JOURNAL_FORMAT) {
            FileRead::Ok(m) => {
                b.months.insert(key, m);
            }
            FileRead::Missing => {}
            _ => left_out.push(key),
        }
    }
    match read_file::<Profile>(&dir.join("profile.json"), PROFILE_FORMAT) {
        FileRead::Ok(p) => b.profile = Some(p),
        FileRead::Missing => {}
        _ => left_out.push("profile".into()),
    }
    ((!b.months.is_empty() || b.profile.is_some()).then_some(b), left_out)
}

/// Refuse a diary from a backup that could do harm, before anything changes: a month key that isn't
/// `YYYY-MM` (it would become a file name, so `../../x` is rejected, never cleaned up) or a file from
/// a newer build (restoring it would drop what this build doesn't know).
pub fn check_bundle(b: &JournalBundle) -> Result<(), String> {
    if let Some(bad) = b.months.keys().find(|k| !is_month_key(k)) {
        return Err(format!("This backup has a diary month with a bad name ({bad}), so nothing was restored."));
    }
    if b.months.values().any(|m| m.format > JOURNAL_FORMAT) || b.profile.as_ref().is_some_and(|p| p.format > PROFILE_FORMAT) {
        return Err(storage::NEWER_BACKUP.into());
    }
    Ok(())
}

/// Replace the diary in `dir` with the bundle. First every file in `dir` is copied into
/// `history/restore-<stamp>/` and each copy's size checked; nothing is written if that fails. Then the
/// bundle's months and profile are written, and months (and the profile) the bundle lacks are removed,
/// which is safe because they are already in the copy folder. Returns the copy folder.
pub fn restore_bundle(dir: &Path, b: &JournalBundle) -> Result<PathBuf, String> {
    check_bundle(b)?;
    let base = format!("restore-{}", storage::fmt_stamp(now_secs()).replace([':', ' '], "-"));
    let mut kept = dir.join("history").join(&base);
    let mut n = 2;
    while kept.exists() {
        kept = dir.join("history").join(format!("{base}-{n}"));
        n += 1;
    }
    let no_copy = "Couldn't keep a copy of your current diary, so nothing was restored. Make sure the data folder isn't full, then try again.";
    let files: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_file())).map(|e| e.path()).collect();
    for f in &files {
        let to = kept.join(f.file_name().unwrap());
        let ok = fs::create_dir_all(&kept).and_then(|_| fs::copy(f, &to)).is_ok() && matches!((fs::metadata(f), fs::metadata(&to)), (Ok(a), Ok(c)) if a.len() == c.len());
        if !ok {
            return Err(no_copy.into());
        }
    }
    let stopped = |e: String| format!("The diary restore stopped part way: {e}. Your diary as it was before is kept in {}.", kept.display());
    for (key, m) in &b.months {
        let text = serde_json::to_string_pretty(m).map_err(|e| stopped(e.to_string()))?;
        storage::write_atomic(&dir.join(format!("{key}.json")), text.as_bytes(), false).map_err(stopped)?;
    }
    if let Some(p) = &b.profile {
        let text = serde_json::to_string_pretty(p).map_err(|e| stopped(e.to_string()))?;
        storage::write_atomic(&dir.join("profile.json"), text.as_bytes(), false).map_err(stopped)?;
    }
    let mut gone: Vec<String> = Journal::open(dir.to_path_buf()).months_on_disk().into_iter().filter(|m| !b.months.contains_key(m)).collect();
    if b.profile.is_none() {
        gone.push("profile".into());
    }
    for key in gone {
        for ext in ["json", "json.bak"] {
            let f = dir.join(format!("{key}.{ext}"));
            if f.exists() {
                fs::remove_file(&f).map_err(|e| stopped(e.to_string()))?;
            }
        }
    }
    Ok(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rb-journal-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn entry(name: &str) -> DiaryEntry {
        DiaryEntry { day: "2026-11-13".into(), name: name.into(), basis: Nutrients { calories: Some(150.0), ..Default::default() }, ..Default::default() }
    }

    #[test]
    fn a_month_that_would_not_read_back_is_not_written() {
        let d = tdir("unreadable");
        let mut j = Journal::open(d.clone());
        j.month_mut("2026-11").unwrap().entries.push(entry("Soup"));
        j.save_dirty().unwrap();
        let before = fs::read(d.join("2026-11.json")).unwrap();
        j.month_mut("2026-11").unwrap().entries.push(DiaryEntry { amount: f32::INFINITY, ..entry("Bad") });
        assert!(j.save_dirty().is_err());
        assert_eq!(fs::read(d.join("2026-11.json")).unwrap(), before, "the good file is left alone");
        assert!(copies(&d, "2026-11").is_empty(), "and nothing is quarantined");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn failed_change_is_not_kept_in_memory() {
        let d = tdir("rollback");
        let mut j = Journal::open(d.clone());
        j.month_mut("2026-11").unwrap().entries.push(entry("A"));
        j.save_dirty().unwrap();
        j.month_mut("2026-11").unwrap().entries.push(DiaryEntry { amount: f32::INFINITY, ..entry("Bad") });
        assert!(j.save_dirty().is_err());
        j.discard_unsaved();
        let names = |j: &mut Journal| j.month("2026-11").unwrap().entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&mut j), ["A"], "read from disk again");
        j.month_mut("2026-11").unwrap().entries.push(entry("B"));
        j.save_dirty().unwrap();
        assert_eq!(names(&mut Journal::open(d.clone())), ["A", "B"], "the next good change saves without the bad entry");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn undo_slot_follows_the_last_good_save() {
        let d = tdir("undoslot");
        let mut j = Journal::open(d.clone());
        let u = |day: &str| DiaryUndo { day: day.into(), ..Default::default() };
        j.month_mut("2026-11").unwrap().entries.push(entry("A"));
        j.set_undo(u("x"));
        j.save_dirty().unwrap();
        j.month_mut("2026-11").unwrap().entries.push(DiaryEntry { amount: f32::INFINITY, ..entry("Bad") });
        j.set_undo(u("y"));
        assert!(j.save_dirty().is_err());
        j.discard_unsaved();
        assert_eq!(j.take_undo(), Some(u("x")));
        assert_eq!(j.take_undo(), None);
        j.set_undo(u("z"));
        assert_eq!(Journal::open(d.clone()).take_undo(), None, "a fresh journal (a restore) has no undo");
        let _ = fs::remove_dir_all(&d);
    }

    const FORMAT_1: &str = r#"{"format": 1, "month": "2026-11", "entries": [{"uid": "u1", "day": "2026-11-13", "name": "Soup"}]}"#;

    fn history_files(d: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(d.join("history")).into_iter().flatten().flatten().map(|e| e.path()).collect();
        v.sort();
        v
    }

    #[test]
    fn older_month_loads_and_is_written_in_the_current_format() {
        let d = tdir("fmt1");
        fs::write(d.join("2026-11.json"), FORMAT_1).unwrap();
        let mut j = Journal::open(d.clone());
        let m = j.month("2026-11").unwrap();
        assert_eq!((m.entries.len(), m.complete.len(), m.format), (1, 0, 1));
        j.month_mut("2026-11").unwrap().entries.push(entry("Tea"));
        j.save_dirty().unwrap();
        let text = fs::read_to_string(d.join("2026-11.json")).unwrap();
        assert!(text.contains("\"format\": 3") && !text.contains("complete"), "{text}");
        // a format-2 build opens it read-only
        assert!(storage::newer_than(text.as_bytes(), 2).is_some());
        assert_eq!(Journal::open(d.clone()).month("2026-11").unwrap().entries.len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn complete_days_round_trip() {
        let d = tdir("complete");
        let mut j = Journal::open(d.clone());
        j.month_mut("2026-11").unwrap().complete.insert("2026-11-15".into());
        j.save_dirty().unwrap();
        let text = fs::read_to_string(d.join("2026-11.json")).unwrap();
        assert!(text.contains("\"complete\"") && text.contains("2026-11-15"));
        assert!(Journal::open(d.clone()).month("2026-11").unwrap().complete.contains("2026-11-15"));
        j.month_mut("2026-11").unwrap().complete.clear();
        j.save_dirty().unwrap();
        assert!(!fs::read_to_string(d.join("2026-11.json")).unwrap().contains("complete"), "the key is written only while a day is set");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn format_upgrade_is_copied_inside_the_snapshot_window() {
        let d = tdir("upgrade");
        fs::write(d.join("2026-11.json"), FORMAT_1).unwrap();
        let mut j = Journal::open(d.clone());
        j.last_snapshot.insert("2026-11".into(), now_secs()); // inside the 6 h window
        j.month_mut("2026-11").unwrap().entries.push(entry("Tea"));
        j.save_dirty().unwrap();
        let h = history_files(&d);
        assert_eq!(h.len(), 1);
        assert_eq!(fs::read(&h[0]).unwrap(), FORMAT_1.as_bytes(), "exactly the format-1 bytes");
        assert!(fs::read_to_string(d.join("2026-11.json")).unwrap().contains("\"format\": 3"));
        // the next save is the current format already, so the window applies again
        j.month_mut("2026-11").unwrap().entries.push(entry("More"));
        j.save_dirty().unwrap();
        assert_eq!(history_files(&d).len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn format_upgrade_refused_when_its_copy_fails() {
        let d = tdir("upgrade-fail");
        fs::write(d.join("2026-11.json"), FORMAT_1).unwrap();
        fs::write(d.join("history"), "in the way").unwrap(); // a file where the folder must go
        let mut j = Journal::open(d.clone());
        j.month_mut("2026-11").unwrap().entries.push(entry("Tea"));
        assert_eq!(j.save_dirty().unwrap_err(), NO_UPGRADE_COPY);
        assert_eq!(fs::read(d.join("2026-11.json")).unwrap(), FORMAT_1.as_bytes(), "the old file is as it was");
        j.discard_unsaved();
        assert_eq!(j.month("2026-11").unwrap().entries.len(), 1, "the day reads as it did on disk");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn profile_keeps_its_own_format() {
        let d = tdir("profile-format");
        let mut j = Journal::open(d.clone());
        j.profile_mut().unwrap();
        j.month_mut("2026-11").unwrap();
        j.save_dirty().unwrap();
        assert!(fs::read_to_string(d.join("profile.json")).unwrap().contains("\"format\": 2"));
        assert!(fs::read_to_string(d.join("2026-11.json")).unwrap().contains("\"format\": 3"));
        assert_eq!((PROFILE_FORMAT, JOURNAL_FORMAT), (2, 3));
        fs::write(d.join("profile.json"), r#"{"format": 3}"#).unwrap();
        assert_eq!(Journal::open(d.clone()).profile_mut().err(), Some(storage::READ_ONLY.to_string()));
        let _ = fs::remove_dir_all(&d);
    }

    const FORMAT_2: &str = r#"{"format": 2, "month": "2026-11", "entries": [{"uid": "u1", "day": "2026-11-13", "name": "Soup"}]}"#;

    #[test]
    fn format_2_month_gains_exercise_and_body_after_a_history_copy() {
        let d = tdir("fmt2");
        fs::write(d.join("2026-11.json"), FORMAT_2).unwrap();
        let mut j = Journal::open(d.clone());
        j.last_snapshot.insert("2026-11".into(), now_secs()); // inside the 6 h window: the copy is still made
        let m = j.month_mut("2026-11").unwrap();
        m.exercise.push(ExerciseEntry { uid: "x1".into(), day: "2026-11-13".into(), name: "Walk".into(), minutes: 30.0, est_kcal: Some(90.0), ..Default::default() });
        m.body.push(BodyEntry { uid: "b1".into(), day: "2026-11-13".into(), measure: "weight".into(), value: 80.0, ..Default::default() });
        j.save_dirty().unwrap();
        let h = history_files(&d);
        assert_eq!(h.len(), 1);
        assert_eq!(fs::read(&h[0]).unwrap(), FORMAT_2.as_bytes(), "exactly the format-2 bytes");
        let back = Journal::open(d.clone()).month("2026-11").unwrap().clone();
        assert_eq!((back.entries.len(), back.exercise.len(), back.body.len(), back.format), (1, 1, 1, 3));
        assert_eq!(back.exercise[0].est_kcal, Some(90.0));
        assert!(storage::newer_than(fs::read_to_string(d.join("2026-11.json")).unwrap().as_bytes(), 2).is_some(), "a format-2 build opens it read-only");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn month_without_new_records_has_no_new_keys() {
        let d = tdir("nonew");
        let mut j = Journal::open(d.clone());
        j.month_mut("2026-11").unwrap().entries.push(entry("Soup"));
        j.save_dirty().unwrap();
        let text = fs::read_to_string(d.join("2026-11.json")).unwrap();
        assert!(!text.contains("exercise") && !text.contains("body"), "{text}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn profile_read_never_dirties_it() {
        let d = tdir("profile-read");
        let mut j = Journal::open(d.clone());
        assert_eq!(j.profile().unwrap(), &Profile::default());
        j.save_dirty().unwrap();
        assert!(!d.join("profile.json").exists(), "reading the profile writes nothing");
        // a profile with nothing set writes only its format
        j.profile_mut().unwrap();
        j.save_dirty().unwrap();
        assert_eq!(fs::read_to_string(d.join("profile.json")).unwrap().split_whitespace().collect::<String>(), r#"{"format":2}"#);
        // a format-1 profile is read without a rewrite, and its first rewrite copies the old bytes
        let old = r#"{"format": 1}"#;
        fs::write(d.join("profile.json"), old).unwrap();
        let mut k = Journal::open(d.clone());
        k.profile().unwrap();
        k.save_dirty().unwrap();
        assert_eq!(fs::read(d.join("profile.json")).unwrap(), old.as_bytes());
        assert!(history_files(&d).is_empty());
        k.profile_mut().unwrap().height_cm = Some(180.0);
        k.save_dirty().unwrap();
        let h = history_files(&d);
        assert_eq!((h.len(), fs::read(&h[0]).unwrap()), (1, old.as_bytes().to_vec()));
        let text = fs::read_to_string(d.join("profile.json")).unwrap();
        assert!(text.contains("\"format\": 2") && text.contains("180"), "{text}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn bundle_carries_exercise_body_and_targets() {
        let (a, b) = (tdir("bundle-src"), tdir("bundle-dst"));
        let mut j = Journal::open(a.clone());
        let m = j.month_mut("2026-11").unwrap();
        m.entries.push(entry("Soup"));
        m.exercise.push(ExerciseEntry { uid: "x1".into(), day: "2026-11-13".into(), name: "Walk".into(), minutes: 30.0, est_kcal: Some(90.0), ..Default::default() });
        m.body.push(BodyEntry { uid: "b1".into(), day: "2026-11-13".into(), measure: "weight".into(), value: 80.25, ..Default::default() });
        let target = goals::TargetSet { effective_from: "2026-11-13".into(), kcal: 2100, templates: vec![goals::Template { id: 1, name: "Training".into(), kcal_offset: 300, ..Default::default() }], week: [None, Some(1), None, None, None, None, None], ..Default::default() };
        let p = j.profile_mut().unwrap();
        p.height_cm = Some(180.34);
        p.targets.push(target);
        j.save_dirty().unwrap();

        let (bundle, left_out) = bundle_from_disk(&a);
        let bundle = bundle.unwrap();
        assert!(left_out.is_empty());
        let text = serde_json::to_string(&bundle).unwrap();
        assert!(text.contains("Walk") && text.contains("80.25") && text.contains("Training") && text.contains("180.34"), "{text}");
        restore_bundle(&b, &bundle).unwrap();
        let (mut from, mut to) = (Journal::open(a.clone()), Journal::open(b.clone()));
        assert_eq!(to.month("2026-11").unwrap(), from.month("2026-11").unwrap());
        assert_eq!(to.profile().unwrap(), from.profile().unwrap());
        assert_eq!((to.month("2026-11").unwrap().exercise.len(), to.month("2026-11").unwrap().body.len(), to.profile().unwrap().targets.len()), (1, 1, 1));
        for x in [&a, &b] {
            let _ = fs::remove_dir_all(x);
        }
    }

    #[test]
    fn old_backup_formats_restore_and_upgrade() {
        let d = tdir("oldbackup");
        let mut bundle = JournalBundle::default();
        bundle.months.insert("2026-11".into(), serde_json::from_str(FORMAT_2).unwrap());
        bundle.profile = Some(serde_json::from_str(r#"{"format": 1}"#).unwrap());
        assert_eq!((bundle.months["2026-11"].format, bundle.profile.as_ref().unwrap().format), (2, 1), "the old formats are kept as they were read");
        check_bundle(&bundle).unwrap();
        restore_bundle(&d, &bundle).unwrap();
        let (old_month, old_profile) = (fs::read(d.join("2026-11.json")).unwrap(), fs::read(d.join("profile.json")).unwrap());
        assert!(String::from_utf8_lossy(&old_month).contains("\"format\": 2") && String::from_utf8_lossy(&old_profile).contains("\"format\": 1"));

        // the first change after the restore copies both old files and writes the new formats
        let mut j = Journal::open(d.clone());
        j.last_snapshot.insert("2026-11".into(), now_secs()); // inside the 6 h window: the copy is still made
        j.last_snapshot.insert("profile".into(), now_secs());
        j.month_mut("2026-11").unwrap().body.push(BodyEntry { uid: "b1".into(), day: "2026-11-13".into(), measure: "weight".into(), value: 80.0, ..Default::default() });
        j.profile_mut().unwrap().height_cm = Some(180.0);
        j.save_dirty().unwrap();
        let kept: Vec<Vec<u8>> = history_files(&d).iter().map(|p| fs::read(p).unwrap()).collect();
        assert_eq!(kept.len(), 2);
        assert!(kept.contains(&old_month) && kept.contains(&old_profile), "both old files copied byte for byte");
        assert!(fs::read_to_string(d.join("2026-11.json")).unwrap().contains("\"format\": 3"));
        assert!(fs::read_to_string(d.join("profile.json")).unwrap().contains("\"format\": 2"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn months_with_data_counts_unsaved_months() {
        let d = tdir("withdata");
        let mut j = Journal::open(d.clone());
        j.month_mut("2026-10").unwrap().entries.push(entry("A"));
        j.save_dirty().unwrap();
        j.month_mut("2026-12").unwrap().entries.push(entry("B"));
        assert_eq!(j.months_with_data(), ["2026-10", "2026-12"]);
        let _ = fs::remove_dir_all(&d);
    }

    fn copies(d: &Path, stem: &str) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(d).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(&format!("{stem}-damaged-"))).collect();
        v.sort();
        v
    }

    fn good_month(m: &str, name: &str) -> String {
        serde_json::to_string_pretty(&JournalMonth { month: m.into(), entries: vec![entry(name)], ..Default::default() }).unwrap()
    }

    const DAMAGED: &[u8] = b"{\"entries\": [";

    #[test]
    fn month_file_round_trip_and_bak() {
        let d = tdir("roundtrip");
        let mut j = Journal::open(d.clone());
        let e = DiaryEntry { day: "2026-01-31".into(), name: "Oat porridge".into(), basis: Nutrients { calories: Some(150.0), ..Default::default() }, ..Default::default() };
        j.month_mut("2026-01").unwrap().entries.push(e.clone());
        j.save_dirty().unwrap();
        assert!(d.join("2026-01.json").exists() && !d.join("2026-02.json").exists());
        let mut again = Journal::open(d.clone());
        let back = again.month("2026-01").unwrap();
        assert_eq!(back.entries, vec![e.clone()]);
        assert_eq!(back.entries[0].basis.sugar_g, None);
        assert!(!fs::read_to_string(d.join("2026-01.json")).unwrap().contains("null"));
        assert!(!d.join("2026-01.json.bak").exists(), "the first write has nothing to keep");
        again.month_mut("2026-01").unwrap().entries.push(DiaryEntry { name: "Tea".into(), ..e });
        again.save_dirty().unwrap();
        assert!(d.join("2026-01.json.bak").exists());
        assert!(Journal::open(d.clone()).month("../x").is_err());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_month_is_quarantined_not_rotated() {
        let d = tdir("damaged");
        fs::write(d.join("2026-11.json"), DAMAGED).unwrap();
        let mut j = Journal::open(d.clone());
        assert_eq!(j.month_mut("2026-11").err(), Some(storage::DAMAGED.to_string()));
        assert!(matches!(j.problems(), [(m, Problem::Damaged { kept: Some(_), .. })] if m == "2026-11"));
        let c = copies(&d, "2026-11");
        assert_eq!(c.len(), 1);
        assert_eq!(fs::read(&c[0]).unwrap(), DAMAGED);
        // the other months still save, and the damaged one is left exactly as it is
        j.month_mut("2026-10").unwrap().entries.push(entry("Toast"));
        j.save_dirty().unwrap();
        assert!(d.join("2026-10.json").exists());
        assert_eq!(fs::read(d.join("2026-11.json")).unwrap(), DAMAGED);
        assert!(!d.join("2026-11.json.bak").exists());
        // starting again on the same damage keeps one copy
        assert!(Journal::open(d.clone()).month("2026-11").is_err());
        assert_eq!(copies(&d, "2026-11").len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_month_with_good_bak_recovers() {
        let d = tdir("recover");
        fs::write(d.join("2026-11.json"), DAMAGED).unwrap();
        fs::write(d.join("2026-11.json.bak"), good_month("2026-11", "Soup")).unwrap();
        let mut j = Journal::open(d.clone());
        assert_eq!(j.month_mut("2026-11").unwrap().entries[0].name, "Soup");
        assert!(matches!(&j.recovered[..], [(m, Some(_))] if m == "2026-11"));
        assert_eq!(fs::read(&copies(&d, "2026-11")[0]).unwrap(), DAMAGED);
        j.month_mut("2026-11").unwrap().entries.push(entry("Tea"));
        j.save_dirty().unwrap();
        assert_ne!(fs::read(d.join("2026-11.json.bak")).unwrap(), DAMAGED, "the damaged bytes never reach .bak");
        assert_eq!(Journal::open(d.clone()).month("2026-11").unwrap().entries.len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn damage_found_at_save_is_copied_first() {
        let d = tdir("atsave");
        let mut j = Journal::open(d.clone());
        j.month_mut("2026-11").unwrap().entries.push(entry("Soup"));
        j.save_dirty().unwrap();
        j.month_mut("2026-11").unwrap().entries.push(entry("Tea"));
        fs::write(d.join("2026-11.json"), b"garbage").unwrap(); // damaged behind the app's back
        j.save_dirty().unwrap();
        let c = copies(&d, "2026-11");
        assert_eq!((c.len(), fs::read(&c[0]).unwrap()), (1, b"garbage".to_vec()));
        assert!(!d.join("2026-11.json.bak").exists(), "garbage is never rotated into .bak");
        assert_eq!(Journal::open(d.clone()).month("2026-11").unwrap().entries.len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_month_is_not_rewritten() {
        let d = tdir("newer");
        let newer = format!(r#"{{"format": {}, "app_version": "9.0.0", "month": "2026-11", "entries": 5}}"#, JOURNAL_FORMAT + 1);
        fs::write(d.join("2026-11.json"), &newer).unwrap();
        let mut j = Journal::open(d.clone());
        assert_eq!(j.month_mut("2026-11").err(), Some(storage::READ_ONLY.to_string()));
        assert!(matches!(j.problems(), [(m, Problem::TooNew { found, .. })] if m == "2026-11" && found == "9.0.0"));
        j.month_mut("2026-10").unwrap().entries.push(entry("Toast"));
        j.save_dirty().unwrap();
        assert_eq!(fs::read(d.join("2026-11.json")).unwrap(), newer.as_bytes());
        assert!(!d.join("2026-11.json.bak").exists() && copies(&d, "2026-11").is_empty());
        // a file that turns newer after it was loaded is refused at the save and locked
        let mut k = Journal::open(d.clone());
        k.month_mut("2026-09").unwrap().entries.push(entry("Rice"));
        fs::write(d.join("2026-09.json"), &newer).unwrap();
        assert_eq!(k.save_dirty(), Err(storage::READ_ONLY.to_string()));
        assert_eq!(fs::read(d.join("2026-09.json")).unwrap(), newer.as_bytes());
        assert!(k.month_mut("2026-09").is_err());
        assert_eq!(k.save_dirty(), Ok(()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn journal_months_ignore_bak_tmp_and_damaged_names() {
        let d = tdir("names");
        for f in ["2026-11.json", "2026-11.json.bak", "2026-11.tmp", "2026-11-damaged-x.json", "profile.json", "2026-13.json", "notes.json"] {
            fs::write(d.join(f), "{}").unwrap();
        }
        fs::create_dir_all(d.join("history")).unwrap();
        fs::write(d.join("history").join("2026-10-5.json"), "{}").unwrap();
        assert_eq!(Journal::open(d.clone()).months_on_disk(), vec!["2026-11".to_string()]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn snapshots_once_per_window_and_pruned() {
        let d = tdir("snaps");
        let hist = d.join("history");
        let count = |p: &str| fs::read_dir(&hist).map(|rd| rd.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(p)).count()).unwrap_or(0);
        let mut j = Journal::open(d.clone());
        for n in 0..4 {
            j.month_mut("2026-11").unwrap().entries.push(entry(&format!("Food {n}")));
            j.save_dirty().unwrap();
            // write 1 creates the file; write 2 is the first replacement and snapshots; 3 and 4 are inside the window
            assert_eq!(count("2026-11-"), if n == 0 { 0 } else { 1 }, "after write {}", n + 1);
        }
        // a second window: put the last snapshot back and the next write snapshots again
        for e in fs::read_dir(&hist).unwrap().flatten() {
            fs::remove_file(e.path()).unwrap();
        }
        j.last_snapshot.insert("2026-11".into(), now_secs() - JOURNAL_SNAPSHOT_GAP - 1);
        j.month_mut("2026-11").unwrap().entries.push(entry("More"));
        j.save_dirty().unwrap();
        assert_eq!(count("2026-11-"), 1);
        // pruning: 30 old files plus the new one leave 28, the oldest gone
        for i in 0..30 {
            fs::write(hist.join(format!("2026-11-{}.json", 100 + i)), "{}").unwrap();
        }
        j.last_snapshot.insert("2026-11".into(), 0);
        j.month_mut("2026-11").unwrap().entries.push(entry("Again"));
        j.save_dirty().unwrap();
        assert_eq!(count("2026-11-"), JOURNAL_SNAPSHOTS_KEPT);
        assert!(!hist.join("2026-11-100.json").exists() && hist.join("2026-11-129.json").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn profile_follows_the_same_rules() {
        let d = tdir("profile-damaged");
        fs::write(d.join("profile.json"), DAMAGED).unwrap();
        let mut j = Journal::open(d.clone());
        assert!(j.profile_mut().is_err());
        assert!(matches!(j.problems(), [(k, Problem::Damaged { kept: Some(_), .. })] if k == "profile"));
        assert_eq!(fs::read(&copies(&d, "profile")[0]).unwrap(), DAMAGED);
        j.save_dirty().unwrap();
        assert_eq!(fs::read(d.join("profile.json")).unwrap(), DAMAGED);
        assert!(!d.join("profile.json.bak").exists());
        let n = tdir("profile-newer");
        let newer = format!(r#"{{"format": {}}}"#, JOURNAL_FORMAT + 1);
        fs::write(n.join("profile.json"), &newer).unwrap();
        let mut k = Journal::open(n.clone());
        assert_eq!(k.profile_mut().err(), Some(storage::READ_ONLY.to_string()));
        assert_eq!(fs::read(n.join("profile.json")).unwrap(), newer.as_bytes());
        // a good profile saves and reads back
        let g = tdir("profile-good");
        let mut p = Journal::open(g.clone());
        p.profile_mut().unwrap();
        p.save_dirty().unwrap();
        assert_eq!(Journal::open(g.clone()).profile_mut().unwrap().format, PROFILE_FORMAT);
        for x in [d, n, g] {
            let _ = fs::remove_dir_all(&x);
        }
    }
}
