#!/bin/sh
# Cut a release: bump the version in Cargo.toml/Cargo.lock, commit, sign-tag vX.Y.Z and push.
#
#   tools/release.sh 0.1.3            ask before doing anything
#   tools/release.sh 0.1.3 --yes      no prompt
#   tools/release.sh 0.1.3 --dry-run  print what would happen, change nothing
#
# Cargo.toml is the single source of the app's version (Settings > About, the update check, the
# app bundle's Info.plist). Pushing the tag starts the Release workflow, which refuses a tag that
# does not match Cargo.toml, so never tag by hand.
set -eu

cd "$(dirname "$0")/.."

usage() {
    echo "usage: tools/release.sh <X.Y.Z[-prerelease]> [--yes] [--dry-run]" >&2
    exit 2
}

version=""
assume_yes=0
dry_run=0
for arg in "$@"; do
    case "$arg" in
        --yes) assume_yes=1 ;;
        --dry-run) dry_run=1 ;;
        -*) usage ;;
        *) [ -z "$version" ] || usage; version="$arg" ;;
    esac
done
[ -n "$version" ] || usage

version="${version#v}"
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
    echo "error: '$version' is not X.Y.Z or X.Y.Z-prerelease" >&2
    exit 1
fi
tag="v$version"

# The `version` line of the [package] table, however many other tables follow it.
current=$(awk '/^\[/ { in_package = ($0 == "[package]") } in_package && /^version = "/ { gsub(/^version = "|"$/, ""); print; exit }' Cargo.toml)
if [ -z "$current" ]; then
    echo "error: could not read [package] version from Cargo.toml" >&2
    exit 1
fi

# Compare the numeric X.Y.Z core only; the new version must be strictly newer.
core() { printf '%s\n' "${1%%-*}"; }
newest=$(printf '%s\n%s\n' "$(core "$current")" "$(core "$version")" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)
if [ "$(core "$version")" = "$(core "$current")" ]; then
    # Only a pre-release can be followed by its own release (0.2.0-rc1 -> 0.2.0).
    case "$current" in
        *-*) [ "$version" != "$current" ] || { echo "error: already at $current" >&2; exit 1; } ;;
        *) echo "error: $version is not newer than the current version $current" >&2; exit 1 ;;
    esac
elif [ "$newest" != "$(core "$version")" ]; then
    echo "error: $version is not newer than the current version $current" >&2
    exit 1
fi

if [ "$(git rev-parse --abbrev-ref HEAD)" != "main" ]; then
    echo "error: releases are cut from main" >&2
    exit 1
fi
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "error: the working tree has uncommitted changes" >&2
    exit 1
fi
if [ -z "$(git config user.signingkey)" ]; then
    echo "error: git user.signingkey is not set; release tags must be signed (git tag -s)" >&2
    exit 1
fi
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    echo "error: tag $tag already exists" >&2
    exit 1
fi

echo "Release $tag"
echo "  1. Cargo.toml / Cargo.lock: $current -> $version"
echo "  2. commit 'Release $tag', signed tag $tag (verified locally before the push)"
echo "  3. git push origin main $tag   (starts the Release workflow)"
if [ "$dry_run" = 1 ]; then
    echo "(dry run: nothing changed)"
    exit 0
fi
if [ "$assume_yes" != 1 ]; then
    printf 'Continue? [y/N] '
    read -r answer
    case "$answer" in y | Y) ;; *) echo "aborted"; exit 1 ;; esac
fi

# Only the [package] version line changes; dependency tables keep their own `version =` keys.
awk -v new="$version" '
    /^\[/ { in_package = ($0 == "[package]") }
    in_package && !done && /^version = "/ { print "version = \"" new "\""; done = 1; next }
    { print }
' Cargo.toml > Cargo.toml.release.tmp
mv Cargo.toml.release.tmp Cargo.toml

# Rewrites just the root package's entry in Cargo.lock.
cargo update --offline --workspace >/dev/null

git add Cargo.toml Cargo.lock
git commit -m "Release $tag"
git tag -s "$tag" -m "DegenView $tag"
# The app only installs updates whose tag GitHub reports as verified, so catch a bad signature
# here, before pushing. Needs gpg.ssh.allowedSignersFile; without it GitHub is the only check.
if [ -n "$(git config gpg.ssh.allowedSignersFile)" ]; then
    if ! git verify-tag "$tag"; then
        echo "error: $tag does not verify locally; nothing was pushed." >&2
        echo "Undo with: git tag -d $tag && git reset --hard HEAD~1" >&2
        exit 1
    fi
else
    echo "warning: gpg.ssh.allowedSignersFile is not set, so $tag was not verified locally" >&2
fi
git push origin main "$tag"
echo "Pushed $tag. Follow the build: gh run watch"
