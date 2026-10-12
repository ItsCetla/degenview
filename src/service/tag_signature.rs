//! Checks that a release's git tag was signed by the DegenView maintainer before the updater
//! downloads anything.
//!
//! Tags are SSH-signed (`git tag -s`). The app carries the maintainer's public key and verifies
//! the signature itself; GitHub's own `verified` flag is never read, because it is only GitHub's
//! word and adds nothing over the zip's checksum, which GitHub also supplies. What this stops is a
//! release created by someone who can publish to the repository but does not hold the signing key.
//! It vouches for "this tag names this commit", not for the zip CI uploaded afterwards; enabling
//! GitHub's immutable releases covers most of that (see AGENTS.md ▸ Releasing).

use crate::service::http::{self, Retry};
use crate::service::update_check::github_headers;
use serde::Deserialize;
use ssh_key::{PublicKey, SshSig};

/// Public keys (OpenSSH format) whose tag signatures are accepted. Add the next key here, in a
/// release signed by the current one, before retiring the old key. `tools/allowed_signers` lists
/// the same keys for the release workflow.
pub const PINNED_SIGNERS: &[&str] =
    &["ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBlY8SnxH+OvwIn51hv77uC/8jPC9iOlrGhk9P6epZcv signing"];

/// The namespace `git tag -s` signs under.
const NAMESPACE: &str = "git";
const API: &str = "https://api.github.com/repos/degenview/degenview/git";

#[derive(Debug, Eq, PartialEq)]
pub enum TagError {
    /// No tag object, a lightweight tag, or no signature on it.
    Unsigned,
    /// A signature exists but is not valid for any pinned key.
    NotTrusted,
    /// Signed, but for a different tag or a non-commit target: a replayed signature.
    WrongTag,
    Network(String),
}

impl std::fmt::Display for TagError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsigned => f.write_str("This release's tag is not signed."),
            Self::NotTrusted => {
                f.write_str("This release's tag is not signed by the DegenView maintainer.")
            }
            Self::WrongTag => f.write_str("This release's tag signature belongs to another tag."),
            Self::Network(message) => {
                write!(f, "Could not check the release signature: {message}")
            }
        }
    }
}

#[derive(Deserialize)]
struct RefDto {
    object: ObjectDto,
}

#[derive(Deserialize)]
struct ObjectDto {
    sha: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct TagDto {
    verification: Option<VerificationDto>,
}

/// Only the raw signature and the bytes it covers; the `verified` verdict is not deserialized.
#[derive(Deserialize)]
struct VerificationDto {
    signature: Option<String>,
    payload: Option<String>,
}

/// Blocking: two GETs. `Ok` only when `tag` is an annotated tag signed by a pinned key.
pub fn verify_release_tag(tag: &str) -> Result<(), TagError> {
    verify_release_tag_at(API, tag, PINNED_SIGNERS)
}

fn verify_release_tag_at(api: &str, tag: &str, keys: &[&str]) -> Result<(), TagError> {
    if tag.is_empty()
        || !tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(TagError::WrongTag);
    }
    let fetch = |url: String| {
        http::get_text(&url, &github_headers(), Retry::Transient)
            .map_err(|error| TagError::Network(error.to_string()))
    };
    let reference: RefDto = serde_json::from_str(&fetch(format!("{api}/ref/tags/{tag}"))?)
        .map_err(|_| TagError::Network("unreadable reply from GitHub".to_owned()))?;
    if reference.object.kind != "tag" {
        // A lightweight tag points straight at a commit and cannot carry a signature.
        return Err(TagError::Unsigned);
    }
    let object: TagDto =
        serde_json::from_str(&fetch(format!("{api}/tags/{}", reference.object.sha))?)
            .map_err(|_| TagError::Network("unreadable reply from GitHub".to_owned()))?;
    let Some(VerificationDto {
        signature: Some(signature),
        payload: Some(payload),
    }) = object.verification
    else {
        return Err(TagError::Unsigned);
    };
    verify_payload(tag, &payload, &signature, keys).map(drop)
}

/// Verifies `signature` (armored SSHSIG) over `payload` (the tag object as git signed it) against
/// `keys`, and that the payload is a commit tag named `tag`. Returns the tagged commit's sha.
pub fn verify_payload(
    tag: &str,
    payload: &str,
    signature: &str,
    keys: &[&str],
) -> Result<String, TagError> {
    let signature = SshSig::from_pem(signature).map_err(|_| TagError::Unsigned)?;
    let trusted = keys
        .iter()
        .filter_map(|key| PublicKey::from_openssh(key).ok())
        .any(|key| {
            key.verify(NAMESPACE, payload.as_bytes(), &signature)
                .is_ok()
        });
    if !trusted {
        return Err(TagError::NotTrusted);
    }
    // The signature is good; make sure it is for this tag and not a replayed older one.
    let header = payload.split("\n\n").next().unwrap_or_default();
    let field = |name: &str| {
        header
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
    };
    match (field("object"), field("type"), field("tag")) {
        (Some(commit), Some("commit"), Some(name)) if name == tag => Ok(commit.to_owned()),
        _ => Err(TagError::WrongTag),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYLOAD: &str = include_str!("testdata/tag_v0.1.2.payload");
    const SIGNATURE: &str = include_str!("testdata/tag_v0.1.2.sig");
    const OTHER_KEY: &str = include_str!("testdata/throwaway.pub");
    const OTHER_KEY_GIT: &str = include_str!("testdata/other_key_git.sig");
    const OTHER_KEY_FILE: &str = include_str!("testdata/other_key_file.sig");
    const COMMIT: &str = "6271e9ef97cde8b3b12d18a04ccc00edb3faa0ea";

    #[test]
    fn the_real_v0_1_2_tag_verifies_against_the_pinned_key() {
        assert_eq!(
            verify_payload("v0.1.2", PAYLOAD, SIGNATURE, PINNED_SIGNERS).unwrap(),
            COMMIT
        );
    }

    #[test]
    fn a_tampered_payload_is_not_trusted() {
        let tampered = PAYLOAD.replace(COMMIT, &"0".repeat(40));
        assert_eq!(
            verify_payload("v0.1.2", &tampered, SIGNATURE, PINNED_SIGNERS),
            Err(TagError::NotTrusted)
        );
    }

    #[test]
    fn a_signature_replayed_for_another_tag_is_rejected() {
        assert_eq!(
            verify_payload("v0.1.3", PAYLOAD, SIGNATURE, PINNED_SIGNERS),
            Err(TagError::WrongTag)
        );
    }

    #[test]
    fn a_signature_by_an_unpinned_key_is_not_trusted() {
        assert_eq!(
            verify_payload("v0.1.2", PAYLOAD, OTHER_KEY_GIT, PINNED_SIGNERS),
            Err(TagError::NotTrusted)
        );
        // The same signature is fine once its key is pinned: the rejection above is the key.
        assert!(verify_payload("v0.1.2", PAYLOAD, OTHER_KEY_GIT, &[OTHER_KEY.trim()]).is_ok());
    }

    #[test]
    fn only_the_git_namespace_counts() {
        assert_eq!(
            verify_payload("v0.1.2", PAYLOAD, OTHER_KEY_FILE, &[OTHER_KEY.trim()]),
            Err(TagError::NotTrusted)
        );
    }

    #[test]
    fn junk_and_missing_signatures_are_unsigned() {
        assert_eq!(
            verify_payload("v0.1.2", PAYLOAD, "", PINNED_SIGNERS),
            Err(TagError::Unsigned)
        );
        assert_eq!(
            verify_payload("v0.1.2", PAYLOAD, "not a signature", PINNED_SIGNERS),
            Err(TagError::Unsigned)
        );
    }

    #[test]
    fn no_pinned_keys_trusts_nothing() {
        assert_eq!(
            verify_payload("v0.1.2", PAYLOAD, SIGNATURE, &[]),
            Err(TagError::NotTrusted)
        );
        assert_eq!(
            verify_payload("v0.1.2", PAYLOAD, SIGNATURE, &["garbage"]),
            Err(TagError::NotTrusted)
        );
    }

    #[test]
    fn the_release_workflows_signer_list_matches_the_pinned_keys() {
        let allowed = include_str!("../../tools/allowed_signers");
        for key in PINNED_SIGNERS {
            let blob = key.split_whitespace().nth(1).unwrap();
            assert!(
                allowed.contains(blob),
                "{key} missing from tools/allowed_signers"
            );
        }
    }

    #[test]
    fn odd_tag_names_never_reach_the_network() {
        for tag in ["", "../x", "v1/../../x", "v1 2"] {
            assert_eq!(
                verify_release_tag_at("http://127.0.0.1:1", tag, PINNED_SIGNERS),
                Err(TagError::WrongTag)
            );
        }
    }

    /// Hits GitHub: `cargo test -- --ignored the_published_tag`.
    #[test]
    #[ignore = "queries GitHub"]
    fn the_published_tag_verifies_live() {
        assert_eq!(verify_release_tag("v0.1.2"), Ok(()));
    }
}
