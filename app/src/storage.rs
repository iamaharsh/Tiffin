//! Saving, history snapshots (rollback), one-file backup/restore and folder sync.
//!
//! The library lives in the user's own data folder (not the install folder), so updates and
//! reinstalls never touch it. Every save is atomic (write temp file, then rename) and a snapshot
//! is taken before risky operations and at most every SNAPSHOT_MIN_GAP seconds.

use crate::model::{now_secs, Library, Recipe, APP_VERSION, LIBRARY_FORMAT};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const SNAPSHOT_MIN_GAP: u64 = 600;
/// Periodic sync interval: every 5 minutes.
pub const AUTO_SYNC_SECS: f64 = 300.0;
/// Retry back-off base (5 s) and cap (10 minutes).
pub const SYNC_RETRY_BASE_SECS: f64 = 5.0;
pub const SYNC_RETRY_MAX_SECS: f64 = 600.0;
/// Delay after a reconnect (1 s).
pub const SYNC_RECONNECT_SECS: f64 = 1.0;

/// Seconds before retry number `failures`: base × (n+1)², capped.
pub fn sync_retry_delay(failures: u32) -> f64 {
    (SYNC_RETRY_BASE_SECS * ((failures as f64) + 1.0).powi(2)).min(SYNC_RETRY_MAX_SECS)
}
/// "Remind me later" for updates: 7 days.
pub const UPDATE_REMIND_DAYS: u64 = 7;
pub const BACKUP_EXT: &str = "recipebox";
/// Shown when a change is refused because the library file couldn't be read.
pub const DAMAGED: &str = "Your library file couldn't be read, so nothing is saved until you choose how to recover it.";
/// Shown when a damaged library file could not be copied aside, so it is left exactly as it is.
pub const NO_SAFETY_COPY: &str = "Tiffin couldn't make a safety copy of your damaged library file, so it won't replace it. Make sure the data folder isn't full or locked by another program, then try again.";
/// Shown when a change is refused because the library was saved by a newer version of the app.
pub const READ_ONLY: &str = "This library was saved by a newer version of Tiffin, so it's open read-only. Update the app to make changes.";
/// Shown when a backup or saved version from a newer version of the app is restored; nothing is changed.
pub const NEWER_BACKUP: &str = "This was saved by a newer version of Tiffin. Update the app to restore it; nothing was changed.";
pub const SYNC_FILE: &str = "recipe-box-sync.json";

/// The data folder keeps this name through the Tiffin rename, so existing users keep their library (SAFE-05).
pub const DATA_FOLDER: &str = "RecipeBox";

pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("XDG_DATA_HOME"))
        .or_else(|_| std::env::var("HOME").map(|h| format!("{}/.local/share", h)))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(base).join(DATA_FOLDER)
}

pub fn documents_dir() -> PathBuf {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Documents")
}

/// Documents folders that moved once for the Tiffin rename: (label, old name, new name).
/// `DATA_FOLDER` is not among them; it keeps its name (SAFE-05).
pub const MOVED_FOLDERS: [(&str, &str, &str); 3] = [
    ("Backups", "Recipe Box Backups", "Tiffin Backups"),
    ("Tracker Exports", "Recipe Box Tracker Exports", "Tiffin Tracker Exports"),
    ("Inbox", "Recipe Box Inbox", "Tiffin Inbox"),
];

/// The folder to use now: the old one while it still holds anything, else the new one.
/// It keeps no state, so a move that stopped half way resumes (or keeps working) after a crash.
pub fn pick_folder(docs: &Path, old: &str, new: &str) -> PathBuf {
    let o = docs.join(old);
    if fs::read_dir(&o).map(|mut r| r.next().is_some()).unwrap_or(false) {
        o
    } else {
        docs.join(new)
    }
}

/// Where backups go for the library in `data`. A test copy (`--data-dir`) keeps its own inside its data folder, so it
/// never writes, restores from or strips the diary out of the owner's backups in Documents.
pub fn backups_dir(data: &Path) -> PathBuf {
    if data != data_dir() {
        return data.join("Backups");
    }
    pick_folder(&documents_dir(), MOVED_FOLDERS[0].1, MOVED_FOLDERS[0].2)
}

/// Shown when a Documents folder could not be moved to its Tiffin name; nothing was deleted.
pub fn folder_not_moved(label: &str) -> String {
    format!("Tiffin couldn't move your {label} folder to its new name yet. Nothing was deleted, and it will try again next time.")
}

/// Moves one of the [`MOVED_FOLDERS`] to its new name. The error is the sentence for the user.
pub fn move_old_folder(label: &str) -> Result<(), String> {
    let Some((_, old, new)) = MOVED_FOLDERS.iter().find(|f| f.0 == label) else { return Ok(()) };
    let docs = documents_dir();
    move_folder(&docs.join(old), &docs.join(new)).map_err(|_| folder_not_moved(label))
}

/// Every file and sub-folder under `root/rel`, as paths relative to `root`.
fn walk(root: &Path, rel: &Path, files: &mut Vec<PathBuf>, dirs: &mut Vec<PathBuf>) -> Result<(), String> {
    let here = root.join(rel);
    for e in fs::read_dir(&here).map_err(|e| format!("Can't read {}: {}", here.display(), e))?.flatten() {
        let r = rel.join(e.file_name());
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            dirs.push(r.clone());
            walk(root, &r, files, dirs)?;
        } else {
            files.push(r);
        }
    }
    Ok(())
}

/// Moves every file of `from` into `to`, sub-folders included. Each copy is written atomically and read
/// back; `from` is deleted only after every file is verified. A file already in `to` is never overwritten:
/// an identical one counts as moved, a different one is a conflict. On any failure nothing is deleted.
pub fn move_folder(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Ok(());
    }
    if to.exists() && !to.is_dir() {
        return Err(format!("{} is not a folder.", to.display()));
    }
    let (mut files, mut dirs) = (vec![], vec![]);
    walk(from, Path::new(""), &mut files, &mut dirs)?;
    let mut problem: Option<String> = None;
    for rel in &files {
        let (src, dst) = (from.join(rel), to.join(rel));
        let bytes = fs::read(&src).map_err(|e| format!("Can't read {}: {}", src.display(), e))?;
        if dst.exists() {
            if !matches!(fs::read(&dst), Ok(d) if d == bytes) {
                problem.get_or_insert(format!("{} already exists with different contents.", dst.display()));
            }
            continue;
        }
        // write_atomic stages through this name and would replace whatever is there
        if dst.with_extension("tmp").exists() {
            problem.get_or_insert(format!("{} is in the way.", dst.with_extension("tmp").display()));
            continue;
        }
        if let Err(e) = write_atomic(&dst, &bytes, false) {
            problem.get_or_insert(format!("Can't write {}: {}", dst.display(), e));
        } else if !matches!(fs::read(&dst), Ok(d) if d == bytes) {
            let _ = fs::remove_file(&dst); // only the copy this call just made
            problem.get_or_insert(format!("The copy of {} didn't match.", src.display()));
        }
    }
    if let Some(p) = problem {
        return Err(p);
    }
    for rel in &files {
        let src = from.join(rel);
        fs::remove_file(&src).map_err(|e| format!("Can't remove {}: {}", src.display(), e))?;
    }
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    for d in dirs.iter().map(|d| from.join(d)).chain(std::iter::once(from.to_path_buf())) {
        fs::remove_dir(&d).map_err(|e| format!("Can't remove {}: {}", d.display(), e))?;
    }
    Ok(())
}

/// library.json inside a data folder.
pub fn library_path(dir: &Path) -> PathBuf {
    dir.join("library.json")
}
fn history_dir(dir: &Path) -> PathBuf {
    dir.join("history")
}

/// Why the library can't be saved right now. While one is set every change is refused, so the
/// shell only needs to call [`Store::check_unlocked`] and show [`Problem::banner`].
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    /// `library.json` (or, when it is missing, its backup) couldn't be read. `kept` is the verified
    /// copy of the unreadable file, or None while no copy could be made.
    Damaged { file: PathBuf, kept: Option<PathBuf> },
    /// The library's `format` is above this build's. It is open read-only and never re-saved. `found` is the
    /// version that wrote it; `partial` is true when some items couldn't be read and are hidden.
    TooNew { found: String, partial: bool },
}

impl Problem {
    /// The sentence shown when a change is refused.
    pub fn refusal(&self) -> &'static str {
        match self {
            Problem::Damaged { .. } => DAMAGED,
            Problem::TooNew { .. } => READ_ONLY,
        }
    }

    /// The permanent message for the sidebar.
    pub fn banner(&self) -> String {
        match self {
            Problem::Damaged { kept: Some(k), .. } => format!("Tiffin couldn't read your library file, so nothing is being saved. A copy was kept at {}.", k.display()),
            Problem::Damaged { file, kept: None } => format!(
                "Tiffin couldn't read your library file at {} or make a safety copy of it, so the file is left exactly as it is and nothing is being saved.",
                file.display()
            ),
            Problem::TooNew { found, partial } => format!(
                "This library was saved by a newer version of Tiffin ({found}). You can look but not change anything here. Update the app to edit.{}",
                if *partial { " Some items can't be shown in this version." } else { "" }
            ),
        }
    }
}

/// Something the user is told once (or, for `Damaged`, for as long as the library stays damaged).
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// The library was opened from its backup copy. `kept` is the copy of the damaged file, or None when the file was missing.
    Recovered { kept: Option<PathBuf> },
    /// A save found library.json unreadable while the app was open: it was kept at `kept` and the open library saved in its place.
    Replaced { kept: PathBuf },
    /// The library is locked until the user picks a way to recover.
    Damaged { file: PathBuf, kept: Option<PathBuf> },
}

/// What `Store::load` found.
pub enum LoadOutcome {
    /// Neither library.json nor library.json.bak exists: a true first run.
    Missing,
    /// A readable library, from library.json or, repaired, from its backup.
    Loaded { lib: Library },
    /// library.json couldn't be read and there is no readable backup. The store is locked (`Store::problem`).
    Damaged,
    /// The library was saved by a newer version. `lib` holds what this build can read; the store is read-only (`Store::problem`).
    TooNew { lib: Library },
}

enum Read {
    Missing,
    Ok(Library),
    Bad,
    /// `format` above this build's. `lib` is what this build can read; `partial` when the rest is hidden.
    TooNew { lib: Library, found: String, partial: bool },
}

/// The two fields read before anything else, so a newer file is recognised even when the rest doesn't parse.
#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    format: u32,
    #[serde(default)]
    app_version: String,
}

fn too_new(header: &[u8]) -> Option<String> {
    newer_than(header, LIBRARY_FORMAT)
}

/// The version that wrote `bytes` when its `format` is above `current`. It reads the header only, so it works even when the body doesn't parse. The journal files use it too.
pub(crate) fn newer_than(bytes: &[u8], current: u32) -> Option<String> {
    let h = serde_json::from_slice::<Header>(bytes).ok().filter(|h| h.format > current)?;
    Some(if h.app_version.is_empty() { "a newer version".into() } else { h.app_version })
}

/// What this build can read of a newer library: every recipe that parses and every category name.
fn salvage(bytes: &[u8]) -> Library {
    let mut lib = Library::default();
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) {
        if let Some(rs) = v.get("recipes").and_then(|r| r.as_array()) {
            lib.recipes = rs.iter().filter_map(|r| serde_json::from_value::<Recipe>(r.clone()).ok()).collect();
        }
        if let Some(cs) = v.get("categories").and_then(|c| c.as_array()) {
            lib.categories = cs.iter().filter_map(|c| c.as_str().map(String::from)).collect();
        }
        lib.next_id = lib.recipes.iter().map(|r| r.id + 1).max().unwrap_or(1);
    }
    lib
}

/// Length and modified time of a file: enough to tell "the file I wrote" from "changed behind my back".
type Stamp = (u64, std::time::SystemTime);

enum Disk {
    Absent,
    Stamp(Stamp),
    /// Exists but can't be inspected (locked, no permission): never treated as absent.
    Unknown,
}

fn disk(path: &Path) -> Disk {
    match fs::metadata(path) {
        Ok(m) => match m.modified() {
            Ok(t) => Disk::Stamp((m.len(), t)),
            Err(_) => Disk::Unknown,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Disk::Absent,
        Err(_) => Disk::Unknown,
    }
}

fn stamp_of(path: &Path) -> Option<Stamp> {
    match disk(path) {
        Disk::Stamp(s) => Some(s),
        _ => None,
    }
}

fn read_library(path: &Path) -> Read {
    match fs::read(path) {
        Ok(bytes) => match (serde_json::from_slice::<Library>(&bytes), too_new(&bytes)) {
            (Ok(lib), None) => Read::Ok(lib),
            (Ok(lib), Some(found)) => Read::TooNew { lib, found, partial: false },
            (Err(_), Some(found)) => Read::TooNew { lib: salvage(&bytes), found, partial: true },
            (Err(_), None) => Read::Bad,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(_) => Read::Bad,
    }
}

/// Keep a verified copy of an unreadable file beside it as `<stem>-damaged-<date>.json`, where the
/// stem is the file name up to its first dot (`library`, `2026-11`, `profile`; `-bak` before
/// `.json` for a backup). The original is never touched. An existing copy with the same bytes is
/// reused, so starting again on a file that stays damaged keeps one copy.
pub(crate) fn quarantine(path: &Path) -> Result<PathBuf, String> {
    let bytes = fs::read(path).map_err(|_| NO_SAFETY_COPY.to_string())?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path.file_name().map(|n| n.to_string_lossy().split('.').next().unwrap_or("").to_string()).unwrap_or_default();
    let prefix = format!("{stem}-damaged-");
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix) && name.ends_with(".json") && e.metadata().map(|m| m.len()).ok() == Some(bytes.len() as u64) && fs::read(e.path()).ok().as_deref() == Some(&bytes[..]) {
                return Ok(e.path());
            }
        }
    }
    let t = now_secs();
    let base = format!("{prefix}{}-{:02}{}", fmt_stamp(t).replace([':', ' '], "-"), t % 60, if path.extension().is_some_and(|x| x == "bak") { "-bak" } else { "" });
    let mut copy = dir.join(format!("{}.json", base));
    let mut n = 2;
    while copy.exists() {
        copy = dir.join(format!("{}-{}.json", base, n));
        n += 1;
    }
    let made = write_atomic(&copy, &bytes, false).and_then(|_| fs::read(&copy).map_err(|e| e.to_string())).is_ok_and(|back| back == bytes);
    if !made {
        let _ = fs::remove_file(&copy);
        return Err(NO_SAFETY_COPY.into());
    }
    Ok(copy)
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8], keep_bak: bool) -> Result<(), String> {
    use std::io::Write;
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    // sync_all before the rename, so a power cut can't leave a truncated file under the real name
    let written = fs::File::create(&tmp).and_then(|mut f| f.write_all(bytes).and_then(|_| f.sync_all()));
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    if keep_bak && path.exists() {
        let _ = fs::copy(path, path.with_extension("json.bak"));
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

pub struct Store {
    pub dir: PathBuf,
    pub read_only: bool,
    /// Set while the library can't be saved; see [`Store::check_unlocked`].
    pub problem: Option<Problem>,
    last_snapshot: u64,
    pending: Option<Notice>,
    /// library.json as this process last wrote it, or last read it as a good library.
    written: Option<Stamp>,
    /// library.json as it was when its damaged bytes were copied aside.
    copied: Option<Stamp>,
}

impl Store {
    pub fn new(dir: PathBuf, read_only: bool) -> Store {
        Store { dir, read_only, problem: None, last_snapshot: 0, pending: None, written: None, copied: None }
    }

    /// True only when a real snapshot can be written: the store is writable and the library loaded cleanly
    /// (`snapshot` returns Ok without writing when read-only). The updater installs nothing when this is false.
    pub fn can_snapshot(&self) -> bool {
        !self.read_only && self.problem.is_none()
    }

    /// Err with the problem's message while the library can't be changed. Every writer in a shell calls this.
    pub fn check_unlocked(&self) -> Result<(), String> {
        match &self.problem {
            Some(p) => Err(p.refusal().into()),
            None => Ok(()),
        }
    }

    /// The notice to show, if any: recovered or replaced once, damaged for as long as the library stays damaged.
    pub fn take_notice(&mut self) -> Option<Notice> {
        if let Some(n) = self.pending.take() {
            return Some(n);
        }
        match &self.problem {
            Some(Problem::Damaged { file, kept }) => Some(Notice::Damaged { file: file.clone(), kept: kept.clone() }),
            // the banner (Problem::banner) says it; no dialog
            Some(Problem::TooNew { .. }) | None => None,
        }
    }

    /// Ok when the library may be swapped for another one: no problem, or a damaged file whose bytes
    /// are safely copied (retrying any copy that failed). Err(NO_SAFETY_COPY) while a copy still fails.
    pub fn can_replace(&mut self) -> Result<(), String> {
        if matches!(self.problem, Some(Problem::TooNew { .. })) {
            return Err(READ_ONLY.into()); // a restore or start-empty would re-save over the newer file
        }
        let Some(Problem::Damaged { file, kept }) = self.problem.clone() else { return Ok(()) };
        let (main, bak) = (library_path(&self.dir), library_path(&self.dir).with_extension("json.bak"));
        let (mut main_copy, mut bak_copy, mut failed) = (None, None, false);
        for (p, is_main) in [(&main, true), (&bak, false)] {
            if matches!(read_library(p), Read::Bad) {
                match quarantine(p) {
                    Ok(c) if is_main => {
                        self.copied = stamp_of(&main);
                        main_copy = Some(c);
                    }
                    Ok(c) => bak_copy = Some(c),
                    Err(_) => failed = true,
                }
            }
        }
        if failed {
            return Err(NO_SAFETY_COPY.into());
        }
        self.problem = Some(Problem::Damaged { file, kept: main_copy.or(bak_copy).or(kept) });
        Ok(())
    }

    /// The user chose how to recover: unlock. Nothing else changes (the kept copy stays).
    pub fn clear_damage(&mut self) {
        if matches!(self.problem, Some(Problem::Damaged { .. })) {
            self.problem = None;
            self.read_only = false;
        }
    }

    /// Protect the library across app updates: keep a snapshot of the old one.
    /// `persist` saves the bumped library on the main load path; the `.bak` recovery path saves it itself.
    fn after_load(&mut self, mut lib: Library, persist: bool) -> Library {
        if (lib.app_version != APP_VERSION || lib.format < LIBRARY_FORMAT) && !self.read_only {
            // no upgrade without its snapshot (the old file couldn't be brought back); the next start tries again
            if self.snapshot(&lib, &format!("before update from {}", lib.app_version)).is_ok() {
                lib.app_version = APP_VERSION.into();
                lib.format = LIBRARY_FORMAT;
                // written now so the next start sees the new version and takes no second snapshot; save keeps the old file as .bak
                if persist {
                    let _ = self.save(&lib);
                }
            }
        }
        lib
    }

    fn damaged(&mut self, file: PathBuf, kept: Option<PathBuf>) -> LoadOutcome {
        self.problem = Some(Problem::Damaged { file, kept });
        self.read_only = true;
        LoadOutcome::Damaged
    }

    fn too_new(&mut self, lib: Library, found: String, partial: bool) -> LoadOutcome {
        self.problem = Some(Problem::TooNew { found, partial });
        self.read_only = true;
        LoadOutcome::TooNew { lib }
    }

    /// Load the library. A damaged library.json is copied aside first (never replaced before its copy
    /// exists) and the backup is used when it is readable.
    pub fn load(&mut self) -> LoadOutcome {
        let main = library_path(&self.dir);
        let bak = main.with_extension("json.bak");
        let main_missing = match read_library(&main) {
            Read::Ok(lib) => {
                self.written = stamp_of(&main);
                return LoadOutcome::Loaded { lib: self.after_load(lib, true) };
            }
            Read::TooNew { lib, found, partial } => return self.too_new(lib, found, partial), // never read or touch the .bak
            Read::Missing => true,
            Read::Bad => false,
        };
        let main_copy = if main_missing { None } else { quarantine(&main).ok() };
        if main_copy.is_some() {
            self.copied = stamp_of(&main);
        }
        match read_library(&bak) {
            Read::Ok(lib) if main_missing || main_copy.is_some() => {
                let lib = self.after_load(lib, false);
                if !self.read_only {
                    // The gate sees the copied stamp: the damaged file is overwritten without rotation, never before its copy.
                    let _ = self.save(&lib);
                }
                self.pending = Some(Notice::Recovered { kept: main_copy });
                LoadOutcome::Loaded { lib }
            }
            Read::Ok(_) => self.damaged(main, None),
            // a newer .bak behind a missing or damaged main: open it read-only, no repair save, no rotation
            Read::TooNew { lib, found, partial } => self.too_new(lib, found, partial),
            Read::Missing if main_missing => LoadOutcome::Missing,
            Read::Missing => self.damaged(main, main_copy),
            Read::Bad => {
                let bak_copy = quarantine(&bak).ok();
                if main_missing {
                    self.damaged(bak, bak_copy)
                } else {
                    self.damaged(main, main_copy)
                }
            }
        }
    }

    /// The one gate in front of library.json. A good file is rotated into `.bak`; a file found
    /// unreadable is copied aside first (and never rotated), or nothing is written at all.
    pub fn save(&mut self, lib: &Library) -> Result<(), String> {
        if self.read_only {
            return Ok(());
        }
        let path = library_path(&self.dir);
        let now = disk(&path);
        let keep_bak = match now {
            Disk::Absent => false,
            Disk::Stamp(s) if Some(s) == self.written => true,
            Disk::Stamp(s) if Some(s) == self.copied => false,
            _ => match read_library(&path) {
                Read::Ok(_) => true,
                Read::Missing => false,
                Read::TooNew { found, partial, .. } => {
                    // another program saved a newer library behind our back: lock like a load does, so the banner shows and later changes are refused up front
                    self.problem = Some(Problem::TooNew { found, partial });
                    self.read_only = true;
                    return Err(READ_ONLY.into());
                }
                Read::Bad => {
                    let kept = quarantine(&path)?;
                    self.copied = stamp_of(&path);
                    self.pending = Some(Notice::Replaced { kept });
                    false
                }
            },
        };
        let text = serde_json::to_string_pretty(lib).map_err(|e| e.to_string())?;
        write_atomic(&path, text.as_bytes(), keep_bak)?;
        self.written = stamp_of(&path);
        if now_secs().saturating_sub(self.last_snapshot) > SNAPSHOT_MIN_GAP {
            self.snapshot(lib, "automatic")?;
        }
        Ok(())
    }

    pub fn snapshot(&mut self, lib: &Library, reason: &str) -> Result<(), String> {
        if self.read_only {
            return Ok(());
        }
        let t = now_secs();
        self.last_snapshot = t;
        let snap = Snapshot { taken: t, reason: reason.into(), library: lib.clone() };
        let text = serde_json::to_string(&snap).map_err(|e| e.to_string())?;
        let dir = history_dir(&self.dir);
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // never overwrite a snapshot: a second one in the same second gets -1, -2, ...
        let mut path = dir.join(format!("snapshot-{}.json", t));
        for n in 1.. {
            if !path.exists() {
                break;
            }
            path = dir.join(format!("snapshot-{}-{}.json", t, n));
        }
        write_atomic(&path, text.as_bytes(), false)?;
        self.prune(lib.settings.snapshots_to_keep.max(5));
        Ok(())
    }

    /// A snapshot, unless one was taken in the last `within` seconds (for small, frequent
    /// changes such as ticking off a shopping list one item at a time).
    pub fn snapshot_unless_recent(&mut self, lib: &Library, reason: &str, within: u64) -> Result<(), String> {
        if now_secs().saturating_sub(self.last_snapshot) < within {
            return Ok(());
        }
        self.snapshot(lib, reason)
    }

    fn prune(&self, keep: usize) {
        let mut list = self.snapshots();
        if list.len() > keep {
            // same second: shorter name first, so snapshot-5 is older than snapshot-5-1 and snapshot-5-2 than snapshot-5-10
            list.sort_by_key(|s| (s.taken, s.path.as_os_str().len(), s.path.clone()));
            for s in &list[..list.len() - keep] {
                let _ = fs::remove_file(&s.path);
            }
        }
    }

    pub fn snapshots(&self) -> Vec<SnapshotInfo> {
        let mut out = vec![];
        let Ok(rd) = fs::read_dir(history_dir(&self.dir)) else { return out };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map(|x| x == "json").unwrap_or(false) {
                if let Ok(t) = fs::read_to_string(&p) {
                    if let Ok(s) = serde_json::from_str::<Snapshot>(&t) {
                        out.push(SnapshotInfo {
                            path: p,
                            taken: s.taken,
                            reason: s.reason,
                            recipes: s.library.recipes.iter().filter(|r| !r.in_trash).count(),
                            favorites: s.library.recipes.iter().filter(|r| r.favorite).count(),
                        });
                    }
                }
            }
        }
        out.sort_by(|a, b| b.taken.cmp(&a.taken));
        out
    }
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    taken: u64,
    reason: String,
    library: Library,
}

#[derive(Clone, Debug)]
pub struct SnapshotInfo {
    pub path: PathBuf,
    pub taken: u64,
    pub reason: String,
    pub recipes: usize,
    pub favorites: usize,
}

pub fn load_snapshot(path: &Path) -> Result<Library, String> {
    let t = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let s: Snapshot = serde_json::from_str(&t).map_err(|e| e.to_string())?;
    Ok(s.library)
}

/// Err(NEWER_BACKUP) when a library read from a backup or snapshot has a `format` above this build's (D-07), so restoring it can't strip its newer fields.
pub fn ensure_not_newer(lib: &Library) -> Result<(), String> {
    if lib.format > LIBRARY_FORMAT {
        return Err(NEWER_BACKUP.into());
    }
    Ok(())
}

// ---------------- backup / restore ----------------

#[derive(Serialize, Deserialize)]
pub struct BackupFile {
    pub kind: String,
    pub app_version: String,
    pub created: u64,
    pub device: String,
    pub library: Library,
    /// Recipe photos (file name → base64 JPEG), so one file restores everything.
    #[serde(default)]
    pub photos: std::collections::BTreeMap<String, String>,
    /// The food diary and profile. None means the backup has no diary section: it was made before the
    /// tracker, or it is the library-only copy for the sync folder. Older builds ignore this key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub journal: Option<crate::tracker::journal::JournalBundle>,
}

// ---------------- recipe photos ----------------

pub const PHOTO_MAX_SIDE: u32 = 1200;

pub fn photos_dir(store_dir: &Path) -> PathBuf {
    store_dir.join("photos")
}

/// Turn any picture (JPEG, PNG, WebP, GIF, BMP) into a JPEG of at most 1200 px in the photos folder.
pub fn photo_name(stem: &str) -> String {
    let safe: String = stem.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
    format!("{}-{}.jpg", if safe.is_empty() { "photo".to_string() } else { safe }, now_secs())
}

pub fn save_photo_bytes(bytes: &[u8], photos: &Path, stem: &str) -> Result<String, String> {
    let name = photo_name(stem);
    write_photo(bytes, photos, &name)?;
    Ok(name)
}

/// JPEG quality for saved photos.
pub const PHOTO_JPEG_QUALITY: u8 = 90;

/// Scale factor and new size. Fit mode: max ÷ longer side; fill mode:
/// max ÷ shorter side. Only shrinks (factor < 1), sizes rounded.
pub fn photo_scale(fill: bool, max: u32, w: u32, h: u32) -> (u32, u32, f64) {
    let (w, h, m) = (w as f64, h as f64, max as f64);
    let s = if fill { if w < h { m / w } else { m / h } } else if w > h { m / w } else { m / h };
    if s < 1.0 {
        ((w * s).round().max(1.0) as u32, (h * s).round().max(1.0) as u32, s)
    } else {
        (w as u32, h as u32, 1.0)
    }
}

/// Decode, turn upright, shrink to ≤ 1200 px and save as JPEG under `name`.
pub fn write_photo(bytes: &[u8], photos: &Path, name: &str) -> Result<(), String> {
    use image::ImageDecoder;
    let bad = || "That file isn't a picture Tiffin can read (use JPEG, PNG, WebP, GIF or BMP).".to_string();
    let mut decoder = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|_| bad())?.into_decoder().map_err(|_| bad())?;
    let orientation = decoder.orientation().unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(|_| bad())?;
    // Camera orientation: 3 → 180°, 6 → 90°, 8 → 270°; other values untouched.
    use image::metadata::Orientation::*;
    img = match orientation {
        Rotate180 => img.rotate180(),
        Rotate90 => img.rotate90(),
        Rotate270 => img.rotate270(),
        _ => img,
    };
    let (nw, nh, s) = photo_scale(false, PHOTO_MAX_SIDE, img.width(), img.height());
    if s < 1.0 {
        img = img.resize_exact(nw, nh, image::imageops::FilterType::Triangle);
    }
    fs::create_dir_all(photos).map_err(|e| e.to_string())?;
    let rgb = img.to_rgb8();
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, PHOTO_JPEG_QUALITY).encode_image(&rgb).map_err(|e| e.to_string())?;
    fs::write(photos.join(name), out.into_inner()).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn import_photo_file(path: &Path, photos: &Path, stem: &str) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("Can't open {}: {}", path.display(), e))?;
    save_photo_bytes(&bytes, photos, stem)
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

fn collect_photos(lib: &Library, photos: &Path) -> std::collections::BTreeMap<String, String> {
    use base64::Engine;
    let mut map = std::collections::BTreeMap::new();
    for r in &lib.recipes {
        if r.photo.is_empty() {
            continue;
        }
        if let Ok(b) = fs::read(photos.join(&r.photo)) {
            map.insert(r.photo.clone(), b64().encode(b));
        }
    }
    map
}

/// Write the photos stored in a backup back into the photos folder.
pub fn restore_photos(b: &BackupFile, photos: &Path) -> usize {
    use base64::Engine;
    let _ = fs::create_dir_all(photos);
    let mut n = 0;
    for (name, data) in &b.photos {
        if name.contains(['/', '\\']) || name.contains("..") {
            continue;
        }
        if let Ok(bytes) = b64().decode(data) {
            if fs::write(photos.join(name), bytes).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Copy photos both ways between this device and the sync folder (missing files only).
pub fn sync_photos(lib: &Library, local: &Path, shared: &Path) {
    let _ = fs::create_dir_all(local);
    let _ = fs::create_dir_all(shared);
    for r in &lib.recipes {
        if r.photo.is_empty() {
            continue;
        }
        let (a, b) = (local.join(&r.photo), shared.join(&r.photo));
        if a.exists() && !b.exists() {
            let _ = fs::copy(&a, &b);
        } else if b.exists() && !a.exists() {
            let _ = fs::copy(&b, &a);
        }
    }
}

pub fn fmt_stamp(secs: u64) -> String {
    let day = (secs / 86400) as i32;
    let (y, m, d) = crate::model::civil_from_days(day);
    let s = secs % 86400;
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m, d, s / 3600, (s % 3600) / 60)
}

/// Whole library in one file.
pub fn backup_to(lib: &Library, dir: &Path) -> Result<PathBuf, String> {
    backup_with_photos(lib, dir, None, None)
}

/// Whole library plus its photos, and the diary when one is given, in one file. The name is made from the
/// minute; a second backup in the same minute gets " (2)", " (3)" and so on, so none overwrites another.
pub fn backup_with_photos(lib: &Library, dir: &Path, photos: Option<&Path>, journal: Option<&crate::tracker::journal::JournalBundle>) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let t = now_secs();
    let stem = format!("Tiffin backup {}", fmt_stamp(t).replace([':', ' '], "-"));
    let mut path = dir.join(format!("{stem}.{BACKUP_EXT}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).{BACKUP_EXT}"));
        n += 1;
    }
    let b = BackupFile {
        kind: "recipe-box-backup".into(),
        app_version: APP_VERSION.into(),
        created: t,
        device: lib.settings.device_name.clone(),
        library: lib.clone(),
        photos: photos.map(|p| collect_photos(lib, p)).unwrap_or_default(),
        journal: journal.cloned(),
    };
    write_atomic(&path, serde_json::to_string(&b).map_err(|e| e.to_string())?.as_bytes(), false)?;
    Ok(path)
}

pub fn read_backup(path: &Path) -> Result<BackupFile, String> {
    let t = fs::read_to_string(path).map_err(|e| format!("Can't open the backup: {}", e))?;
    parse_backup(&t)
}

/// The text of a backup file → the backup, or "This file isn't a Tiffin backup.".
pub fn parse_backup(t: &str) -> Result<BackupFile, String> {
    let b: BackupFile = serde_json::from_str(t).map_err(|_| "This file isn't a Tiffin backup.".to_string())?;
    if b.kind != "recipe-box-backup" {
        return Err("This file isn't a Tiffin backup.".into());
    }
    Ok(b)
}

/// The backups folder under both names (backups made before the rename still count while the old folder exists), then the extra folders.
pub fn backup_dirs(data: &Path, extra: &[PathBuf]) -> Vec<PathBuf> {
    let docs = documents_dir();
    let mut dirs = if data != data_dir() { vec![backups_dir(data)] } else { vec![docs.join(MOVED_FOLDERS[0].2), docs.join(MOVED_FOLDERS[0].1)] };
    for d in extra {
        if !dirs.contains(d) {
            dirs.push(d.clone());
        }
    }
    dirs
}

/// The backup files (by extension) in `dirs`, unread.
fn backup_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    dirs.iter().filter_map(|d| fs::read_dir(d).ok()).flat_map(|rd| rd.flatten().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == BACKUP_EXT)).collect()
}

/// Every backup in the backups folder and the extra folders: path, created, recipe count, and whether it has a diary.
pub fn list_backups(data: &Path, extra: &[PathBuf]) -> Vec<(PathBuf, u64, usize, bool)> {
    let mut out = vec![];
    for p in backup_files(&backup_dirs(data, extra)) {
        if let Ok(b) = read_backup(&p) {
            out.push((p, b.created, b.library.recipes.len(), b.journal.is_some()));
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Take the diary section out of every backup in `dirs` that has one. Only the `journal` key goes: the rest is
/// kept as JSON values (never through `BackupFile`, so keys a newer build wrote survive), and each file is
/// replaced atomically. A file that isn't a Tiffin backup is left alone. Returns how many changed and the
/// files that couldn't be read or changed (one that doesn't parse may still hold a diary, so it is named too).
pub fn strip_diary(dirs: &[PathBuf]) -> (usize, Vec<PathBuf>) {
    let (mut changed, mut failed) = (0, vec![]);
    for p in backup_files(dirs) {
        let Ok(text) = fs::read_to_string(&p) else {
            failed.push(p);
            continue;
        };
        let Ok(serde_json::Value::Object(mut b)) = serde_json::from_str::<serde_json::Value>(&text) else {
            failed.push(p);
            continue;
        };
        if b.get("kind").and_then(|k| k.as_str()) != Some("recipe-box-backup") || b.remove("journal").is_none() {
            continue;
        }
        match serde_json::to_string(&b).map_err(|e| e.to_string()).and_then(|t| write_atomic(&p, t.as_bytes(), false)) {
            Ok(()) => changed += 1,
            Err(_) => failed.push(p),
        }
    }
    (changed, failed)
}

// ---------------- sync (through a shared folder such as OneDrive or Dropbox) ----------------

#[derive(Default, Debug, Clone)]
pub struct SyncReport {
    pub pulled: usize,
    pub pushed: usize,
    pub deleted: usize,
    pub message: String,
}

fn merge_by<T: Clone, K: PartialEq>(mine: &mut Vec<T>, theirs: &[T], key: impl Fn(&T) -> K, modified: impl Fn(&T) -> u64) -> usize {
    let mut pulled = 0;
    for t in theirs {
        match mine.iter_mut().find(|m| key(m) == key(t)) {
            Some(m) => {
                if modified(t) > modified(m) {
                    *m = t.clone();
                    pulled += 1;
                }
            }
            None => {
                mine.push(t.clone());
                pulled += 1;
            }
        }
    }
    pulled
}

/// Two-way merge: newest edit of each recipe wins; favorites, ratings and collections travel with
/// the recipe; deletions travel as tombstones so they don't come back.
pub fn merge_libraries(mine: &mut Library, theirs: &Library) -> SyncReport {
    let mut rep = SyncReport::default();
    // tombstones
    for (uid, t) in &theirs.tombstones {
        if !mine.tombstones.iter().any(|(u, _)| u == uid) {
            mine.tombstones.push((uid.clone(), *t));
        }
    }
    let before = mine.recipes.len();
    let tomb = mine.tombstones.clone();
    mine.recipes.retain(|r| !tomb.iter().any(|(u, t)| *u == r.uid && *t >= r.modified));
    rep.deleted = before - mine.recipes.len();

    let mut next = mine.next_id;
    for t in &theirs.recipes {
        if tomb.iter().any(|(u, ts)| *u == t.uid && *ts >= t.modified) {
            continue;
        }
        match mine.recipes.iter_mut().find(|m| m.uid == t.uid) {
            Some(m) => {
                if t.modified > m.modified {
                    let id = m.id;
                    *m = t.clone();
                    m.id = id;
                    rep.pulled += 1;
                }
            }
            None => {
                let mut r: Recipe = t.clone();
                r.id = next;
                next += 1;
                mine.recipes.push(r);
                rep.pulled += 1;
            }
        }
    }
    mine.next_id = next.max(theirs.next_id);
    for c in &theirs.categories {
        if !mine.categories.contains(c) {
            mine.categories.push(c.clone());
        }
    }
    // Other lists merge by id (ids are allocated per library; collisions are rare and the newer wins).
    rep.pulled += merge_by(&mut mine.pantry, &theirs.pantry, |p| p.name.to_lowercase(), |p| p.modified);
    rep.pulled += merge_by(&mut mine.grocery_lists, &theirs.grocery_lists, |g| g.name.clone(), |g| g.modified);
    rep.pulled += merge_by(&mut mine.meals, &theirs.meals, |m| (m.day, m.kind as u8, m.name.clone()), |m| m.modified);
    rep.pulled += merge_by(&mut mine.menus, &theirs.menus, |m| m.name.clone(), |m| m.modified);
    rep.pulled += merge_by(&mut mine.my_foods, &theirs.my_foods, |f| f.name.to_lowercase(), |f| f.modified);
    crate::nutrition::set_my_foods(&mine.my_foods);
    rep.pushed = mine.recipes.iter().filter(|m| !theirs.recipes.iter().any(|t| t.uid == m.uid && t.modified >= m.modified)).count();
    rep
}

pub fn sync_folder(lib: &mut Library, store: &mut Store) -> Result<SyncReport, String> {
    if store.read_only {
        return Err("The library is open read-only, so it can't sync.".into());
    }
    let folder = lib.settings.sync_folder.trim().to_string();
    if folder.is_empty() {
        return Err("Choose a sync folder first (for example a OneDrive or Dropbox folder).".into());
    }
    let dir = PathBuf::from(&folder);
    fs::create_dir_all(&dir).map_err(|e| format!("Can't use that folder: {}", e))?;
    let path = dir.join(SYNC_FILE);
    // Only a missing file is a first sync; one that exists but can't be read is never overwritten.
    let shared_text = match fs::read_to_string(&path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Can't read the sync file in {} ({}). Nothing was changed; try syncing again in a moment.", dir.display(), e)),
    };
    if shared_text.as_deref().is_some_and(|t| too_new(t.as_bytes()).is_some()) {
        return Err("The sync folder was written by a newer version of Tiffin. Update this app before syncing; your library was not changed.".into());
    }
    store.snapshot(lib, "before sync")?;
    let mut rep = SyncReport::default();
    if let Some(t) = shared_text {
        let theirs: Library = serde_json::from_str(&t).map_err(|_| "The sync file is damaged; your library was not changed.".to_string())?;
        // keep local-only settings
        let settings = lib.settings.clone();
        rep = merge_libraries(lib, &theirs);
        lib.settings = settings;
    } else {
        rep.pushed = lib.recipes.len();
    }
    lib.settings.last_sync = Some(now_secs());
    let mut shared = lib.clone();
    shared.settings = Default::default();
    write_atomic(&path, serde_json::to_string(&shared).map_err(|e| e.to_string())?.as_bytes(), true)?;
    sync_photos(lib, &photos_dir(&store.dir), &dir.join("photos"));
    store.save(lib)?;
    rep.message = format!("Synced: {} received, {} sent, {} removed", rep.pulled, rep.pushed, rep.deleted);
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Recipe;

    fn test_jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, 120]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::codecs::png::PngEncoder::new(&mut out);
        image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    #[test]
    fn photo_scale_rule() {
        // fit: longer side to the limit, never enlarged
        assert_eq!(photo_scale(false, 1200, 2400, 1600), (1200, 800, 0.5));
        assert_eq!(photo_scale(false, 1200, 800, 600), (800, 600, 1.0));
        // fill: shorter side to the limit
        assert_eq!(photo_scale(true, 300, 1200, 600).0, 600);
        assert_eq!(photo_scale(true, 300, 1200, 600).1, 300);
    }

    #[test]
    fn photos_shrink_and_survive_backup() {
        let dir = std::env::temp_dir().join(format!("rb-photo-{}", std::process::id()));
        let photos = dir.join("photos");
        let name = save_photo_bytes(&test_jpeg(2400, 1600), &photos, "Pad Thai").unwrap();
        let saved = image::open(photos.join(&name)).unwrap();
        assert_eq!((saved.width(), saved.height()), (1200, 800));
        let mut lib = Library::default();
        lib.add_recipe(Recipe { name: "Pad Thai".into(), photo: name.clone(), ..Default::default() });
        let p = backup_with_photos(&lib, &dir.join("bk"), Some(&photos), None).unwrap();
        let b = read_backup(&p).unwrap();
        assert_eq!(b.photos.len(), 1);
        let other = dir.join("restored");
        assert_eq!(restore_photos(&b, &other), 1);
        assert!(other.join(&name).exists());
        assert!(save_photo_bytes(b"not a picture", &photos, "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_backups_in_one_minute_both_survive() {
        let dir = std::env::temp_dir().join(format!("rb-twobk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut lib = Library::default();
        lib.add_recipe(Recipe { name: "One".into(), ..Default::default() });
        let a = backup_to(&lib, &dir).unwrap();
        lib.add_recipe(Recipe { name: "Two".into(), ..Default::default() });
        let b = backup_to(&lib, &dir).unwrap();
        assert_ne!(a, b);
        assert_eq!(read_backup(&a).unwrap().library.recipes.len(), 1);
        assert_eq!(read_backup(&b).unwrap().library.recipes.len(), 2);
        assert!(!list_backups(&data_dir(), &[dir.clone()]).is_empty());
        assert!(parse_backup("{}").is_err() && parse_backup(&fs::read_to_string(&a).unwrap()).unwrap().journal.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    fn put_file(p: &Path, bytes: &[u8]) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, bytes).unwrap();
    }

    #[test]
    fn move_folder_copies_verifies_then_deletes() {
        let d = tdir("mv-ok");
        let (old, new) = (d.join("old"), d.join("new"));
        put_file(&old.join("a.recipebox"), b"AAA");
        put_file(&old.join("Imported/b.json"), b"BBB");
        move_folder(&old, &new).unwrap();
        assert_eq!(fs::read(new.join("a.recipebox")).unwrap(), b"AAA");
        assert_eq!(fs::read(new.join("Imported/b.json")).unwrap(), b"BBB");
        assert!(!old.exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn move_folder_never_overwrites_and_keeps_the_old_copy() {
        let d = tdir("mv-conflict");
        let (old, new) = (d.join("old"), d.join("new"));
        put_file(&old.join("a.recipebox"), b"OLD");
        put_file(&old.join("b.recipebox"), b"BBB");
        put_file(&new.join("a.recipebox"), b"NEW");
        assert!(move_folder(&old, &new).is_err());
        assert_eq!(fs::read(new.join("a.recipebox")).unwrap(), b"NEW");
        assert_eq!(fs::read(old.join("a.recipebox")).unwrap(), b"OLD");
        assert_eq!(fs::read(old.join("b.recipebox")).unwrap(), b"BBB");
        // an identical file counts as moved
        put_file(&new.join("a.recipebox"), b"OLD");
        move_folder(&old, &new).unwrap();
        assert!(!old.exists());
        assert_eq!(fs::read(new.join("b.recipebox")).unwrap(), b"BBB");
        // a missing old folder is fine and creates nothing
        let (none, fresh) = (d.join("none"), d.join("fresh"));
        move_folder(&none, &fresh).unwrap();
        assert!(!none.exists() && !fresh.exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn move_folder_fails_safe_when_the_target_cannot_be_made() {
        let d = tdir("mv-blocked");
        let (old, new) = (d.join("old"), d.join("new"));
        put_file(&old.join("a.recipebox"), b"AAA");
        put_file(&old.join("Imported/b.json"), b"BBB");
        fs::write(&new, b"i am a file").unwrap();
        assert!(move_folder(&old, &new).is_err());
        assert_eq!(fs::read(old.join("a.recipebox")).unwrap(), b"AAA");
        assert_eq!(fs::read(old.join("Imported/b.json")).unwrap(), b"BBB");
        assert_eq!(fs::read(&new).unwrap(), b"i am a file");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn pick_folder_keeps_the_old_folder_while_it_holds_files() {
        let d = tdir("pick");
        assert_eq!(pick_folder(&d, "Old", "New"), d.join("New"));
        fs::create_dir_all(d.join("Old")).unwrap();
        assert_eq!(pick_folder(&d, "Old", "New"), d.join("New"), "an empty old folder is not in use");
        put_file(&d.join("Old/x.txt"), b"x");
        assert_eq!(pick_folder(&d, "Old", "New"), d.join("Old"));
        fs::remove_file(d.join("Old/x.txt")).unwrap();
        assert_eq!(pick_folder(&d, "Old", "New"), d.join("New"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn old_named_backups_still_list() {
        let d = tdir("oldbk");
        let mut lib = Library::default();
        lib.add_recipe(Recipe { name: "One".into(), ..Default::default() });
        let p = backup_to(&lib, &d).unwrap();
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("Tiffin backup "));
        let old = d.join("Recipe Box backup 2026-01-01-10-00.recipebox");
        fs::rename(&p, &old).unwrap();
        assert!(list_backups(&data_dir(), &[d.clone()]).iter().any(|b| b.0 == old));
        assert_eq!(read_backup(&old).unwrap().library.recipes.len(), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_test_copy_keeps_its_backups_out_of_documents() {
        let d = tdir("ownbk");
        let docs = documents_dir();
        assert_eq!(backups_dir(&d), d.join("Backups"));
        let sync = d.join("sync");
        let dirs = backup_dirs(&d, &[sync.clone()]);
        assert_eq!(dirs, vec![d.join("Backups"), sync]);
        assert!(dirs.iter().all(|x| !x.starts_with(&docs)));
        // the real data folder still uses Documents
        assert!(backups_dir(&data_dir()).starts_with(&docs));
        let _ = fs::remove_dir_all(&d);
    }

    /// Builds a library with photos for screenshot checks: RB_DEMO=<folder> cargo test photo_demo -- --ignored
    #[test]
    #[ignore]
    fn photo_demo() {
        let dir = PathBuf::from(std::env::var("RB_DEMO").unwrap());
        let mut lib = crate::model::sample_library(crate::model::today());
        let photos = photos_dir(&dir);
        for (i, r) in lib.recipes.iter_mut().enumerate().take(4) {
            let img = image::RgbImage::from_fn(800, 600, |x, y| {
                let t = (x as f32 / 800.0, y as f32 / 600.0);
                image::Rgb([(200.0 - 120.0 * t.1 + 20.0 * i as f32) as u8, (120.0 + 80.0 * t.0) as u8, (60.0 + 40.0 * i as f32) as u8])
            });
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
            r.photo = save_photo_bytes(&out.into_inner(), &photos, &format!("demo{}", i)).unwrap();
        }
        let mut st = Store::new(dir, false);
        st.save(&lib).unwrap();
    }

    #[test]
    fn pantry_expiry_order() {
        use crate::model::{days_from_civil, expiry_status, Expiry};
        let today = days_from_civil(2026, 11, 4); // a Wednesday
        assert_eq!(expiry_status(today - 1, today), Expiry::Expired);
        assert_eq!(expiry_status(today, today), Expiry::Today);
        assert_eq!(expiry_status(today + 3, today), Expiry::ThisWeek); // Saturday
        assert_eq!(expiry_status(today + 4, today), Expiry::ThisMonth); // next Sunday
        assert_eq!(expiry_status(today + 30, today), Expiry::Later);
    }

    #[test]
    fn favorites_survive_sync_and_newest_wins() {
        let mut a = Library::default();
        let id = a.add_recipe(Recipe { name: "Soup".into(), uid: "u1".into(), modified: 10, ..Default::default() });
        let mut b = a.clone();
        b.recipe_mut(id).unwrap().favorite = true;
        b.recipe_mut(id).unwrap().categories.push("Soups".into());
        b.recipe_mut(id).unwrap().modified = 20;
        b.add_recipe(Recipe { name: "Bread".into(), uid: "u2".into(), modified: 15, ..Default::default() });
        let rep = merge_libraries(&mut a, &b);
        assert_eq!(rep.pulled, 2);
        let s = a.recipes.iter().find(|r| r.uid == "u1").unwrap();
        assert!(s.favorite);
        assert_eq!(s.categories, vec!["Soups".to_string()]);
        // deletion travels
        let mut c = a.clone();
        c.recipes.retain(|r| r.uid != "u2");
        c.tombstones.push(("u2".into(), 30));
        merge_libraries(&mut a, &c);
        assert!(!a.recipes.iter().any(|r| r.uid == "u2"));
    }

    #[test]
    fn backup_round_trip_and_snapshots() {
        let dir = std::env::temp_dir().join(format!("recipebox-test-{}", now_secs() ^ std::process::id() as u64));
        let mut lib = crate::model::sample_library(20000);
        let p = backup_to(&lib, &dir).unwrap();
        let b = read_backup(&p).unwrap();
        assert_eq!(b.library.recipes.len(), lib.recipes.len());
        let mut st = Store::new(dir.clone(), false);
        st.snapshot(&lib, "one").unwrap();
        lib.recipes.clear();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        st.snapshot(&lib, "two").unwrap();
        let snaps = st.snapshots();
        assert_eq!(snaps.len(), 2);
        let old = load_snapshot(&snaps[1].path).unwrap();
        assert!(!old.recipes.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    // ---------------- load states, the save gate and the lock ----------------

    /// A fresh, empty temp folder of its own.
    fn tdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rb-store-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn main_of(d: &Path) -> PathBuf {
        d.join("library.json")
    }
    fn bak_of(d: &Path) -> PathBuf {
        d.join("library.json.bak")
    }
    fn good(n: usize) -> Library {
        let mut lib = Library::default();
        for i in 0..n {
            lib.add_recipe(Recipe { name: format!("Soup {}", i), ..Default::default() });
        }
        lib
    }
    fn put(p: &Path, lib: &Library) {
        fs::write(p, serde_json::to_string_pretty(lib).unwrap()).unwrap();
    }
    fn copies(d: &Path) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(d).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("library-damaged-")).collect();
        v.sort();
        v
    }
    fn recipes(o: LoadOutcome) -> usize {
        match o {
            LoadOutcome::Loaded { lib } => lib.recipes.len(),
            _ => panic!("expected Loaded"),
        }
    }

    #[test]
    fn can_snapshot_only_when_writable_and_clean() {
        let d = tdir("can-snap");
        let mut st = Store::new(d.clone(), false);
        assert!(matches!(st.load(), LoadOutcome::Missing));
        assert!(st.can_snapshot());
        st.read_only = true;
        assert!(!st.can_snapshot());
        let bad = tdir("can-snap-bad");
        fs::write(main_of(&bad), "not json").unwrap();
        let mut st = Store::new(bad.clone(), false);
        assert!(matches!(st.load(), LoadOutcome::Damaged));
        assert!(!st.can_snapshot());
        let _ = fs::remove_dir_all(&d);
        let _ = fs::remove_dir_all(&bad);
    }

    #[test]
    fn load_missing_is_first_run() {
        let d = tdir("missing");
        let mut st = Store::new(d.clone(), false);
        assert!(matches!(st.load(), LoadOutcome::Missing));
        assert!(st.problem.is_none() && st.take_notice().is_none());
        assert!(!main_of(&d).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn load_damaged_keeps_the_file_and_locks() {
        for (i, bytes) in ["", "{\"recipes\": [", "{\"recipes\": 5}"].iter().enumerate() {
            let d = tdir(&format!("damaged{}", i));
            fs::write(main_of(&d), bytes).unwrap();
            let mut st = Store::new(d.clone(), false);
            assert!(matches!(st.load(), LoadOutcome::Damaged), "{bytes:?}");
            assert_eq!(fs::read(main_of(&d)).unwrap(), bytes.as_bytes());
            assert!(st.save(&good(2)).is_ok());
            assert_eq!(fs::read(main_of(&d)).unwrap(), bytes.as_bytes(), "a save must not touch it");
            let c = copies(&d);
            assert_eq!(c.len(), 1);
            assert_eq!(fs::read(&c[0]).unwrap(), bytes.as_bytes());
            assert!(!bak_of(&d).exists());
            assert_eq!(st.check_unlocked(), Err(DAMAGED.to_string()));
            let _ = fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn load_bad_main_good_bak_recovers() {
        let d = tdir("recover");
        fs::write(main_of(&d), "{\"recipes\": [").unwrap();
        put(&bak_of(&d), &good(3));
        let bak_bytes = fs::read(bak_of(&d)).unwrap();
        let mut st = Store::new(d.clone(), false);
        assert_eq!(recipes(st.load()), 3);
        assert!(matches!(read_library(&main_of(&d)), Read::Ok(l) if l.recipes.len() == 3));
        assert_eq!(fs::read(bak_of(&d)).unwrap(), bak_bytes);
        let c = copies(&d);
        assert_eq!(c.len(), 1);
        assert_eq!(fs::read(&c[0]).unwrap(), b"{\"recipes\": [");
        assert_eq!(st.take_notice(), Some(Notice::Recovered { kept: Some(c[0].clone()) }));
        assert_eq!(st.take_notice(), None);
        assert!(st.problem.is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_main_good_bak_recovers_with_notice() {
        let d = tdir("missingmain");
        put(&bak_of(&d), &good(2));
        let bak_bytes = fs::read(bak_of(&d)).unwrap();
        let mut st = Store::new(d.clone(), false);
        assert_eq!(recipes(st.load()), 2);
        assert!(matches!(read_library(&main_of(&d)), Read::Ok(l) if l.recipes.len() == 2));
        assert_eq!(fs::read(bak_of(&d)).unwrap(), bak_bytes);
        assert_eq!(st.take_notice(), Some(Notice::Recovered { kept: None }));
        assert_eq!(st.take_notice(), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_main_never_becomes_the_bak() {
        let d = tdir("neverbak");
        fs::write(main_of(&d), "garbage one").unwrap();
        put(&bak_of(&d), &good(1));
        let mut st = Store::new(d.clone(), false);
        let lib = match st.load() {
            LoadOutcome::Loaded { lib } => lib,
            _ => panic!(),
        };
        for _ in 0..2 {
            st.save(&lib).unwrap();
            assert_ne!(fs::read(bak_of(&d)).unwrap(), b"garbage one");
        }
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn damage_found_later_never_reaches_the_bak() {
        let d = tdir("later");
        put(&main_of(&d), &good(2));
        put(&bak_of(&d), &good(1));
        let bak_bytes = fs::read(bak_of(&d)).unwrap();
        let mut st = Store::new(d.clone(), false);
        let lib = match st.load() {
            LoadOutcome::Loaded { lib } => lib,
            _ => panic!(),
        };
        assert!(st.take_notice().is_none());
        fs::write(main_of(&d), "another program wrote this").unwrap();
        st.save(&lib).unwrap();
        assert_eq!(fs::read(bak_of(&d)).unwrap(), bak_bytes);
        let c = copies(&d);
        assert_eq!(c.len(), 1);
        assert_eq!(fs::read(&c[0]).unwrap(), b"another program wrote this");
        assert!(matches!(read_library(&main_of(&d)), Read::Ok(_)));
        assert_eq!(st.take_notice(), Some(Notice::Replaced { kept: c[0].clone() }));
        assert_eq!(st.take_notice(), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn repeated_starts_keep_one_damaged_copy() {
        let d = tdir("repeat");
        fs::write(main_of(&d), "garbage").unwrap();
        for _ in 0..3 {
            assert!(matches!(Store::new(d.clone(), false).load(), LoadOutcome::Damaged));
        }
        assert_eq!(copies(&d).len(), 1);
        assert_eq!(fs::read(&copies(&d)[0]).unwrap(), b"garbage");
        fs::write(main_of(&d), "different garbage").unwrap();
        assert!(matches!(Store::new(d.clone(), false).load(), LoadOutcome::Damaged));
        assert_eq!(copies(&d).len(), 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(windows)]
    #[test]
    fn uncopyable_damage_stays_locked() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tdir("uncopyable");
        fs::write(main_of(&d), "garbage").unwrap();
        put(&bak_of(&d), &good(1));
        let bak_bytes = fs::read(bak_of(&d)).unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(main_of(&d)).unwrap();
        let mut st = Store::new(d.clone(), false);
        assert!(matches!(st.load(), LoadOutcome::Damaged));
        assert!(matches!(st.problem, Some(Problem::Damaged { kept: None, .. })));
        assert_eq!(st.check_unlocked(), Err(DAMAGED.to_string()));
        assert_eq!(st.can_replace(), Err(NO_SAFETY_COPY.to_string()));
        st.read_only = false; // as if a caller forgot the lock
        assert_eq!(st.save(&good(2)), Err(NO_SAFETY_COPY.to_string()));
        drop(held);
        assert_eq!(fs::read(main_of(&d)).unwrap(), b"garbage");
        assert_eq!(fs::read(bak_of(&d)).unwrap(), bak_bytes);
        assert!(copies(&d).is_empty());
        assert!(st.can_replace().is_ok());
        let c = copies(&d);
        assert_eq!(c.len(), 1);
        assert_eq!(fs::read(&c[0]).unwrap(), b"garbage");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_store_refuses_until_cleared() {
        let d = tdir("refuses");
        fs::write(main_of(&d), "garbage").unwrap();
        let mut st = Store::new(d.clone(), false);
        assert!(matches!(st.load(), LoadOutcome::Damaged));
        assert_eq!(st.check_unlocked(), Err(DAMAGED.to_string()));
        assert!(st.can_replace().is_ok());
        assert!(matches!(st.take_notice(), Some(Notice::Damaged { .. })));
        assert!(matches!(st.take_notice(), Some(Notice::Damaged { .. })));
        st.clear_damage();
        assert!(st.check_unlocked().is_ok() && !st.read_only);
        assert!(st.take_notice().is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn read_only_store_never_syncs() {
        let d = tdir("nosync");
        let sync = d.join("shared");
        fs::write(main_of(&d), "garbage").unwrap();
        let mut st = Store::new(d.clone(), false);
        assert!(matches!(st.load(), LoadOutcome::Damaged));
        let mut lib = good(1);
        lib.settings.sync_folder = sync.to_string_lossy().to_string();
        assert!(sync_folder(&mut lib, &mut st).is_err());
        assert!(!sync.exists() || fs::read_dir(&sync).unwrap().next().is_none());
        let _ = fs::remove_dir_all(&d);
    }

    /// A library file from a newer build: two recipes, a field and a meal kind this build doesn't know.
    fn newer_text() -> String {
        // built from LIBRARY_FORMAT so the next bump can't quietly turn this into a current-format file
        format!(
            r#"{{"format": {}, "app_version": "9.0.0", "future_field": {{"a": 1}}, "categories": ["Soups"],
        "recipes": [{{"id": 1, "name": "Soup A"}}, {{"id": 2, "name": "Soup B"}}],
        "meals": [{{"id": 1, "name": "x", "kind": "Brunch-of-the-future"}}]}}"#,
            LIBRARY_FORMAT + 1
        )
    }

    #[test]
    fn newer_library_opens_read_only_and_is_not_rewritten() {
        let d = tdir("newer");
        fs::write(main_of(&d), newer_text()).unwrap();
        let before = fs::read(main_of(&d)).unwrap();
        let mut st = Store::new(d.clone(), false);
        let lib = match st.load() {
            LoadOutcome::TooNew { lib } => lib,
            _ => panic!("expected TooNew"),
        };
        assert_eq!(st.problem, Some(Problem::TooNew { found: "9.0.0".into(), partial: true }));
        assert_eq!(lib.recipes.len(), 2);
        assert_eq!(lib.categories, vec!["Soups".to_string()]);
        assert_eq!(st.check_unlocked(), Err(READ_ONLY.to_string()));
        assert_eq!(st.can_replace(), Err(READ_ONLY.to_string()));
        assert!(st.take_notice().is_none());
        assert!(st.problem.as_ref().unwrap().banner().contains("Some items can't be shown"));
        assert!(st.save(&lib).is_ok());
        assert!(st.snapshot(&lib, "x").is_ok());
        assert_eq!(fs::read(main_of(&d)).unwrap(), before);
        assert!(!d.join("history").exists() && !bak_of(&d).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn older_library_still_gets_the_update_snapshot() {
        let d = tdir("older");
        let mut lib = good(1);
        lib.app_version = "0.0.1".into();
        put(&main_of(&d), &lib);
        let mut st = Store::new(d.clone(), false);
        assert_eq!(recipes(st.load()), 1);
        assert!(st.problem.is_none());
        assert!(st.snapshots().iter().any(|s| s.reason == "before update from 0.0.1"));
        let _ = fs::remove_dir_all(&d);
    }

    /// A format-1 library with one My Food: fiber printed as 0, sugar not given.
    const FORMAT1: &str = r#"{"format": 1, "app_version": "0.0.1", "my_foods": [{"name": "Whey", "serving_g": 31, "per_serving": {"calories": 120, "fiber_g": 0}}]}"#;

    #[test]
    fn format1_library_loads_as_format2() {
        let d = tdir("format1");
        fs::write(main_of(&d), FORMAT1).unwrap();
        let mut st = Store::new(d.clone(), false);
        let LoadOutcome::Loaded { lib } = st.load() else { panic!("expected Loaded") };
        let f = &lib.my_foods[0];
        assert_eq!((f.per_serving.calories, f.per_serving.fiber_g, f.per_serving.sugar_g), (Some(120.0), Some(0.0), None));
        assert_eq!((f.piece.as_str(), f.piece_g), ("", 0.0));
        assert_eq!(lib.format, LIBRARY_FORMAT);
        assert!(st.snapshots().iter().any(|s| s.reason == "before update from 0.0.1"), "the old file is snapshotted first");
        st.save(&lib).unwrap();
        let text = fs::read_to_string(main_of(&d)).unwrap();
        assert!(text.contains(&format!("\"format\": {}", LIBRARY_FORMAT)));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["my_foods"][0]["per_serving"], serde_json::json!({"calories": 120.0, "fiber_g": 0.0}), "unknown is a missing key, never null or 0");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn failed_update_snapshot_blocks_the_format_upgrade() {
        let d = tdir("nosnap");
        fs::write(main_of(&d), FORMAT1).unwrap();
        fs::write(d.join("history"), "a file where the history folder should be").unwrap();
        let LoadOutcome::Loaded { lib } = Store::new(d.clone(), false).load() else { panic!("expected Loaded") };
        assert_eq!((lib.format, lib.app_version.as_str()), (1, "0.0.1"));
        assert_eq!(fs::read(main_of(&d)).unwrap(), FORMAT1.as_bytes());
        assert!(!bak_of(&d).exists(), "no .bak while the snapshot failed");
        fs::remove_file(d.join("history")).unwrap();
        let mut st = Store::new(d.clone(), false);
        let LoadOutcome::Loaded { lib } = st.load() else { panic!("expected Loaded") };
        assert_eq!((lib.format, lib.app_version.as_str()), (LIBRARY_FORMAT, APP_VERSION));
        assert!(st.snapshots().iter().any(|s| s.reason.starts_with("before update from")));
        let on_disk: Library = serde_json::from_str(&fs::read_to_string(main_of(&d)).unwrap()).unwrap();
        assert_eq!((on_disk.format, on_disk.app_version.as_str()), (LIBRARY_FORMAT, APP_VERSION));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_format_2_library_upgrades_to_3_with_its_foods_unchanged() {
        let d = tdir("f2to3");
        // this build's own version: only the format can trigger the upgrade
        let f2 = format!(r#"{{"format": {}, "app_version": "{}", "my_foods": [{{"id": 4, "name": "Soy milk", "serving": "1 cup", "serving_g": 250, "serving_ml": 250, "per_serving": {{"calories": 100, "protein_g": 7}}, "source": "Photo of the label"}}]}}"#, LIBRARY_FORMAT - 1, APP_VERSION);
        fs::write(main_of(&d), f2).unwrap();
        let mut st = Store::new(d.clone(), false);
        let LoadOutcome::Loaded { lib } = st.load() else { panic!("expected Loaded") };
        assert_eq!(lib.format, LIBRARY_FORMAT);
        assert!(st.snapshots().iter().any(|s| s.reason.starts_with("before update from")), "the older-format file is kept first");
        let on_disk: Library = serde_json::from_str(&fs::read_to_string(main_of(&d)).unwrap()).unwrap();
        let f = &on_disk.my_foods[0];
        assert_eq!((on_disk.format, f.id, f.name.as_str(), f.serving_g, f.serving_ml, f.barcode.as_str()), (LIBRARY_FORMAT, 4, "Soy milk", 250.0, 250.0, ""));
        assert_eq!((f.per_serving.calories, f.per_serving.protein_g, f.per_serving.fat_g), (Some(100.0), Some(7.0), None));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn update_snapshot_is_taken_once() {
        let d = tdir("once");
        fs::write(main_of(&d), FORMAT1).unwrap();
        let mut st = Store::new(d.clone(), false);
        st.load();
        let on_disk: Library = serde_json::from_str(&fs::read_to_string(main_of(&d)).unwrap()).unwrap();
        assert_eq!((on_disk.format, on_disk.app_version.as_str()), (LIBRARY_FORMAT, APP_VERSION));
        let old: Library = serde_json::from_str(&fs::read_to_string(bak_of(&d)).unwrap()).unwrap();
        assert_eq!(old.app_version, "0.0.1", "the old file is kept as .bak");
        let taken = |s: &Store| s.snapshots().iter().filter(|x| x.reason == "before update from 0.0.1").count();
        assert_eq!(taken(&st), 1);
        // a second start: asserted on last_snapshot, because same-second snapshots share a file name
        let mut again = Store::new(d.clone(), false);
        again.load();
        assert_eq!(again.last_snapshot, 0);
        assert_eq!(taken(&again), 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn same_second_snapshots_never_overwrite() {
        let d = tdir("samesec");
        let mut st = Store::new(d.clone(), false);
        let lib = good(1);
        for i in 0..5 {
            st.snapshot(&lib, &format!("reason {i}")).unwrap();
        }
        let files = fs::read_dir(d.join("history")).unwrap().count();
        assert_eq!(files, 5, "five snapshots, five files, even when taken in the same second");
        let mut reasons: Vec<String> = st.snapshots().into_iter().map(|s| s.reason).collect();
        reasons.sort();
        assert_eq!(reasons, (0..5).map(|i| format!("reason {i}")).collect::<Vec<_>>());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn read_only_load_bumps_nothing() {
        let d = tdir("roload");
        fs::write(main_of(&d), FORMAT1).unwrap();
        let LoadOutcome::Loaded { lib } = Store::new(d.clone(), true).load() else { panic!("expected Loaded") };
        assert_eq!((lib.format, lib.app_version.as_str()), (1, "0.0.1"));
        assert_eq!(fs::read(main_of(&d)).unwrap(), FORMAT1.as_bytes());
        assert!(!bak_of(&d).exists() && !d.join("history").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_main_with_newer_bak_stays_read_only() {
        let d = tdir("newerbak");
        fs::write(bak_of(&d), newer_text()).unwrap();
        let bak_bytes = fs::read(bak_of(&d)).unwrap();
        let mut st = Store::new(d.clone(), false);
        let lib = match st.load() {
            LoadOutcome::TooNew { lib } => lib,
            _ => panic!("expected TooNew"),
        };
        assert!(matches!(st.problem, Some(Problem::TooNew { .. })));
        assert!(st.save(&lib).is_ok());
        assert!(!main_of(&d).exists());
        assert_eq!(fs::read(bak_of(&d)).unwrap(), bak_bytes);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_file_appearing_behind_our_back_is_not_overwritten() {
        let d = tdir("newerlate");
        put(&main_of(&d), &good(1));
        let mut st = Store::new(d.clone(), false);
        let lib = match st.load() {
            LoadOutcome::Loaded { lib } => lib,
            _ => panic!(),
        };
        fs::write(main_of(&d), newer_text()).unwrap();
        assert_eq!(st.save(&lib), Err(READ_ONLY.to_string()));
        // the save locks the store like a load does: the banner shows and later changes are refused up front
        assert_eq!(st.problem, Some(Problem::TooNew { found: "9.0.0".into(), partial: true }));
        assert!(st.read_only);
        assert_eq!(st.check_unlocked(), Err(READ_ONLY.to_string()));
        assert_eq!(st.can_replace(), Err(READ_ONLY.to_string()));
        assert!(st.problem.as_ref().unwrap().banner().contains("9.0.0"));
        assert_eq!(st.save(&lib), Ok(())); // the read-only no-op: writes nothing
        assert_eq!(fs::read(main_of(&d)).unwrap(), newer_text().as_bytes());
        assert!(!bak_of(&d).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn unreadable_sync_file_is_not_overwritten() {
        let d = tdir("unreadablesync");
        let shared = d.join("shared");
        let mut st = Store::new(d.clone(), false);
        let mut lib = good(2);
        lib.settings.sync_folder = shared.to_string_lossy().to_string();
        assert_eq!(sync_folder(&mut lib, &mut st).unwrap().pushed, 2); // a missing sync file is a first sync
        assert!(shared.join(SYNC_FILE).exists());
        fs::write(shared.join(SYNC_FILE), [0xff, 0xfe, b'{']).unwrap(); // exists, but isn't text
        let before = fs::read(shared.join(SYNC_FILE)).unwrap();
        let err = sync_folder(&mut lib, &mut st).unwrap_err();
        assert!(err.contains("Can't read the sync file"), "{err}");
        assert_eq!(fs::read(shared.join(SYNC_FILE)).unwrap(), before);
        assert_eq!(lib.recipes.len(), 2);
        assert!(!shared.join("recipe-box-sync.json.bak").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn newer_sync_file_is_not_merged() {
        let d = tdir("newersync");
        let shared = d.join("shared");
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join(SYNC_FILE), newer_text()).unwrap();
        let before = fs::read(shared.join(SYNC_FILE)).unwrap();
        let mut st = Store::new(d.clone(), false);
        let mut lib = good(1);
        lib.settings.sync_folder = shared.to_string_lossy().to_string();
        let err = sync_folder(&mut lib, &mut st).unwrap_err();
        assert!(err.contains("written by a newer version"), "{err}");
        assert_eq!(lib.recipes.len(), 1);
        assert_eq!(fs::read(shared.join(SYNC_FILE)).unwrap(), before);
        assert!(!d.join("history").exists() && !main_of(&d).exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn data_folder_name_is_stable() {
        assert_eq!(DATA_FOLDER, "RecipeBox");
        assert!(data_dir().ends_with(DATA_FOLDER));
    }
}
