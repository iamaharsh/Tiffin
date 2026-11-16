//! The journal: diary month files (`journal/YYYY-MM.json`) and the profile (`journal/profile.json`)
//! kept beside the library. Tracker data never goes into library.json, so a diary change never
//! rewrites the recipes and a recipe change never rewrites the diary.

use crate::model::{now_secs, MealType, Nutrients};
use crate::storage::{self, Problem};
use crate::tracker::day::is_month_key;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The number that decides whether a month file or profile.json is too new for this build. Bump it
/// only for a change an older build would lose when it re-saves one of these files (for example a
/// new kind of record in month files), exactly as `LIBRARY_FORMAT` does for the library.
pub const JOURNAL_FORMAT: u32 = 1;

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
    /// The nutrition the entry was logged with: per 100 g when `per_100g`, otherwise for the whole amount.
    pub basis: Nutrients,
    pub per_100g: bool,
}

/// One month's diary. Month files will gain exercise and body records the same way, with a `JOURNAL_FORMAT` bump.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct JournalMonth {
    pub format: u32,
    /// `YYYY-MM`
    pub month: String,
    pub entries: Vec<DiaryEntry>,
}

impl Default for JournalMonth {
    fn default() -> Self {
        JournalMonth { format: JOURNAL_FORMAT, month: String::new(), entries: vec![] }
    }
}

/// The user's body details and targets. Only the format for now; the goal engine fills it in.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub format: u32,
}

impl Default for Profile {
    fn default() -> Self {
        Profile { format: JOURNAL_FORMAT }
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
}

/// What reading one journal file found.
enum FileRead<T> {
    Missing,
    Ok(T),
    Bad,
    /// `format` above [`JOURNAL_FORMAT`]; holds the version that wrote it.
    TooNew(String),
}

fn read_file<T: DeserializeOwned>(path: &Path) -> FileRead<T> {
    match fs::read(path) {
        Ok(bytes) => match storage::newer_than(&bytes, JOURNAL_FORMAT) {
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
        Journal { dir, months: BTreeMap::new(), dirty: BTreeSet::new(), profile: None, profile_dirty: false, problems: vec![], recovered: vec![], last_snapshot: BTreeMap::new() }
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
        match read_file::<T>(&main) {
            FileRead::Ok(v) => Ok(v),
            FileRead::TooNew(found) => Err(self.lock(key, too_new(found))), // never read or touch the .bak
            FileRead::Missing => match read_file::<T>(&bak) {
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
                match read_file::<T>(&bak) {
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
        Ok(self.months.get_mut(m).unwrap())
    }

    /// The profile for changing; marked changed. Err while profile.json is locked.
    pub fn profile_mut(&mut self) -> Result<&mut Profile, String> {
        if let Some(p) = self.locked("profile") {
            return Err(p.refusal().into());
        }
        if self.profile.is_none() {
            self.profile = Some(self.load("profile", Profile::default())?);
        }
        self.profile_dirty = true;
        Ok(self.profile.as_mut().unwrap())
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
        let keep_bak = match read_file::<T>(&path) {
            FileRead::Missing => false,
            FileRead::Ok(_) => {
                self.snapshot_if_due(key, &path);
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
        let hist = self.dir.join("history");
        if let Ok(bytes) = fs::read(path) {
            let _ = storage::write_atomic(&hist.join(format!("{key}-{now}.json")), &bytes, false);
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
        match read_file::<JournalMonth>(&dir.join(format!("{key}.json"))) {
            FileRead::Ok(m) => {
                b.months.insert(key, m);
            }
            FileRead::Missing => {}
            _ => left_out.push(key),
        }
    }
    match read_file::<Profile>(&dir.join("profile.json")) {
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
    if b.months.values().any(|m| m.format > JOURNAL_FORMAT) || b.profile.as_ref().is_some_and(|p| p.format > JOURNAL_FORMAT) {
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
        assert_eq!(Journal::open(g.clone()).profile_mut().unwrap().format, JOURNAL_FORMAT);
        for x in [d, n, g] {
            let _ = fs::remove_dir_all(&x);
        }
    }
}
