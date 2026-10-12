//! "Is there a newer DegenView?" — asks GitHub for the latest published release of
//! `degenview/degenview` and compares its tag with the running version. Read-only: it never
//! downloads or installs anything, it only reports where the release page is.

use crate::service::http::{self, Retry};
use serde::Deserialize;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/degenview/degenview/releases/latest";
/// Where "view release" falls back to when the API reply carries no page URL.
pub const RELEASES_URL: &str = "https://github.com/degenview/degenview/releases";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateOutcome {
    UpToDate,
    Available(Release),
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    /// The tag as published: "v0.2.0".
    pub tag: String,
    /// The tag without a leading `v`: "0.2.0".
    pub version: String,
    pub url: String,
    /// This platform's download, when the release has one `service::self_update` can install.
    pub asset: Option<ReleaseAsset>,
    /// Why the release is only offered for manual download, when it is not installable.
    pub manual_only: Option<&'static str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lowercase hex SHA-256, from the `digest` GitHub computes for every upload.
    pub sha256: String,
}

/// The largest download the updater will accept.
const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
/// Accounts GitHub may record as a release's author: the release workflow's bot, and the
/// maintainer for a release they published by hand.
const RELEASE_AUTHORS: &[&str] = &["github-actions[bot]", "nicoSWD"];
const DOWNLOAD_PREFIX: &str = "https://github.com/degenview/degenview/releases/download/";

#[derive(Deserialize)]
struct ReleaseDto {
    tag_name: String,
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    /// Set by GitHub once a release is published as immutable: its assets and tag can no longer
    /// change.
    #[serde(default)]
    immutable: bool,
    author: Option<AuthorDto>,
    #[serde(default)]
    assets: Vec<AssetDto>,
}

#[derive(Deserialize)]
struct AuthorDto {
    login: String,
}

#[derive(Deserialize)]
struct AssetDto {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// The release file name ends with this on the platform the updater can replace in place.
fn platform_suffix() -> Option<&'static str> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("-macos-arm64.zip")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("-macos-x64.zip")
    } else {
        None
    }
}

fn select_asset(assets: Vec<AssetDto>, suffix: Option<&str>) -> Option<ReleaseAsset> {
    let suffix = suffix?;
    let asset = assets
        .into_iter()
        .find(|asset| asset.name.ends_with(suffix))?;
    let sha256 = asset.digest?.strip_prefix("sha256:")?.to_ascii_lowercase();
    let valid = sha256.len() == 64 && sha256.bytes().all(|byte| byte.is_ascii_hexdigit());
    (valid
        && asset.browser_download_url.starts_with(DOWNLOAD_PREFIX)
        && (1..=MAX_ASSET_BYTES).contains(&asset.size))
    .then_some(ReleaseAsset {
        name: asset.name,
        url: asset.browser_download_url,
        size: asset.size,
        sha256,
    })
}

/// Headers for every GitHub API request the app makes.
pub fn github_headers() -> [(&'static str, &'static str); 3] {
    [
        ("Accept", "application/vnd.github+json"),
        (
            "User-Agent",
            concat!("DegenView/", env!("CARGO_PKG_VERSION")),
        ),
        ("X-GitHub-Api-Version", "2022-11-28"),
    ]
}

/// Blocking: one GET. Call it from a worker thread.
pub fn check(current: &str) -> UpdateOutcome {
    let body = http::get_text(
        LATEST_RELEASE_URL,
        &[
            ("Accept", "application/vnd.github+json"),
            (
                "User-Agent",
                concat!("DegenView/", env!("CARGO_PKG_VERSION")),
            ),
            ("X-GitHub-Api-Version", "2022-11-28"),
        ],
        Retry::Transient,
    );
    match body {
        Ok(body) => evaluate(&body, current),
        // No published release yet (or repository not public): nothing newer exists.
        Err(http::Error::Status(404)) => UpdateOutcome::UpToDate,
        Err(http::Error::Status(403 | 429)) => {
            UpdateOutcome::Failed("GitHub is rate limiting requests. Try again later.".to_owned())
        }
        Err(error) => UpdateOutcome::Failed(error.to_string()),
    }
}

fn evaluate(body: &str, current: &str) -> UpdateOutcome {
    let release: ReleaseDto = match serde_json::from_str(body) {
        Ok(release) => release,
        Err(_) => return UpdateOutcome::Failed("GitHub sent an unreadable reply.".to_owned()),
    };
    if release.draft || release.prerelease {
        return UpdateOutcome::UpToDate;
    }
    let (Some(latest), Some(running)) =
        (Version::parse(&release.tag_name), Version::parse(current))
    else {
        return UpdateOutcome::Failed(format!(
            "Could not compare version {current} with release {}.",
            release.tag_name
        ));
    };
    if latest > running {
        // Only install what cannot be swapped after publishing and that the maintainer or the release
        // workflow published; anything else is left to a manual download.
        let manual_only = if !release.immutable {
            Some("Not published as an immutable release, so it needs a manual download.")
        } else if !release.author.as_ref().is_some_and(|author| {
            RELEASE_AUTHORS
                .iter()
                .any(|login| login.eq_ignore_ascii_case(&author.login))
        }) {
            Some(
                "Not published by the maintainer or the release workflow, so it needs a manual download.",
            )
        } else {
            None
        };
        let assets = if manual_only.is_none() {
            release.assets
        } else {
            Vec::new()
        };
        UpdateOutcome::Available(Release {
            tag: release.tag_name.clone(),
            version: release
                .tag_name
                .trim()
                .trim_start_matches(['v', 'V'])
                .to_owned(),
            url: release
                .html_url
                .filter(|url| url.starts_with("https://github.com/"))
                .unwrap_or_else(|| RELEASES_URL.to_owned()),
            asset: select_asset(assets, platform_suffix()),
            manual_only,
        })
    } else {
        UpdateOutcome::UpToDate
    }
}

/// `major.minor.patch` with an optional `v` prefix and an optional `-prerelease`/`+build` tail.
/// A prerelease sorts before the same release (`0.2.0-rc1 < 0.2.0`).
#[derive(Clone, Debug, Eq, PartialEq)]
struct Version {
    core: [u64; 3],
    prerelease: Option<String>,
}

impl Version {
    fn parse(text: &str) -> Option<Self> {
        let text = text.trim().trim_start_matches(['v', 'V']);
        let text = text.split('+').next()?;
        let (core, prerelease) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre.to_owned())),
            None => (text, None),
        };
        let mut parts = core.split('.');
        let mut numbers = [0_u64; 3];
        for slot in &mut numbers {
            *slot = parts.next()?.parse().ok()?;
        }
        parts.next().is_none().then_some(Self {
            core: numbers,
            prerelease,
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        self.core
            .cmp(&other.core)
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => Ordering::Equal,
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A release as the release workflow publishes it: immutable, authored by the bot.
    fn reply(tag: &str) -> String {
        reply_by(tag, true, "github-actions[bot]")
    }

    fn reply_by(tag: &str, immutable: bool, author: &str) -> String {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let assets: Vec<String> = ["macos-arm64", "macos-x64"]
            .iter()
            .map(|platform| {
                format!(
                    r#"{{"name":"DegenView-{tag}-{platform}.zip","size":10,"digest":"{digest}","browser_download_url":"{DOWNLOAD_PREFIX}{tag}/DegenView-{tag}-{platform}.zip"}}"#
                )
            })
            .collect();
        format!(
            r#"{{"tag_name":"{tag}","immutable":{immutable},"author":{{"login":"{author}"}},"html_url":"https://github.com/degenview/degenview/releases/tag/{tag}","assets":[{}]}}"#,
            assets.join(",")
        )
    }

    fn available(outcome: UpdateOutcome) -> Release {
        match outcome {
            UpdateOutcome::Available(release) => release,
            other => panic!("expected an update, got {other:?}"),
        }
    }

    #[test]
    fn versions_order_numerically_not_textually() {
        let v = |text| Version::parse(text).unwrap();
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("v1.0.0") > v("0.99.99"));
        assert!(v("0.2.0-rc1") < v("0.2.0"));
        assert_eq!(v("v0.1.0"), v("0.1.0+build5"));
        assert!(Version::parse("0.1").is_none());
        assert!(Version::parse("nightly").is_none());
        assert!(Version::parse("1.2.3.4").is_none());
    }

    #[test]
    fn a_newer_tag_is_an_update() {
        let release = available(evaluate(&reply("v0.2.0"), "0.1.0"));
        assert_eq!(release.tag, "v0.2.0");
        assert_eq!(release.version, "0.2.0");
        assert_eq!(
            release.url,
            "https://github.com/degenview/degenview/releases/tag/v0.2.0"
        );
        assert_eq!(release.manual_only, None);
        // An installable asset exists only on platforms the updater can replace in place.
        assert_eq!(release.asset.is_some(), platform_suffix().is_some());
    }

    #[test]
    fn only_immutable_releases_from_the_workflow_are_installable() {
        let hand_made = available(evaluate(&reply_by("v0.2.0", true, "someone"), "0.1.0"));
        assert!(hand_made.asset.is_none());
        assert!(hand_made.manual_only.unwrap().contains("maintainer"));
        // The maintainer's own account is accepted, case-insensitively like GitHub logins.
        for login in ["nicoSWD", "nicoswd"] {
            let by_maintainer = available(evaluate(&reply_by("v0.2.0", true, login), "0.1.0"));
            assert_eq!(by_maintainer.manual_only, None, "{login}");
        }
        let mutable = available(evaluate(
            &reply_by("v0.2.0", false, "github-actions[bot]"),
            "0.1.0",
        ));
        assert!(mutable.asset.is_none());
        assert!(mutable.manual_only.unwrap().contains("immutable"));
        let anonymous = r#"{"tag_name":"v0.2.0","immutable":true}"#;
        let anonymous = available(evaluate(anonymous, "0.1.0"));
        assert!(anonymous.asset.is_none() && anonymous.manual_only.is_some());
    }

    #[test]
    fn same_or_older_tag_is_up_to_date() {
        assert_eq!(evaluate(&reply("v0.1.0"), "0.1.0"), UpdateOutcome::UpToDate);
        assert_eq!(evaluate(&reply("v0.1.0"), "0.3.0"), UpdateOutcome::UpToDate);
    }

    #[test]
    fn drafts_and_prereleases_are_ignored() {
        let body = r#"{"tag_name":"v9.0.0","prerelease":true}"#;
        assert_eq!(evaluate(body, "0.1.0"), UpdateOutcome::UpToDate);
        let body = r#"{"tag_name":"v9.0.0","draft":true}"#;
        assert_eq!(evaluate(body, "0.1.0"), UpdateOutcome::UpToDate);
    }

    #[test]
    fn foreign_release_urls_fall_back_to_the_releases_page() {
        let body = r#"{"tag_name":"v0.2.0","html_url":"https://evil.example/x"}"#;
        assert_eq!(available(evaluate(body, "0.1.0")).url, RELEASES_URL);
    }

    fn asset(name: &str, url_tail: &str, size: u64, digest: Option<&str>) -> AssetDto {
        AssetDto {
            name: name.to_owned(),
            browser_download_url: format!("{DOWNLOAD_PREFIX}{url_tail}"),
            size,
            digest: digest.map(str::to_owned),
        }
    }

    #[test]
    fn the_platforms_zip_is_picked_with_its_digest() {
        let digest = format!("sha256:{}", "AB".repeat(32));
        let assets = vec![
            asset(
                "DegenView-v1.0.0-macos-x64.zip",
                "v1.0.0/x64",
                10,
                Some(&digest),
            ),
            asset(
                "DegenView-v1.0.0-macos-arm64.zip",
                "v1.0.0/arm",
                20,
                Some(&digest),
            ),
        ];
        let picked = select_asset(assets, Some("-macos-arm64.zip")).unwrap();
        assert_eq!(picked.size, 20);
        assert_eq!(picked.sha256, "ab".repeat(32));
        assert!(picked.url.ends_with("v1.0.0/arm"));
    }

    #[test]
    fn assets_without_a_trustworthy_digest_or_url_are_not_installable() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        let pick = |asset| select_asset(vec![asset], Some("-macos-arm64.zip"));
        let name = "DegenView-v1.0.0-macos-arm64.zip";
        assert!(pick(asset(name, "v1.0.0/a", 5, None)).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 5, Some("sha256:abc"))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 5, Some("md5:00"))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", 0, Some(&digest))).is_none());
        assert!(pick(asset(name, "v1.0.0/a", MAX_ASSET_BYTES + 1, Some(&digest))).is_none());
        let mut foreign = asset(name, "v1.0.0/a", 5, Some(&digest));
        foreign.browser_download_url = "https://evil.example/a.zip".to_owned();
        assert!(pick(foreign).is_none());
        assert!(select_asset(vec![asset(name, "x", 5, Some(&digest))], None).is_none());
    }

    #[test]
    fn garbage_is_a_failure_not_an_update() {
        assert!(matches!(
            evaluate("<html>", "0.1.0"),
            UpdateOutcome::Failed(_)
        ));
        assert!(matches!(
            evaluate(r#"{"tag_name":"nightly"}"#, "0.1.0"),
            UpdateOutcome::Failed(_)
        ));
    }
}
