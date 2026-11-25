//! Crash test for the library's atomic save, run in the browser by desktop/web-tests/crash.html.
//! `crash_save save <dir>` saves library versions until it is killed (version n holds n recipes, each
//! stamped "v<n>", sized by n so a version can be shorter than the one before), with a snapshot every
//! third save. `crash_save check <dir>` loads what is left and prints one line:
//! `OK <n> main|backup [cut-mid-write] [bak-broken] [snapshot-broken]`, `MISSING` or `BAD <why>`.
use recipe_box::model::{Library, Recipe};
use recipe_box::storage::{load_snapshot, LoadOutcome, Notice, Store};
use std::path::{Path, PathBuf};

/// Bytes of padding per recipe in version n: 40, 25 or 10 KB, so some saves shrink the file.
fn pad_len(n: usize) -> usize {
    [40_000, 25_000, 10_000][n % 3]
}

fn version(n: usize) -> Library {
    let pad = "x".repeat(pad_len(n));
    let mut lib = Library::default();
    lib.recipes = (0..n).map(|i| Recipe { id: i as u64 + 1, name: format!("r{i}"), directions: format!("v{n}"), ingredients: pad.clone(), ..Recipe::default() }).collect();
    lib
}

/// Exactly version n, nothing mixed in from another.
fn whole(lib: &Library) -> bool {
    let n = lib.recipes.len();
    lib.recipes.iter().enumerate().all(|(i, r)| r.name == format!("r{i}") && r.directions == format!("v{n}") && r.ingredients.len() == pad_len(n))
}

/// A library file read on its own (`.bak` holds a plain library).
fn file_whole(p: &Path) -> bool {
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str::<Library>(&t).ok()).is_some_and(|l| whole(&l))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(args.get(2).map(String::as_str).unwrap_or("/data"));
    let mut store = Store::new(dir.clone(), false);
    match args.get(1).map(String::as_str) {
        Some("save") => {
            let start = match store.load() {
                LoadOutcome::Loaded { lib } => lib.recipes.len(),
                _ => 0,
            };
            for n in start + 1.. {
                let lib = version(n);
                store.save(&lib).expect("save");
                if n % 3 == 0 {
                    store.snapshot(&lib, "crash test").expect("snapshot");
                }
                println!("saved {n}");
            }
        }
        Some("check") => {
            // looked at before load, which may repair (and so use up) the leftovers
            let cut = dir.join("library.tmp").exists();
            let bak = dir.join("library.json.bak");
            let bak_broken = bak.exists() && !file_whole(&bak);
            let mut line = match store.load() {
                LoadOutcome::Missing => "MISSING".to_string(),
                LoadOutcome::Damaged => "BAD damaged".to_string(),
                LoadOutcome::TooNew { .. } => "BAD too new".to_string(),
                LoadOutcome::Loaded { lib } => {
                    let from = if matches!(store.take_notice(), Some(Notice::Recovered { .. })) { "backup" } else { "main" };
                    if whole(&lib) { format!("OK {} {from}", lib.recipes.len()) } else { format!("BAD mixed {}", lib.recipes.len()) }
                }
            };
            // every file in history/, not Store::snapshots(), which leaves out the ones it can't read
            let snapshot_broken = std::fs::read_dir(dir.join("history"))
                .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).any(|p| !load_snapshot(&p).is_ok_and(|l| whole(&l))))
                .unwrap_or(false);
            for (on, tag) in [(cut, " cut-mid-write"), (bak_broken, " bak-broken"), (snapshot_broken, " snapshot-broken")] {
                if on {
                    line.push_str(tag);
                }
            }
            println!("{line}");
        }
        _ => eprintln!("usage: crash_save save|check <dir>"),
    }
}
