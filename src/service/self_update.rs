//! Replaces the running macOS app bundle with a newer release.
//!
//! Two steps so nothing on disk changes until the user says so: `prepare` downloads the release
//! zip, checks its SHA-256 against the digest GitHub published for it, unpacks it and sanity-checks
//! the bundle; `apply_and_relaunch` swaps the bundle in place, then a detached shell waits for this
//! process to exit and reopens the app. The previous bundle is kept in the staging folder until the
//! new version starts and calls `cleanup_stale`, so a build that never launches can be restored by
//! hand.
//!
//! Everything lives in a hidden folder next to the app (same volume, so the swap is two atomic
//! renames); an app in a folder the user cannot write to simply is not installable.

use crate::service::tag_signature;
use crate::service::update_check::{Release, ReleaseAsset};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;

const STAGING_NAME: &str = ".DegenView-update";
const APP_NAME: &str = "DegenView.app";
const BUNDLE_ID: &str = "com.degenview.rust";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the relaunch helper waits for this process to exit before giving up.
const RELAUNCH_WAIT_TICKS: u32 = 300;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    Verifying,
    Downloading,
    Unpacking,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Verifying => "Checking the release signature…",
            Self::Downloading => "Downloading the new version…",
            Self::Unpacking => "Unpacking and checking it…",
        }
    }
}

/// A downloaded, verified update waiting for the user's restart.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prepared {
    app: PathBuf,
    staging: PathBuf,
    new_app: PathBuf,
}

/// The `.app` bundle this process runs from, when it can be replaced in place.
pub fn install_target() -> Result<PathBuf, String> {
    if !cfg!(target_os = "macos") {
        return Err("Automatic updates are only available on macOS.".to_owned());
    }
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|error| error.to_string())?;
    let app = bundle_for_executable(&exe)
        .ok_or_else(|| "DegenView is not running from an app bundle.".to_owned())?;
    if app
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation")
    {
        return Err("Move DegenView to the Applications folder first.".to_owned());
    }
    let parent = app.parent().ok_or("The app has no parent folder.")?;
    if !is_writable(parent) {
        return Err(format!("{} is not writable.", parent.display()));
    }
    Ok(app)
}

/// `/…/DegenView.app` for `/…/DegenView.app/Contents/MacOS/DegenView`.
fn bundle_for_executable(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app";
    is_bundle.then(|| app.to_path_buf())
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".DegenView-write-test-{}", std::process::id()));
    let ok = std::fs::create_dir(&probe).is_ok();
    let _ = std::fs::remove_dir(&probe);
    ok
}

/// Blocking: checks the release tag's signature, then downloads, verifies and unpacks the
/// release's zip next to `app`. Nothing is replaced yet, and nothing is downloaded for a tag that
/// is not signed by a pinned key.
pub fn prepare(
    app: &Path,
    release: &Release,
    progress: &dyn Fn(Stage),
) -> Result<Prepared, String> {
    let asset = release
        .asset
        .as_ref()
        .ok_or("This release has no download for this Mac.")?;
    progress(Stage::Verifying);
    tag_signature::verify_release_tag(&release.tag).map_err(|error| error.to_string())?;
    let parent = app.parent().ok_or("The app has no parent folder.")?;
    let staging = parent.join(STAGING_NAME);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .map_err(|error| format!("Could not create {STAGING_NAME}: {error}"))?;
    prepare_in(&staging, app, asset, progress).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&staging);
    })
}

fn prepare_in(
    staging: &Path,
    app: &Path,
    asset: &ReleaseAsset,
    progress: &dyn Fn(Stage),
) -> Result<Prepared, String> {
    progress(Stage::Downloading);
    let bytes = download(asset)?;
    verify(&bytes, asset)?;

    progress(Stage::Unpacking);
    let zip = staging.join("update.zip");
    std::fs::write(&zip, &bytes)
        .map_err(|error| format!("Could not save the download: {error}"))?;
    drop(bytes);
    let extracted = staging.join("new");
    let status = std::process::Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(&zip)
        .arg(&extracted)
        .output()
        .map_err(|error| format!("Could not unpack the download: {error}"))?;
    if !status.status.success() {
        return Err(format!(
            "Could not unpack the download: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    let _ = std::fs::remove_file(&zip);
    let new_app = extracted.join(APP_NAME);
    check_bundle(&new_app)?;
    Ok(Prepared {
        app: app.to_path_buf(),
        staging: staging.to_path_buf(),
        new_app,
    })
}

fn download(asset: &ReleaseAsset) -> Result<Vec<u8>, String> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .http_status_as_error(false)
        .build();
    let mut response = ureq::Agent::new_with_config(config)
        .get(&asset.url)
        .header(
            "User-Agent",
            concat!("DegenView/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|error| format!("Download failed: {error}"))?;
    let status = response.status().as_u16();
    if status != 200 {
        return Err(format!("Download failed: HTTP {status}."));
    }
    response
        .body_mut()
        .with_config()
        .limit(asset.size + 1)
        .read_to_vec()
        .map_err(|error| format!("Download failed: {error}"))
}

fn verify(bytes: &[u8], asset: &ReleaseAsset) -> Result<(), String> {
    if bytes.len() as u64 != asset.size {
        return Err("The download is incomplete.".to_owned());
    }
    let hash = Sha256::digest(bytes);
    let hex: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    if hex != asset.sha256 {
        return Err("The download does not match its published checksum.".to_owned());
    }
    Ok(())
}

/// The unpacked bundle must look like DegenView: its executable and our bundle identifier.
fn check_bundle(app: &Path) -> Result<(), String> {
    let executable = app.join("Contents/MacOS/DegenView");
    let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap_or_default();
    if !executable.is_file() || !plist.contains(BUNDLE_ID) {
        return Err("The download is not a DegenView app.".to_owned());
    }
    Ok(())
}

/// Swaps the prepared bundle in and schedules the reopen. On `Ok` the caller must quit now.
pub fn apply_and_relaunch(prepared: &Prepared) -> Result<(), String> {
    swap(prepared)?;
    if let Err(error) = spawn_relaunch(prepared) {
        // Without the helper nobody would reopen the app; the new bundle is in place regardless,
        // so tell the user rather than quitting into nothing.
        return Err(format!(
            "Updated, but could not restart: {error}. Reopen DegenView."
        ));
    }
    Ok(())
}

fn swap(prepared: &Prepared) -> Result<(), String> {
    let old = prepared.staging.join("old").join(APP_NAME);
    std::fs::create_dir_all(old.parent().expect("old has a parent"))
        .map_err(|error| error.to_string())?;
    std::fs::rename(&prepared.app, &old)
        .map_err(|error| format!("Could not move the current app aside: {error}"))?;
    if let Err(error) = std::fs::rename(&prepared.new_app, &prepared.app) {
        let _ = std::fs::rename(&old, &prepared.app);
        return Err(format!("Could not install the new app: {error}"));
    }
    Ok(())
}

/// Reopens the app once this process has exited. `HOME` and `DEGENVIEW_*` are passed on because
/// `open` otherwise starts the app with a clean environment.
#[cfg(unix)]
fn spawn_relaunch(prepared: &Prepared) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let script = format!(
        "pid=$1; shift; i=0; \
         while kill -0 \"$pid\" 2>/dev/null && [ $i -lt {RELAUNCH_WAIT_TICKS} ]; do sleep 0.2; i=$((i+1)); done; \
         exec \"$@\""
    );
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(std::process::id().to_string())
        .args(["/usr/bin/open", "-n"]);
    for (name, value) in std::env::vars_os() {
        let forwarded = name
            .to_str()
            .is_some_and(|name| name == "HOME" || name.starts_with("DEGENVIEW_"));
        if forwarded {
            let mut assignment = name;
            assignment.push("=");
            assignment.push(value);
            command.arg("--env").arg(assignment);
        }
    }
    command
        .arg(&prepared.app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
}

#[cfg(not(unix))]
fn spawn_relaunch(_: &Prepared) -> std::io::Result<()> {
    Err(std::io::Error::other("unsupported platform"))
}

/// Run once the app has started: removes the staging folder (and the previous bundle in it) a
/// finished or abandoned update left next to the app.
pub fn cleanup_stale() {
    let Ok(exe) = std::env::current_exe().and_then(|exe| exe.canonicalize()) else {
        return;
    };
    if let Some(parent) = bundle_for_executable(&exe)
        .as_deref()
        .and_then(Path::parent)
    {
        let _ = std::fs::remove_dir_all(parent.join(STAGING_NAME));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "degenview-self-update-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_app(root: &Path, marker: &str) -> PathBuf {
        let app = root.join(APP_NAME);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/MacOS/DegenView"), marker).unwrap();
        fs::write(
            app.join("Contents/Info.plist"),
            format!("<string>{BUNDLE_ID}</string>"),
        )
        .unwrap();
        app
    }

    #[test]
    fn the_bundle_is_found_from_its_executable() {
        assert_eq!(
            bundle_for_executable(Path::new(
                "/Applications/DegenView.app/Contents/MacOS/DegenView"
            )),
            Some(PathBuf::from("/Applications/DegenView.app"))
        );
        assert_eq!(
            bundle_for_executable(Path::new("/repo/target/release/degenview")),
            None
        );
        assert_eq!(
            bundle_for_executable(Path::new("/x/Tools/Contents/MacOS/degenview")),
            None
        );
    }

    #[test]
    fn a_checksum_mismatch_or_short_body_is_rejected() {
        let asset = |size, sha256: &str| ReleaseAsset {
            name: "a.zip".to_owned(),
            url: String::new(),
            size,
            sha256: sha256.to_owned(),
        };
        // sha256("abc")
        let abc = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify(b"abc", &asset(3, abc)).is_ok());
        assert!(verify(b"abd", &asset(3, abc)).is_err());
        assert!(verify(b"ab", &asset(3, abc)).is_err());
    }

    #[test]
    fn only_a_degenview_bundle_passes_the_check() {
        let root = scratch("check");
        assert!(check_bundle(&fake_app(&root, "x")).is_ok());
        fs::write(
            root.join(APP_NAME).join("Contents/Info.plist"),
            "<string>other</string>",
        )
        .unwrap();
        assert!(check_bundle(&root.join(APP_NAME)).is_err());
        assert!(check_bundle(&root.join("Missing.app")).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn the_swap_installs_the_new_bundle_and_keeps_the_old_one() {
        let root = scratch("swap");
        let app = fake_app(&root, "old");
        let staging = root.join(STAGING_NAME);
        let new_dir = staging.join("new");
        fs::create_dir_all(&new_dir).unwrap();
        let new_app = fake_app(&new_dir, "new");
        let prepared = Prepared {
            app: app.clone(),
            staging: staging.clone(),
            new_app,
        };
        swap(&prepared).unwrap();
        assert_eq!(
            fs::read_to_string(app.join("Contents/MacOS/DegenView")).unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(
                staging
                    .join("old")
                    .join(APP_NAME)
                    .join("Contents/MacOS/DegenView")
            )
            .unwrap(),
            "old"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_swap_puts_the_old_bundle_back() {
        let root = scratch("rollback");
        let app = fake_app(&root, "old");
        let staging = root.join(STAGING_NAME);
        fs::create_dir_all(&staging).unwrap();
        let prepared = Prepared {
            app: app.clone(),
            staging,
            new_app: root.join("missing.app"),
        };
        assert!(swap(&prepared).is_err());
        assert_eq!(
            fs::read_to_string(app.join("Contents/MacOS/DegenView")).unwrap(),
            "old"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Hits GitHub: `cargo test -- --ignored the_latest_release_downloads_and_unpacks`.
    #[test]
    #[ignore = "downloads the latest release"]
    #[cfg(target_os = "macos")]
    fn the_latest_release_downloads_and_unpacks() {
        use crate::service::update_check::{UpdateOutcome, check};
        let UpdateOutcome::Available(release) = check("0.0.1") else {
            panic!("no release found");
        };
        assert!(
            release.asset.is_some(),
            "a macOS asset for this architecture"
        );
        let root = scratch("live");
        let app = fake_app(&root, "old");
        let prepared = prepare(&app, &release, &|_| {}).unwrap();
        assert!(check_bundle(&prepared.new_app).is_ok());
        swap(&prepared).unwrap();
        assert_ne!(
            fs::read(app.join("Contents/MacOS/DegenView")).unwrap(),
            b"old"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_read_only_folder_is_not_writable() {
        let root = scratch("writable");
        assert!(is_writable(&root));
        assert!(!is_writable(&root.join("missing")));
        let _ = fs::remove_dir_all(root);
    }
}
