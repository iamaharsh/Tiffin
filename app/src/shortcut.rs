//! Cleaning up after the old Recipe Box app on Tiffin's first launch: its per-user install and its Desktop
//! and Start-menu shortcuts. Never touches %APPDATA%\RecipeBox.

use std::path::{Path, PathBuf};

pub const OLD_SHORTCUT_NAME: &str = if cfg!(windows) { "Recipe Box.lnk" } else { "recipe-box.desktop" };

/// Shown when the old Recipe Box app could not be removed on launch; the library is not affected.
pub const OLD_APP_NOT_REMOVED: &str = "Tiffin couldn't remove the old Recipe Box app. You can remove it in Settings > Apps. Your recipes are not affected.";

#[cfg(windows)]
const OLD_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Recipe Box";

/// Run a PowerShell script. Values (paths, names) travel only as environment variables, read in
/// the script as `$env:NAME`, never pasted into the script text, so no quote in a path can end a string.
#[cfg(windows)]
fn powershell(script: &str, env: &[(&str, &str)]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let script = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; {script}");
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .envs(env.iter().copied())
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("Couldn't run PowerShell: {}", e))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}').to_string())
}

/// The real desktop folder (follows OneDrive or other redirection).
pub fn desktop_dir() -> PathBuf {
    #[cfg(windows)]
    if let Ok(p) = powershell("[Environment]::GetFolderPath('Desktop')", &[]) {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Desktop")
}

/// Start menu folder for this user.
pub fn start_menu_dir() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("Microsoft").join("Windows").join("Start Menu").join("Programs"))
}

/// Where the old shortcut lives: (label, full path of the shortcut file). The spare copy inside the
/// data folder is deliberately not listed; that folder is never touched.
pub fn shortcut_places() -> Vec<(&'static str, PathBuf)> {
    let mut v = vec![("Desktop", desktop_dir().join(OLD_SHORTCUT_NAME))];
    if cfg!(windows) {
        if let Some(s) = start_menu_dir() {
            v.push(("Start menu", s.join(OLD_SHORTCUT_NAME)));
        }
    }
    v
}

/// True when a shortcut target is one of the old programs (`recipe_box.exe` or `recipe-box-desktop.exe`, any folder).
#[cfg_attr(not(windows), allow(dead_code))]
fn is_old_exe(target: &str) -> bool {
    target.rsplit(['\\', '/']).next().is_some_and(|n| n.eq_ignore_ascii_case("recipe_box.exe") || n.eq_ignore_ascii_case("recipe-box-desktop.exe"))
}

/// Delete the shortcut `lnk` when it launches an old program; any other shortcut is left exactly as it
/// is. Returns whether it deleted the shortcut.
pub fn remove_if_old(lnk: &Path) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let l = lnk.display().to_string();
        let target = powershell("$s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:RB_LNK); $s.TargetPath", &[("RB_LNK", &l)])?;
        if !is_old_exe(&target) {
            return Ok(false);
        }
        std::fs::remove_file(lnk).map_err(|e| format!("Can't remove {}: {}", lnk.display(), e))?;
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        let _ = lnk;
        Ok(false)
    }
}

/// Remove the Desktop and Start-menu "Recipe Box" shortcuts that launch an old program. Tiffin's own
/// installer writes Tiffin.lnk in both places, so nothing is created here. Returns the ones it removed.
pub fn remove_old_shortcuts() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return vec![];
    }
    shortcut_places().into_iter().map(|(_, p)| p).filter(|p| p.exists() && remove_if_old(p).unwrap_or(false)).collect()
}

/// The path out of a `reg query ... /v UninstallString` listing, without its quotes.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_uninstall_string(reg_output: &str) -> Option<String> {
    reg_output.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("UninstallString")?;
        let v = rest.trim_start().strip_prefix("REG_SZ")?.trim();
        Some(v.trim_matches('"').to_string())
    })
}

/// The folder of the old install, only when the registry value is a genuine old uninstaller: an absolute
/// path to `uninstall.exe` next to `recipe-box-desktop.exe`, not this program's folder and not inside the
/// data folder. Anything else (quotes, control characters, other files) is refused.
#[cfg_attr(not(windows), allow(dead_code))]
fn old_install_dir(uninstall: &str, current_exe: &Path, data_dir: &Path) -> Result<PathBuf, String> {
    let refuse = || OLD_APP_NOT_REMOVED.to_string();
    if uninstall.contains('"') || uninstall.chars().any(|c| c.is_control()) {
        return Err(refuse());
    }
    let p = Path::new(uninstall);
    if !p.is_absolute() || !p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("uninstall.exe")) {
        return Err(refuse());
    }
    let dir = p.parent().ok_or_else(refuse)?;
    let lower = |p: &Path| PathBuf::from(p.display().to_string().to_lowercase());
    let same_as_self = current_exe.parent().is_some_and(|c| lower(c) == lower(dir));
    if !dir.join("recipe-box-desktop.exe").is_file() || same_as_self || lower(dir).starts_with(lower(data_dir)) {
        return Err(refuse());
    }
    Ok(dir.to_path_buf())
}

/// The old install's UninstallString from `root` (HKCU or HKLM), if that key exists.
#[cfg(windows)]
fn uninstall_string(root: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg").args(["query", &format!("{root}\\{OLD_KEY}"), "/v", "UninstallString"]).creation_flags(0x0800_0000).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_uninstall_string(&String::from_utf8_lossy(&out.stdout))
}

/// Silently uninstall the old per-user Recipe Box app (D-02). Ok when it is gone or was never there; Err is
/// the sentence for the user. Only the leftover `uninstall.exe` and the then-empty folder are deleted, never
/// recursively, and the data folder is never touched. Does nothing on other systems.
pub fn remove_old_recipe_box() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let Some(value) = uninstall_string("HKCU") else {
            // a per-machine install needs admin rights Tiffin does not ask for
            return if uninstall_string("HKLM").is_some() { Err(OLD_APP_NOT_REMOVED.into()) } else { Ok(()) };
        };
        let exe = std::env::current_exe().map_err(|_| OLD_APP_NOT_REMOVED.to_string())?;
        let dir = old_install_dir(&value, &exe, &crate::storage::data_dir())?;
        // NSIS wants `_?=<folder>` last and unquoted (plan 02.1-07: with it, uninstall.exe blocks until done)
        let _ = std::process::Command::new(dir.join("uninstall.exe")).arg("/S").raw_arg(format!("_?={}", dir.display())).creation_flags(0x0800_0000).status();
        if uninstall_string("HKCU").is_some() {
            return Err(OLD_APP_NOT_REMOVED.into());
        }
        // the uninstaller leaves only itself behind; an error here just leaves the folder
        let _ = std::fs::remove_file(dir.join("uninstall.exe"));
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }
    #[cfg(not(windows))]
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn old_exe_is_recognised() {
        for yes in [r"C:\x\app\target\debug\recipe_box.exe", "C:/x/RECIPE_BOX.EXE", r"C:\x\RECIPE-BOX-DESKTOP.EXE"] {
            assert!(super::is_old_exe(yes), "{yes}");
        }
        for no in [r"C:\x\tiffin.exe", r"C:\Program Files\Other\app.exe", "other.exe", ""] {
            assert!(!super::is_old_exe(no), "{no}");
        }
    }

    #[test]
    fn parse_reads_the_uninstall_string() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Recipe Box\r\n    UninstallString    REG_SZ    \"C:\\Users\\a\\AppData\\Local\\Recipe Box\\uninstall.exe\"\r\n";
        assert_eq!(super::parse_uninstall_string(out).as_deref(), Some(r"C:\Users\a\AppData\Local\Recipe Box\uninstall.exe"));
        assert_eq!(super::parse_uninstall_string("ERROR: The system was unable to find the specified registry key or value."), None);
        assert_eq!(super::parse_uninstall_string(""), None);
    }

    #[test]
    fn only_a_real_old_install_folder_is_accepted() {
        let root = std::env::temp_dir().join(format!("rb old install {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (old, bare, data, mine) = (root.join("Recipe Box"), root.join("bare"), root.join("data"), root.join("Tiffin"));
        for d in [&old, &bare, &data, &mine] {
            std::fs::create_dir_all(d).unwrap();
        }
        for d in [&old, &data, &mine] {
            std::fs::write(d.join("recipe-box-desktop.exe"), b"").unwrap();
        }
        let un = |d: &PathBuf| d.join("uninstall.exe").display().to_string();
        let me = mine.join("tiffin.exe");
        assert_eq!(super::old_install_dir(&un(&old), &me, &data).unwrap(), old);
        let refused = [
            old.join("other.exe").display().to_string(),      // another file name
            un(&bare),                                        // no recipe-box-desktop.exe beside it
            un(&mine),                                        // the folder of the running program
            un(&data),                                        // inside the data folder
            "uninstall.exe".into(),                           // relative
            format!("{}\" /x", un(&old)),                     // embedded quote
            format!("{}\u{7}", un(&old)),                     // control character
        ];
        for r in refused {
            assert!(super::old_install_dir(&r, &me, &data).is_err(), "{r}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn powershell_values_never_become_script() {
        let value = "a b 'c' \u{2019}d\u{2019} $env:USERNAME; \"e\" `f` $(1+1) caf\u{e9}";
        assert_eq!(super::powershell("[Console]::Out.Write($env:RB_TEST)", &[("RB_TEST", value)]).unwrap(), value);
    }

    #[cfg(windows)]
    #[test]
    fn only_old_shortcuts_are_removed() {
        let d = std::env::temp_dir().join(format!("rb shortcut \u{2019}test {}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let make = |name: &str, target: &str| {
            let lnk = d.join(name);
            let l = lnk.display().to_string();
            super::powershell("$s=(New-Object -ComObject WScript.Shell).CreateShortcut($env:RB_LNK); $s.TargetPath=$env:RB_T; $s.Save()", &[("RB_LNK", &l), ("RB_T", target)]).unwrap();
            lnk
        };
        let old = make("old.lnk", r"C:\nowhere\recipe-box-desktop.exe");
        let other = make("other.lnk", r"C:\nowhere\other.exe");
        assert_eq!(super::remove_if_old(&old), Ok(true));
        assert_eq!(super::remove_if_old(&other), Ok(false));
        assert!(!old.exists());
        assert!(other.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
