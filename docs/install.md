# Installing desigma

Download the file for your platform from the
[latest release](https://github.com/deepfounder-ai/desigma/releases):

| Platform | File |
|---|---|
| macOS (Apple Silicon and Intel) | `desigma-<version>-macos.dmg` (one universal file for both) |
| Windows (x64), installer | `desigma-<version>-x86_64-windows-setup.exe` (recommended) |
| Windows (x64), portable | `desigma-<version>-x86_64-pc-windows-msvc.zip` |
| Linux (x64) | `desigma-<version>-x86_64-unknown-linux-gnu.tar.gz` |
| macOS, for scripts and CLI use | `desigma-<version>-macos-universal.tar.gz` |

The Windows installer needs no administrator rights. The macOS disk image holds `desigma.app` (the editor, with the `desigma-diff` tool inside). The
Windows and Linux archives hold the `desigma` editor, the `desigma-diff` tool, the licenses and the
README. `SHA256SUMS` on the release page lets you check the download:

```sh
sha256sum -c SHA256SUMS --ignore-missing      # Linux
shasum -a 256 -c SHA256SUMS --ignore-missing  # macOS
```

## The first-launch warning

desigma's release builds are **not code-signed yet**, so Windows and macOS warn
the first time you open them. The warning is about the missing publisher
signature, not about anything the app does: the source is public and every
release is built by GitHub Actions from a tagged commit.

### Windows

**Installer (recommended).**

1. Download `desigma-<version>-x86_64-windows-setup.exe` and double-click it.
2. SmartScreen shows **"Windows protected your PC"**. Click **More info**, then **Run anyway**.
3. Follow the wizard. By default desigma installs for your user only, into
   `%LOCALAPPDATA%\Programs\desigma`, with no administrator prompt. Choose "Install for all
   users" when the wizard asks to put it in `C:\Program Files\desigma` instead.
4. Tick **Create a desktop shortcut** and **Open .desigma files with desigma** if you want them,
   and leave **Launch desigma** ticked to start it right away.

Running a newer installer later upgrades the existing install in place. To uninstall, use
**Settings → Apps → Installed apps → desigma → Uninstall**. Your design files are never touched.
For a silent install, run the setup with `/VERYSILENT /CURRENTUSER`. (The in-app update uses
`/SILENT` and adds `/RELAUNCH=1`, which makes the installer start desigma again when it is done.)

**Portable zip.** Unzip the archive anywhere and double-click `desigma.exe` (it carries the
desigma icon and version info). The SmartScreen steps are the same. Delete the folder to remove it.

**If Windows Defender quarantines the download or the exe.** Unsigned builds are occasionally
flagged by mistake. Open **Windows Security → Virus & threat protection → Protection history**,
restore the item and choose **Allow on device**, then check the file against `SHA256SUMS`
(`certutil -hashfile <file> SHA256`) before running it.

### macOS

One `.dmg` works on both Apple Silicon and Intel Macs (macOS 11 or newer).

1. Download `desigma-<version>-macos.dmg` from the release page.
2. Double-click the `.dmg` to open it.
3. Drag **desigma** onto the **Applications** shortcut in that window.
4. Open **Applications** and double-click **desigma** (you can eject the disk image now).
5. The first time, macOS says it cannot verify that desigma is free of malware. Click **Done**.
6. Open **System Settings → Privacy & Security** and scroll down to the message that
   "desigma" was blocked. Click **Open Anyway**.
7. Confirm with your password or Touch ID, then click **Open** in the last dialog.

That is only needed once; afterwards desigma opens like any other app. The same steps are
in `Read me first.txt` inside the disk image.

If you prefer Terminal, this replaces steps 5 to 7:

```sh
xattr -dr com.apple.quarantine /Applications/desigma.app
```

The app is ad-hoc signed but not notarized by Apple yet, which is why the one-time confirmation
is needed.

**Command-line tool.** `desigma-diff` lives inside the app:

```sh
/Applications/desigma.app/Contents/MacOS/desigma-diff --help
```

**Opening files.** `.desigma` and `.fig` files list desigma under **Open With** in Finder.
Double-clicking a `.desigma` file opens it in the running editor (a file that replaces unsaved
work asks first, exactly like **File → Open**); running
`/Applications/desigma.app/Contents/MacOS/desigma path/to/file.desigma` does the same.

`desigma-<version>-macos-universal.tar.gz` holds the same `desigma.app` plus the licenses, for
scripting; unpack it and run `xattr -dr com.apple.quarantine desigma.app` once.

### Linux

No warning. Unpack, make sure the system libraries are installed (most desktops
have them already), and run it:

```sh
tar -xzf desigma-<version>-x86_64-unknown-linux-gnu.tar.gz
cd desigma-<version>-x86_64-unknown-linux-gnu
./desigma
```

To add desigma to your application menu with its icon, run the bundled installer. It needs no
sudo and installs under `~/.local` (binary, `.desktop` entry, icons and file types):

```sh
./install.sh
```

On a minimal system install the libraries the editor links against, for
example on Debian or Ubuntu:

```sh
sudo apt install libxkbcommon0 libwayland-client0 libx11-6 libxcursor1 libxrandr2 libxi6 libgl1 libfontconfig1
```

## GPU rendering

The canvas draws with the GPU (Vulkan, Metal, DirectX 12 or OpenGL) and falls
back to the CPU renderer when no suitable adapter is found. If you see drawing
problems, turn off **Preferences → Use GPU rendering**.

## Updates

A few seconds after launch, and then once a day while it runs, desigma asks GitHub's
release list (one request to `api.github.com`, with the user agent `desigma/<version>`)
whether a newer version is out. If there is one, a banner offers the release notes and
**Update now**. Nothing is downloaded before you press it. **Skip this version** stops the
reminders until the next release.

**Update now** downloads the file for your install from the release, with a progress bar and
a Cancel button, and checks its SHA-256 against the release's `SHA256SUMS` (a mismatch or a
missing entry stops the update and deletes the download). Then the dialog offers **Restart to
update**. If you are sharing or in a live session, desigma asks "Stop sharing?" / "Leave the
session?" first, and if the document has unsaved changes it asks Save / Discard / Cancel. After
that desigma closes, installs the new version and starts it again. Downloads only come from
this repository's release files on `github.com` (and GitHub's release file hosts it redirects
to); nothing else is fetched.

What happens per install:

| Install | What the update does |
|---|---|
| Windows installer | runs the new `setup.exe` silently (`/SILENT /SUPPRESSMSGBOXES /NORESTART /CLOSEAPPLICATIONS /RELAUNCH=1`) in the same per-user or all-users mode; the installer starts desigma again |
| Windows portable zip | unpacks the new `desigma.exe` (and `desigma-diff.exe` if it sits beside it) next to the old one, renames the running exe to `desigma.old.exe`, moves the new one in and starts it; the old file is deleted at the next start |
| macOS | unpacks the new `desigma.app` from `desigma-<version>-macos-universal.tar.gz` beside the old one; a small shell script waits for desigma to quit, swaps the bundles, clears the quarantine flag and opens the app |
| Linux tarball | renames the new `desigma` (and `desigma-diff`) over the old files, which is safe while the old one runs; if you installed with `install.sh`, the desktop entry, icons and file types are refreshed too |

When desigma cannot update itself it says why and keeps **Open release page** as a link, so you
can install the new version by hand as before. That is the case when the folder is not writable
by you (for example `/Applications` without administrator rights, or a copy that macOS runs
from a read-only location: move it to Applications first), for Linux packages that your
package manager owns (`/usr/bin`, `/opt`, snaps, Flatpak) and AppImages, and for builds from
source.

Turn the automatic check off with **Preferences → Check for updates automatically**, or set
`DESIGMA_NO_UPDATE_CHECK=1`. **Help → Check for updates…** still works, and Help also shows the
version you are running. Builds from source never check and never update themselves.

## Building from source

```sh
git clone https://github.com/deepfounder-ai/desigma
cd desigma
cargo run --release -p desigma-app
```

The pinned Rust toolchain installs automatically through `rust-toolchain.toml`.
See [CONTRIBUTING.md](../CONTRIBUTING.md) for the Linux build packages and the
test workflow.

## Signed builds

Signing is planned: Windows binaries through a code-signing certificate, the macOS
app signed with a Developer ID and notarized. Once that lands, these warnings go away and
this page will say so.
