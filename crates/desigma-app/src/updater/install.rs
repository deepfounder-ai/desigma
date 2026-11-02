//! Putting a downloaded update in place. Three steps, each a plain function so they run (and are tested) on
//! every platform; only the process-launching details differ per system:
//!
//! 1. [`detect`]: how this copy was installed, or why it cannot update itself.
//! 2. [`prepare`]: unpack the verified download next to what it replaces. Nothing is replaced yet.
//! 3. [`commit`]: once the user chose to restart, swap the files (or start the helper that does), returning
//!    what to launch when the window has closed ([`Launch`]).
//!
//! | Install | Prepare | Commit |
//! |---|---|---|
//! | Windows, Inno setup (`unins000.exe` beside the exe) | keep `setup.exe` in the temp dir | launch it silently with `/RELAUNCH=1` |
//! | Windows, portable zip | unzip `desigma.exe` to `desigma.new.exe` beside it | rename the running exe to `desigma.old.exe`, rename the new one in, launch it; the next start deletes `.old.exe` |
//! | macOS `.app` | untar to `.desigma-update/` beside the bundle | start a shell script that waits for our pid, swaps the bundles, clears quarantine and opens the app |
//! | Linux tarball | copy the binaries to hidden `.name.update` files beside the originals | rename them over the originals (atomic, fine for a running binary), refresh `install.sh` resources, launch |

use super::assets::{Kind, Os};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// An installed copy that can update itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub kind: Kind,
    pub exe: PathBuf,
}

/// What [`prepare`] produced: everything [`commit`] needs, already on disk.
#[derive(Clone, Debug, PartialEq)]
pub enum Prepared {
    /// Windows: the verified `setup.exe`.
    Setup { setup: PathBuf },
    /// Windows portable and Linux: (current file, new file) pairs, the main binary last.
    Swap {
        pairs: Vec<(PathBuf, PathBuf)>,
        /// Keep the replaced file as `<name>.old.exe` (Windows cannot delete a running exe, but can rename it).
        aside: bool,
        /// What to start afterwards.
        relaunch: PathBuf,
        /// Linux: the new `install.sh` and the prefix `install.sh` put desigma under, to refresh icons and so on.
        resources: Option<(PathBuf, PathBuf)>,
    },
    /// macOS: the unpacked new bundle, the bundle it replaces and the staging directory around the new one.
    Bundle { new_app: PathBuf, app: PathBuf, stage: PathBuf },
}

/// Why this copy of desigma cannot update itself (or how), from where it is running.
pub fn detect() -> Result<Target, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the running program ({e})"))?;
    if Os::current() != Os::Mac && !cfg!(target_arch = "x86_64") {
        return Err("releases have no build for this processor".into());
    }
    detect_at(Os::current(), &exe, std::env::var_os("APPIMAGE").is_some())
}

/// [`detect`] for a given system and executable path.
pub fn detect_at(os: Os, exe: &Path, appimage: bool) -> Result<Target, String> {
    let dir = exe.parent().ok_or("the program has no folder")?;
    let kind = match os {
        // Inno Setup puts its uninstaller beside the app
        Os::Windows if dir.join("unins000.exe").is_file() => Kind::WindowsInstaller,
        Os::Windows => Kind::WindowsPortable,
        Os::Mac => {
            app_bundle(exe).ok_or("desigma is not running from an app bundle")?;
            Kind::MacApp
        }
        Os::Linux => {
            if appimage {
                return Err("an AppImage is replaced as a whole file; download the new one".into());
            }
            if let Some(root) = MANAGED.iter().find(|root| exe.starts_with(root)) {
                return Err(format!("desigma is installed in {root}, which your package manager owns"));
            }
            Kind::LinuxTarball
        }
    };
    Ok(Target { kind, exe: exe.to_path_buf() })
}

/// Linux folders that package managers (and sandboxes) own: updating there would fight them.
const MANAGED: [&str; 9] = ["/usr/bin", "/bin", "/usr/sbin", "/usr/lib", "/usr/libexec", "/opt", "/snap", "/app", "/var/lib/flatpak"];

/// `…/desigma.app` for `…/desigma.app/Contents/MacOS/desigma`.
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app").then(|| app.to_path_buf())
}

/// Downloads and unpacked archives live here until the update is installed or the next launch cleans up.
pub fn work_dir() -> PathBuf {
    std::env::temp_dir().join("desigma-update")
}

fn not_writable(dir: &Path, e: &std::io::Error) -> String {
    format!("desigma can't write to {} ({e}), so it can't replace itself there", dir.display())
}

/// Unpacks the verified download. `archive` is the file `kind` names; the result is ready for [`commit`].
pub fn prepare(target: &Target, archive: &Path, work: &Path) -> Result<Prepared, String> {
    let dir = target.exe.parent().ok_or("the program has no folder")?;
    match target.kind {
        Kind::WindowsInstaller => Ok(Prepared::Setup { setup: archive.to_path_buf() }),
        Kind::WindowsPortable => prepare_portable(&target.exe, dir, archive),
        Kind::MacApp => prepare_bundle(&app_bundle(&target.exe).ok_or("desigma is not running from an app bundle")?, archive),
        Kind::LinuxTarball => prepare_linux(&target.exe, dir, archive, work),
    }
}

/// The portable zip holds `desigma-<ver>-<triple>/desigma.exe` (and `desigma-diff.exe`). Only the file names
/// matter: nothing is written to a path from the archive.
fn prepare_portable(exe: &Path, dir: &Path, archive: &Path) -> Result<Prepared, String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("cannot open the download ({e})"))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| format!("the download is not a zip file ({e})"))?;
    let main = exe.file_name().ok_or("the program has no name")?.to_string_lossy().into_owned();
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    // desigma-diff.exe is replaced only when it sits beside the exe. desigma-mcp.exe is added when
    // the archive has it. The main binary goes last.
    for name in ["desigma-mcp.exe", "desigma-diff.exe", "desigma.exe"] {
        if name == "desigma-diff.exe" && !dir.join(name).is_file() {
            continue;
        }
        let new = dir.join(name.replace(".exe", ".new.exe"));
        if let Err(e) = extract_by_name(&mut zip, name, &new, b"MZ") {
            if name == "desigma-mcp.exe" && e.contains("has no") {
                continue;
            }
            for (_, staged) in &pairs {
                let _ = std::fs::remove_file(staged);
            }
            return Err(e);
        }
        pairs.push((dir.join(if name == "desigma.exe" { main.as_str() } else { name }), new));
    }
    Ok(Prepared::Swap { pairs, aside: true, relaunch: exe.to_path_buf(), resources: None })
}

/// Writes the zip entry whose file name is `name` to `dest`; it must be non-empty and start with `magic`.
fn extract_by_name<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str, dest: &Path, magic: &[u8]) -> Result<(), String> {
    let index = (0..zip.len())
        .find(|&i| zip.by_index_raw(i).is_ok_and(|f| f.is_file() && f.name().is_ok_and(|n| n.rsplit(['/', '\\']).next() == Some(name))))
        .ok_or_else(|| format!("the download has no {name}"))?;
    let entry = zip.by_index(index).map_err(|e| format!("cannot read {name} from the download ({e})"))?;
    let dir = dest.parent().unwrap_or(Path::new("."));
    let mut out = std::fs::File::create(dest).map_err(|e| not_writable(dir, &e))?;
    let copied = std::io::copy(&mut entry.take(super::download::MAX_DOWNLOAD), &mut out);
    drop(out);
    let checked = match copied {
        Err(e) => Err(format!("cannot write {} ({e})", dest.display())),
        Ok(0) => Err(format!("{name} in the download is empty")),
        Ok(_) if !starts_with(dest, magic) => Err(format!("{name} in the download is not a program for this system")),
        Ok(_) => Ok(()),
    };
    if checked.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    checked
}

fn starts_with(path: &Path, magic: &[u8]) -> bool {
    let mut head = vec![0u8; magic.len()];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).is_ok() && head == magic
}

/// `tar` for a gzipped archive. Windows ships bsdtar in System32 (Git's GNU tar would read `C:` as a host name).
fn untar(archive: &Path, dest: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32").join("tar.exe")
    } else {
        PathBuf::from("tar")
    };
    let out = Command::new(program).arg("-xzf").arg(archive).arg("-C").arg(dest).stdin(Stdio::null()).output().map_err(|e| format!("cannot run tar ({e})"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("cannot unpack the download ({})", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// `name` at the top of `root` or one folder down (the tarballs wrap everything in `desigma-<ver>-<target>/`).
fn find_in(root: &Path, name: &str) -> Option<PathBuf> {
    let direct = root.join(name);
    if direct.exists() {
        return Some(direct);
    }
    std::fs::read_dir(root).ok()?.filter_map(Result::ok).map(|e| e.path().join(name)).find(|p| p.exists())
}

/// macOS: the tarball holds `desigma-<ver>-macos-universal/desigma.app`. It is unpacked beside the bundle so
/// the final move is a rename on one volume.
fn prepare_bundle(app: &Path, archive: &Path) -> Result<Prepared, String> {
    let parent = app.parent().ok_or("the app has no folder")?;
    let stage = parent.join(".desigma-update");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir(&stage)
        .map_err(|e| format!("{}. Move desigma to a folder you own, or install the update from the release page", not_writable(parent, &e)))?;
    let found = untar(archive, &stage).and_then(|()| {
        let new_app = find_in(&stage, "desigma.app").ok_or("the download has no desigma.app")?;
        if !new_app.join("Contents/MacOS/desigma").is_file() {
            return Err("the desigma.app in the download is incomplete".to_string());
        }
        Ok(new_app)
    });
    match found {
        Ok(new_app) => Ok(Prepared::Bundle { new_app, app: app.to_path_buf(), stage }),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            Err(e)
        }
    }
}

/// Linux: copies `desigma` (and `desigma-diff` when it sits beside it) to hidden `.<name>.update` files in the
/// same folder, so [`commit`] can rename them over the originals.
fn prepare_linux(exe: &Path, dir: &Path, archive: &Path, work: &Path) -> Result<Prepared, String> {
    let extract = work.join("extract");
    let _ = std::fs::remove_dir_all(&extract);
    std::fs::create_dir_all(&extract).map_err(|e| format!("cannot create {} ({e})", extract.display()))?;
    untar(archive, &extract)?;
    let main = find_in(&extract, "desigma").filter(|p| p.is_file()).ok_or("the download has no desigma program")?;
    let root = main.parent().ok_or("the download is malformed")?.to_path_buf();
    let mut sources = vec![];
    if dir.join("desigma-diff").is_file() && root.join("desigma-diff").is_file() {
        sources.push((root.join("desigma-diff"), dir.join("desigma-diff")));
    }
    if root.join("desigma-mcp").is_file() {
        sources.push((root.join("desigma-mcp"), dir.join("desigma-mcp")));
    }
    sources.push((main, exe.to_path_buf()));
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (source, current) in sources {
        let staged = dir.join(format!(".{}.update", current.file_name().ok_or("the program has no name")?.to_string_lossy()));
        let copied = starts_with(&source, b"\x7fELF")
            .then_some(())
            .ok_or_else(|| "the download is not a program for this system".to_string())
            .and_then(|()| std::fs::copy(&source, &staged).map_err(|e| not_writable(dir, &e)))
            .and_then(|_| make_executable(&staged).map_err(|e| format!("cannot make the update executable ({e})")));
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&staged);
            for (_, s) in &pairs {
                let _ = std::fs::remove_file(s);
            }
            return Err(e);
        }
        pairs.push((current, staged));
    }
    // installed by install.sh: <prefix>/bin/desigma next to <prefix>/share/applications/desigma.desktop
    let script = root.join("install.sh");
    let prefix = dir.parent().filter(|_| dir.file_name().is_some_and(|n| n == "bin")).filter(|p| p.join("share/applications/desigma.desktop").is_file());
    let resources = prefix.filter(|_| script.is_file()).map(|p| (script, p.to_path_buf()));
    Ok(Prepared::Swap { pairs, aside: false, relaunch: exe.to_path_buf(), resources })
}

/// What to start once the window has closed.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    Nothing,
    Spawn { program: PathBuf, args: Vec<String> },
}

/// Setup's switches for an update: no wizard pages, no message boxes, no reboot, close desigma if it is still
/// running, and (our own parameter, read by `scripts/windows/desigma.iss`) start desigma again at the end.
pub const SETUP_ARGS: [&str; 5] = ["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/RELAUNCH=1"];

/// The macOS helper: waits for desigma to exit, swaps the bundles (putting the old one back if that fails),
/// clears the quarantine flag and opens the app. Arguments: pid, new bundle, bundle to replace, staging folder.
/// Paths come in as arguments, never spliced into the script.
pub const MAC_SCRIPT: &str = r#"trap '' HUP
pid=$1; new=$2; app=$3; stage=$4
n=0
while kill -0 "$pid" 2>/dev/null; do
  n=$((n + 1))
  [ "$n" -gt 300 ] && exit 1
  sleep 0.2
done
old="$app.old.$pid"
rm -rf "$old"
if mv "$app" "$old"; then
  if mv "$new" "$app"; then
    rm -rf "$old"
  else
    mv "$old" "$app"
  fi
fi
rm -rf "$stage"
xattr -dr com.apple.quarantine "$app" 2>/dev/null
open "$app"
"#;

/// Makes the change that cannot be taken back once the window closes, or fails with the files untouched.
/// The caller closes the window only on `Ok` and runs the returned [`Launch`] afterwards.
pub fn commit(prepared: &Prepared) -> Result<Launch, String> {
    match prepared {
        Prepared::Setup { setup } => {
            if !setup.is_file() {
                return Err("the downloaded installer is gone".into());
            }
            Ok(Launch::Spawn { program: setup.clone(), args: SETUP_ARGS.map(String::from).to_vec() })
        }
        Prepared::Swap { pairs, aside, relaunch, resources } => {
            swap_files(pairs, *aside)?;
            if let Some((script, prefix)) = resources {
                // best effort: the program itself is already replaced
                let _ = Command::new("sh")
                    .arg(script)
                    .env("PREFIX", prefix)
                    .env("DESIGMA_RESOURCES_ONLY", "1")
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            Ok(Launch::Spawn { program: relaunch.clone(), args: vec![] })
        }
        Prepared::Bundle { new_app, app, stage } => {
            let mut cmd = Command::new("/bin/sh");
            cmd.args(["-c", MAC_SCRIPT, "desigma-update"]).arg(std::process::id().to_string()).arg(new_app).arg(app).arg(stage);
            spawn_detached(&mut cmd).map_err(|e| format!("cannot start the updater ({e})"))?;
            Ok(Launch::Nothing)
        }
    }
}

/// `<name>.old.exe` for `<name>.exe`: the running program's old file, renamed rather than deleted.
pub fn old_name(target: &Path) -> PathBuf {
    target.with_extension("old.exe")
}

/// Puts each new file in place of its current one. With `aside` the current file is first renamed to
/// [`old_name`] (a running exe can be renamed, not overwritten); otherwise the new file is renamed over it,
/// which is atomic and fine for a running binary on Unix. If a step fails, the pairs already swapped are put
/// back (`aside`) and the error names the file.
pub fn swap_files(pairs: &[(PathBuf, PathBuf)], aside: bool) -> Result<(), String> {
    let fail = |what: &str, path: &Path, e: std::io::Error| format!("cannot {what} {} ({e})", path.display());
    if !aside {
        for (target, new) in pairs {
            std::fs::rename(new, target).map_err(|e| fail("replace", target, e))?;
        }
        return Ok(());
    }
    let mut done: Vec<(&Path, PathBuf)> = Vec::new();
    let undo = |done: &[(&Path, PathBuf)]| {
        for (target, old) in done.iter().rev() {
            let _ = std::fs::remove_file(target);
            let _ = std::fs::rename(old, target);
        }
    };
    for (target, new) in pairs {
        let old = old_name(target);
        if old.exists() {
            if let Err(e) = std::fs::remove_file(&old) {
                undo(&done);
                return Err(fail("remove the previous", &old, e));
            }
        }
        if let Err(e) = std::fs::rename(target, &old) {
            undo(&done);
            return Err(fail("move aside", target, e));
        }
        if let Err(e) = std::fs::rename(new, target) {
            let _ = std::fs::rename(&old, target);
            undo(&done);
            return Err(fail("replace", target, e));
        }
        done.push((target, old));
    }
    Ok(())
}

impl Launch {
    /// Starts the program, detached from this process.
    pub fn run(self) {
        if let Launch::Spawn { program, args } = self {
            let mut cmd = Command::new(&program);
            cmd.args(args);
            if let Err(e) = spawn_detached(&mut cmd) {
                eprintln!("desigma: cannot start {}: {e}", program.display());
            }
        }
    }
}

/// Set when the user restarts to update: [`Launch::run`] happens once the window has closed.
static STASHED: Mutex<Option<Launch>> = Mutex::new(None);

pub fn stash(launch: Launch) {
    *STASHED.lock().unwrap_or_else(|e| e.into_inner()) = Some(launch);
}

/// Runs what [`stash`] kept, once. Called from the close handling and again when `main` gets control back.
pub fn run_stashed() {
    let launch = STASHED.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(launch) = launch {
        launch.run();
    }
}

#[cfg(test)]
pub fn take_stashed() -> Option<Launch> {
    STASHED.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// A child that survives this process: its own process group (Unix) or no console tie (Windows).
fn spawn_detached(cmd: &mut Command) -> std::io::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
        cmd.creation_flags(0x0000_0008 | 0x0000_0200);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(drop)
}

fn make_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Files and folders an update leaves behind, to remove at the next start: the renamed old exe, staged but
/// never installed files and the download folder.
pub fn leftovers(os: Os, exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else { return vec![] };
    let mut out: Vec<PathBuf> = match os {
        Os::Windows => ["desigma.old.exe", "desigma-diff.old.exe", "desigma-mcp.old.exe", "desigma.new.exe", "desigma-diff.new.exe", "desigma-mcp.new.exe"]
            .iter()
            .map(|n| dir.join(n))
            .collect(),
        Os::Mac => app_bundle(exe).and_then(|app| Some(app.parent()?.join(".desigma-update"))).into_iter().collect(),
        Os::Linux => {
            let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            [name.as_str(), "desigma-diff", "desigma-mcp"].iter().map(|n| dir.join(format!(".{n}.update"))).collect()
        }
    };
    out.push(work_dir());
    out
}

/// Removes `paths` in the background, retrying for a while: right after an update the previous process or the
/// installer may still be holding them.
pub fn remove_later(paths: Vec<PathBuf>) {
    std::thread::spawn(move || {
        for _ in 0..40 {
            if paths.iter().all(|p| remove_path(p)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    });
}

/// Whether `path` is gone afterwards.
pub fn remove_path(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let _ = if path.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    !path.exists()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("desigma_install_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn the_install_type_comes_from_the_exe_path() {
        let dir = temp("detect");
        let exe = dir.join("desigma.exe");
        assert_eq!(detect_at(Os::Windows, &exe, false).unwrap().kind, Kind::WindowsPortable);
        write(&dir.join("unins000.exe"), "x");
        assert_eq!(detect_at(Os::Windows, &exe, false).unwrap().kind, Kind::WindowsInstaller);

        let mac = Path::new("/Applications/desigma.app/Contents/MacOS/desigma");
        assert_eq!(detect_at(Os::Mac, mac, false).unwrap().kind, Kind::MacApp);
        assert_eq!(app_bundle(mac).unwrap(), Path::new("/Applications/desigma.app"));
        assert!(detect_at(Os::Mac, Path::new("/work/target/release/desigma"), false).unwrap_err().contains("app bundle"));
        assert!(detect_at(Os::Mac, Path::new("/x/desigma.app/Contents/Resources/desigma"), false).is_err());

        for ok in ["/home/me/.local/bin/desigma", "/home/me/Downloads/desigma-1-x86_64-unknown-linux-gnu/desigma", "/usr/local/bin/desigma"] {
            assert_eq!(detect_at(Os::Linux, Path::new(ok), false).unwrap().kind, Kind::LinuxTarball, "{ok}");
        }
        for managed in ["/usr/bin/desigma", "/opt/desigma/desigma", "/snap/desigma/1/bin/desigma", "/app/bin/desigma", "/usr/lib/desigma/desigma"] {
            assert!(detect_at(Os::Linux, Path::new(managed), false).unwrap_err().contains("package manager"), "{managed}");
        }
        assert!(detect_at(Os::Linux, Path::new("/home/me/desigma.AppImage"), true).unwrap_err().contains("AppImage"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_portable_swap_renames_the_running_exe_aside() {
        let dir = temp("swap");
        let (exe, new) = (dir.join("desigma.exe"), dir.join("desigma.new.exe"));
        let (diff, diff_new) = (dir.join("desigma-diff.exe"), dir.join("desigma-diff.new.exe"));
        write(&exe, "old");
        write(&new, "new");
        write(&diff, "old diff");
        write(&diff_new, "new diff");
        // an .old.exe from the update before is replaced
        write(&dir.join("desigma.old.exe"), "ancient");
        swap_files(&[(diff.clone(), diff_new.clone()), (exe.clone(), new.clone())], true).unwrap();
        assert_eq!((read(&exe).as_str(), read(&diff).as_str()), ("new", "new diff"));
        assert_eq!((read(&dir.join("desigma.old.exe")).as_str(), read(&dir.join("desigma-diff.old.exe")).as_str()), ("old", "old diff"));
        assert!(!new.exists() && !diff_new.exists());
        // the next start deletes the old files and anything staged
        let left = leftovers(Os::Windows, &exe);
        assert!(left.contains(&dir.join("desigma.old.exe")) && left.contains(&dir.join("desigma-diff.old.exe")) && left.contains(&work_dir()));
        for p in left.iter().filter(|p| p.starts_with(&dir)) {
            assert!(remove_path(p));
        }
        let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["desigma-diff.exe", "desigma.exe"]);
        assert!(remove_path(&dir.join("not-there")), "nothing to remove counts as removed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_swap_puts_everything_back() {
        let dir = temp("rollback");
        let (exe, diff) = (dir.join("desigma.exe"), dir.join("desigma-diff.exe"));
        write(&exe, "old");
        write(&diff, "old diff");
        write(&dir.join("desigma-diff.new.exe"), "new diff");
        // the main binary's new file is missing, so the second pair fails after the first succeeded
        let e = swap_files(&[(diff.clone(), dir.join("desigma-diff.new.exe")), (exe.clone(), dir.join("desigma.new.exe"))], true).unwrap_err();
        assert!(e.starts_with("cannot replace") && e.contains("desigma.exe"), "{e}");
        assert_eq!((read(&exe).as_str(), read(&diff).as_str()), ("old", "old diff"), "both are as they were");
        assert!(!dir.join("desigma.old.exe").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unix_style_swap_renames_over() {
        let dir = temp("over");
        let (bin, staged) = (dir.join("desigma"), dir.join(".desigma.update"));
        write(&bin, "old");
        write(&staged, "new");
        swap_files(&[(bin.clone(), staged.clone())], false).unwrap();
        assert_eq!(read(&bin), "new");
        assert!(!staged.exists());
        assert!(swap_files(&[(bin.clone(), staged)], false).is_err(), "a missing new file is an error");
        assert_eq!(read(&bin), "new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn build_zip(path: &Path, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn the_portable_zip_is_unpacked_beside_the_exe() {
        let dir = temp("portable");
        let install = dir.join("app");
        std::fs::create_dir_all(&install).unwrap();
        let exe = install.join("desigma.exe");
        write(&exe, "MZ-old");
        let archive = dir.join("portable.zip");
        build_zip(
            &archive,
            &[
                ("desigma-9-x86_64-pc-windows-msvc/desigma.exe", b"MZ-new"),
                ("desigma-9-x86_64-pc-windows-msvc/desigma-diff.exe", b"MZ-diff"),
                ("desigma-9-x86_64-pc-windows-msvc/README.md", b"readme"),
            ],
        );
        let target = Target { kind: Kind::WindowsPortable, exe: exe.clone() };
        // no desigma-diff.exe beside the exe: only the editor is replaced
        let Prepared::Swap { pairs, aside, relaunch, resources } = prepare(&target, &archive, &dir).unwrap() else { panic!("expected a swap") };
        assert_eq!((pairs.clone(), aside, relaunch, resources), (vec![(exe.clone(), install.join("desigma.new.exe"))], true, exe.clone(), None));
        assert_eq!(read(&install.join("desigma.new.exe")), "MZ-new");
        assert_eq!(read(&exe), "MZ-old", "nothing is replaced before the restart");
        // with one, both are, the editor last
        write(&install.join("desigma-diff.exe"), "MZ-old-diff");
        let Prepared::Swap { pairs, .. } = prepare(&target, &archive, &dir).unwrap() else { panic!("expected a swap") };
        assert_eq!(pairs.iter().map(|(t, _)| t.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>(), ["desigma-diff.exe", "desigma.exe"]);
        let Launch::Spawn { program, args } = ({
            let prepared = Prepared::Swap { pairs, aside: true, relaunch: exe.clone(), resources: None };
            commit(&prepared).unwrap()
        }) else {
            panic!("expected a launch")
        };
        assert_eq!((program, args.is_empty()), (exe.clone(), true));
        assert_eq!((read(&exe).as_str(), read(&install.join("desigma.old.exe")).as_str()), ("MZ-new", "MZ-old"));
        assert_eq!(read(&install.join("desigma-diff.exe")), "MZ-diff");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_zips_are_refused_and_leave_nothing_behind() {
        let dir = temp("badzip");
        let exe = dir.join("desigma.exe");
        write(&exe, "MZ-old");
        let target = Target { kind: Kind::WindowsPortable, exe };
        let archive = dir.join("a.zip");
        write(&archive, "not a zip");
        assert!(prepare(&target, &archive, &dir).unwrap_err().contains("not a zip"));
        build_zip(&archive, &[("x/readme.txt", b"hi")]);
        assert_eq!(prepare(&target, &archive, &dir).unwrap_err(), "the download has no desigma.exe");
        build_zip(&archive, &[("x/desigma.exe", b"#!/bin/sh")]);
        assert!(prepare(&target, &archive, &dir).unwrap_err().contains("not a program"));
        build_zip(&archive, &[("x/desigma.exe", b"")]);
        assert!(prepare(&target, &archive, &dir).unwrap_err().contains("empty"));
        assert!(!dir.join("desigma.new.exe").exists(), "no half-written file");
        // a folder that cannot be written to: the exe's folder does not exist
        let ghost = Target { kind: Kind::WindowsPortable, exe: dir.join("gone").join("desigma.exe") };
        build_zip(&archive, &[("x/desigma.exe", b"MZ")]);
        assert!(prepare(&ghost, &archive, &dir).unwrap_err().contains("can't write to"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_installer_runs_silently_and_relaunches() {
        let dir = temp("setup");
        let setup = dir.join("desigma-9-x86_64-windows-setup.exe");
        let target = Target { kind: Kind::WindowsInstaller, exe: dir.join("desigma.exe") };
        write(&setup, "MZ");
        let prepared = prepare(&target, &setup, &dir).unwrap();
        assert_eq!(prepared, Prepared::Setup { setup: setup.clone() });
        let Launch::Spawn { program, args } = commit(&prepared).unwrap() else { panic!("expected a launch") };
        assert_eq!(program, setup);
        assert_eq!(args, ["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", "/RELAUNCH=1"]);
        std::fs::remove_file(&setup).unwrap();
        assert!(commit(&prepared).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn have_tar() -> bool {
        let program = if cfg!(windows) {
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_default()).join("System32").join("tar.exe")
        } else {
            PathBuf::from("tar")
        };
        Command::new(program).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    /// `files` (relative paths) under a folder called `top`, packed as `top.tar.gz` in `dir`.
    fn build_tarball(dir: &Path, top: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let src = dir.join("src");
        for (name, bytes) in files {
            let p = src.join(top).join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        }
        let archive = dir.join(format!("{top}.tar.gz"));
        let program = if cfg!(windows) {
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_default()).join("System32").join("tar.exe")
        } else {
            PathBuf::from("tar")
        };
        let status = Command::new(program).arg("-czf").arg(&archive).arg("-C").arg(&src).arg(top).status().unwrap();
        assert!(status.success());
        archive
    }

    #[test]
    fn the_mac_tarball_is_unpacked_beside_the_bundle() {
        if !have_tar() {
            return;
        }
        let dir = temp("mac");
        let apps = dir.join("Applications");
        let exe = apps.join("desigma.app/Contents/MacOS/desigma");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        write(&exe, "old");
        let archive = build_tarball(
            &dir,
            "desigma-9-macos-universal",
            &[("desigma.app/Contents/MacOS/desigma", b"new"), ("desigma.app/Contents/Info.plist", b"<plist/>"), ("README.md", b"r")],
        );
        let target = Target { kind: Kind::MacApp, exe: exe.clone() };
        let Prepared::Bundle { new_app, app, stage } = prepare(&target, &archive, &dir).unwrap() else { panic!("expected a bundle") };
        assert_eq!(app, apps.join("desigma.app"));
        assert_eq!(stage, apps.join(".desigma-update"));
        assert!(new_app.starts_with(&stage) && new_app.ends_with("desigma.app"), "{new_app:?}");
        assert_eq!(read(&new_app.join("Contents/MacOS/desigma")), "new");
        assert_eq!(read(&exe), "old", "the installed app is untouched until the restart");
        assert!(leftovers(Os::Mac, &exe).contains(&stage));
        // an archive without an app is refused and the staging folder removed
        let junk = build_tarball(&dir.join("junk"), "other", &[("readme", b"x")]);
        let e = prepare(&target, &junk, &dir).unwrap_err();
        assert!(e.contains("no desigma.app"), "{e}");
        assert!(!stage.exists());
        // a folder that cannot be written to
        let ghost = Target { kind: Kind::MacApp, exe: dir.join("nowhere/desigma.app/Contents/MacOS/desigma") };
        assert!(prepare(&ghost, &archive, &dir).unwrap_err().contains("can't write to"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_linux_tarball_is_staged_beside_the_binary() {
        if !have_tar() {
            return;
        }
        let dir = temp("linux");
        let prefix = dir.join(".local");
        let bin = prefix.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(prefix.join("share/applications")).unwrap();
        write(&prefix.join("share/applications/desigma.desktop"), "[Desktop Entry]");
        let exe = bin.join("desigma");
        write(&exe, "\x7fELF-old");
        write(&bin.join("desigma-diff"), "\x7fELF-old-diff");
        let archive = build_tarball(
            &dir,
            "desigma-9-x86_64-unknown-linux-gnu",
            &[("desigma", b"\x7fELF-new"), ("desigma-diff", b"\x7fELF-new-diff"), ("install.sh", b"#!/bin/sh\n"), ("icons/16.png", b"png")],
        );
        let work = dir.join("work");
        let target = Target { kind: Kind::LinuxTarball, exe: exe.clone() };
        let Prepared::Swap { pairs, aside, relaunch, resources } = prepare(&target, &archive, &work).unwrap() else { panic!("expected a swap") };
        assert_eq!(pairs, vec![(bin.join("desigma-diff"), bin.join(".desigma-diff.update")), (exe.clone(), bin.join(".desigma.update"))]);
        assert!(!aside && relaunch == exe);
        let (script, got_prefix) = resources.expect("an install.sh layout refreshes its resources");
        assert!(script.ends_with("install.sh") && got_prefix == prefix, "{script:?} {got_prefix:?}");
        assert_eq!(read(&exe), "\x7fELF-old", "nothing is replaced yet");
        assert_eq!(read(&bin.join(".desigma.update")), "\x7fELF-new");
        assert!(leftovers(Os::Linux, &exe).contains(&bin.join(".desigma.update")));
        // run in place from the unpacked folder (no share/ beside it) and without desigma-diff next to it
        let loose = dir.join("unpacked");
        std::fs::create_dir_all(&loose).unwrap();
        write(&loose.join("desigma"), "\x7fELF-old");
        let loose_target = Target { kind: Kind::LinuxTarball, exe: loose.join("desigma") };
        let Prepared::Swap { pairs, resources, .. } = prepare(&loose_target, &archive, &work).unwrap() else { panic!("expected a swap") };
        assert_eq!((pairs.len(), resources), (1, None));
        // a download that is not a Linux program is refused and leaves no staged file
        let wrong = build_tarball(&dir.join("wrong"), "desigma-9", &[("desigma", b"MZ")]);
        assert!(prepare(&loose_target, &wrong, &work).unwrap_err().contains("not a program"));
        assert!(!loose.join(".desigma.update").exists());
        // commit renames the staged files over the originals
        let target_prepared = prepare(&target, &archive, &work).unwrap();
        let Launch::Spawn { program, .. } = commit(&target_prepared).unwrap() else { panic!("expected a launch") };
        assert_eq!(program, exe);
        assert_eq!((read(&exe).as_str(), read(&bin.join("desigma-diff")).as_str()), ("\x7fELF-new", "\x7fELF-new-diff"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_mac_script_waits_swaps_and_opens() {
        // steps, in order, with the paths passed as arguments
        let order = [
            "kill -0 \"$pid\"",
            "mv \"$app\" \"$old\"",
            "mv \"$new\" \"$app\"",
            "mv \"$old\" \"$app\"",
            "xattr -dr com.apple.quarantine \"$app\"",
            "open \"$app\"",
        ];
        let mut at = 0;
        for step in order {
            at += MAC_SCRIPT[at..].find(step).unwrap_or_else(|| panic!("missing or out of order: {step}"));
        }
        assert!(MAC_SCRIPT.contains("pid=$1; new=$2; app=$3; stage=$4"));
        assert!(!MAC_SCRIPT.contains("{}"), "nothing is spliced into the script");
    }

    #[cfg(unix)]
    #[test]
    fn the_mac_script_replaces_a_bundle() {
        let dir = temp("script");
        let log = dir.join("log");
        // stand-ins for the two macOS tools, as shell functions so no file has to be executed
        let fakes = format!("xattr() {{ echo \"xattr $*\" >> '{0}'; }}\nopen() {{ echo \"open $*\" >> '{0}'; }}\n", log.display());
        let app = dir.join("desigma.app");
        let stage = dir.join(".desigma-update");
        let new = stage.join("desigma.app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        write(&app.join("v"), "old");
        write(&new.join("v"), "new");
        // a pid that is not running, so the script does not wait
        let mut gone = Command::new("true").spawn().unwrap();
        gone.wait().unwrap();
        let status = Command::new("sh")
            .args(["-c", &format!("{fakes}{MAC_SCRIPT}"), "desigma-update"])
            .arg(gone.id().to_string())
            .arg(&new)
            .arg(&app)
            .arg(&stage)
            .status()
            .unwrap();
        assert!(status.success());
        assert_eq!(read(&app.join("v")), "new");
        assert!(!stage.exists() && !dir.join(format!("desigma.app.old.{}", gone.id())).exists());
        let log = read(&log);
        assert!(log.contains("xattr -dr com.apple.quarantine") && log.contains(&format!("open {}", app.display())), "{log}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
