//! Updates from GitHub Releases (D-07..D-11). The check, the download, the signature check and the install
//! all run here; the page only shows the answer.
// shortcut: the Tauri updater is linked on desktop targets only (see Cargo.toml); a mobile build would gate this module.

use serde::Serialize;
use tauri_plugin_updater::UpdaterExt;

use crate::Shared;

// Lower-case fragments; the page places them in its own sentences.
const NO_CONNECTION: &str = "no connection to the update server";
const NO_ANSWER: &str = "the update server didn't answer";
const BAD_SIGNATURE: &str = "the download failed its security check, so it was not installed";
const BLOCKED: &str = "Windows blocked the installer";
const SNAPSHOT_FAILED: &str = "Tiffin couldn't back up your library first, so nothing was changed";
const UP_TO_DATE: &str = "Tiffin is already up to date";
const UNSAVED: &str = "an edit is still open, so nothing was changed";

#[derive(Serialize)]
pub(crate) struct UpdateInfo {
    version: String,
    short: String,
    notes: String,
}

/// "0.3.0" is shown as "0.3"; only a zero patch is dropped.
fn short_version(v: &str) -> String {
    v.strip_suffix(".0").filter(|rest| rest.matches('.').count() == 1).unwrap_or(v).to_string()
}

/// A plugin error shaped for the banner: no file paths, at most 160 characters.
fn trimmed(msg: &str) -> String {
    let words: Vec<&str> = msg.split_whitespace().filter(|w| !(w.contains(":\\") || w.starts_with("\\\\") || w.starts_with('/'))).collect();
    let cut: String = words.join(" ").chars().take(160).collect();
    format!("something went wrong ({cut})")
}

fn reason(e: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    match e {
        E::Minisign(_) | E::Base64(_) | E::SignatureUtf8(_) | E::SignedVersionMismatch { .. } | E::MissingSignedVersion => BAD_SIGNATURE.into(),
        E::Reqwest(r) if r.is_timeout() => NO_ANSWER.into(),
        E::Reqwest(r) if r.is_connect() => NO_CONNECTION.into(),
        E::Network(_) => NO_CONNECTION.into(),
        E::ReleaseNotFound => NO_ANSWER.into(),
        E::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => BLOCKED.into(),
        other => trimmed(&other.to_string()),
    }
}

fn updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    app.updater_builder().timeout(std::time::Duration::from_secs(10)).build().map_err(|e| reason(&e))
}

/// A newer signed release, if GitHub has one. Runs in Rust so the page needs no updater permission.
#[tauri::command]
pub(crate) async fn check_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let found = updater(&app)?.check().await.map_err(|e| reason(&e))?;
    Ok(found.map(|u| UpdateInfo { short: short_version(&u.version), version: u.version, notes: u.body.unwrap_or_default() }))
}

/// Page-only drafts the install would lose; not library data, so no `mutate` and nothing is saved.
#[tauri::command]
pub(crate) fn set_unsaved(state: tauri::State<Shared>, unsaved: bool) -> Result<(), String> {
    state.lock().unwrap().unsaved = unsaved;
    Ok(())
}

/// The last gate before the installer: no open edit, and a real snapshot or no install.
fn snapshot_first(state: &Shared, version: &str) -> Result<(), String> {
    let mut guard = state.lock().unwrap();
    let st = &mut *guard;
    // drafts live only in the page; the install restarts the app and would lose them
    if st.unsaved {
        return Err(UNSAVED.into());
    }
    // no snapshot, no install (the rule itself is the crate's)
    if !st.store.can_snapshot() {
        return Err(SNAPSHOT_FAILED.into());
    }
    st.store.snapshot(&st.lib, &format!("before update to {version}")).map_err(|_| SNAPSHOT_FAILED.to_string())
}

/// The gate runs after the download, right before the installer, so a failed download leaves no snapshot.
fn after_download<B>(state: &Shared, version: &str, downloaded: Result<B, String>, install: impl FnOnce(B) -> Result<(), String>) -> Result<(), String> {
    let bytes = downloaded?;
    snapshot_first(state, version)?;
    install(bytes)
}

/// Check, download and verify (minisign, against the key compiled into the app), then the gate (no open edit, a real
/// snapshot), then the installer. The gate sits after the download so a draft opened meanwhile still blocks the install.
/// On Windows the process exits inside `Update::install` and the installer relaunches Tiffin.
#[tauri::command]
pub(crate) async fn install_update(app: tauri::AppHandle, state: tauri::State<'_, Shared>) -> Result<(), String> {
    let Some(update) = updater(&app)?.check().await.map_err(|e| reason(&e))? else { return Err(UP_TO_DATE.into()) };
    let downloaded = update.download(|_, _| {}, || {}).await.map_err(|e| reason(&e));
    after_download(&state, &update.version, downloaded, |bytes| update.install(bytes).map_err(|e| reason(&e)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn tdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("rb-desktop-update-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn short_version_drops_a_zero_patch() {
        assert_eq!(short_version("0.3.0"), "0.3");
        assert_eq!(short_version("0.3.1"), "0.3.1");
        assert_eq!(short_version("1.0.0"), "1.0");
    }

    #[test]
    fn reasons_are_short_and_have_no_paths() {
        let long = format!("failed to open C:\\Users\\x\\AppData\\file.exe {}", "word ".repeat(80));
        let out = trimmed(&long);
        assert!(out.starts_with("something went wrong (") && out.ends_with(')'));
        assert!(out.chars().count() <= 160 + "something went wrong ()".len());
        assert!(!out.contains("C:\\") && !out.contains("file.exe"));
        assert!(!trimmed("see /etc/hosts now").contains("/etc"));
    }

    // snapshots are named by the second, so count by reason instead of by file
    fn updates_taken(shared: &Shared) -> usize {
        shared.lock().unwrap().store.snapshots().iter().filter(|s| s.reason == "before update to 9.9.9").count()
    }

    #[test]
    fn no_snapshot_no_install() {
        // normal library: one snapshot, named for the version
        let shared = Mutex::new(crate::load_state(tdir("ok")));
        assert_eq!(snapshot_first(&shared, "9.9.9"), Ok(()));
        assert_eq!(updates_taken(&shared), 1);
        // an open edit: refused before any snapshot work
        let shared = Mutex::new(crate::load_state(tdir("unsaved")));
        shared.lock().unwrap().unsaved = true;
        assert_eq!(snapshot_first(&shared, "9.9.9"), Err(UNSAVED.to_string()));
        assert_eq!(updates_taken(&shared), 0);
        // damaged library
        let d = tdir("damaged");
        drop(crate::load_state(d.clone()));
        std::fs::write(d.join("library.json"), "not json").unwrap();
        let _ = std::fs::remove_file(d.join("library.json.bak"));
        let shared = Mutex::new(crate::load_state(d));
        assert_eq!(snapshot_first(&shared, "9.9.9"), Err(SNAPSHOT_FAILED.to_string()));
        assert_eq!(updates_taken(&shared), 0);
        // read-only store
        let shared = Mutex::new(crate::load_state(tdir("readonly")));
        shared.lock().unwrap().store.read_only = true;
        assert_eq!(snapshot_first(&shared, "9.9.9"), Err(SNAPSHOT_FAILED.to_string()));
        assert_eq!(updates_taken(&shared), 0);
    }

    #[test]
    fn gate_runs_after_the_download() {
        use std::cell::Cell;
        // failed download: its own text, no install, no snapshot
        let shared = Mutex::new(crate::load_state(tdir("dl-fail")));
        let ran = Cell::new(false);
        let r = after_download(&shared, "9.9.9", Err::<Vec<u8>, _>(NO_CONNECTION.to_string()), |_| {
            ran.set(true);
            Ok(())
        });
        assert_eq!(r, Err(NO_CONNECTION.to_string()));
        assert!(!ran.get());
        assert_eq!(updates_taken(&shared), 0);
        // a draft opened during the download: refused, no install, no snapshot
        let shared = Mutex::new(crate::load_state(tdir("dl-draft")));
        shared.lock().unwrap().unsaved = true;
        let r = after_download(&shared, "9.9.9", Ok(vec![1u8]), |_| {
            ran.set(true);
            Ok(())
        });
        assert_eq!(r, Err(UNSAVED.to_string()));
        assert!(!ran.get());
        assert_eq!(updates_taken(&shared), 0);
        // normal: the snapshot exists before the installer runs, and gets the downloaded bytes
        let shared = Mutex::new(crate::load_state(tdir("dl-ok")));
        let r = after_download(&shared, "9.9.9", Ok(vec![1u8, 2]), |bytes| {
            assert_eq!(bytes, vec![1u8, 2]);
            assert_eq!(updates_taken(&shared), 1);
            ran.set(true);
            Ok(())
        });
        assert_eq!(r, Ok(()));
        assert!(ran.get());
        // an installer error comes back unchanged
        let shared = Mutex::new(crate::load_state(tdir("dl-inst")));
        let r = after_download(&shared, "9.9.9", Ok(()), |_| Err(BLOCKED.to_string()));
        assert_eq!(r, Err(BLOCKED.to_string()));
    }
}
