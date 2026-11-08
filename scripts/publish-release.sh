#!/usr/bin/env bash
# Publishes the version in Cargo.toml as a GitHub release, the place UniHub's updater
# looks for new versions:
#   1. checks the working tree is committed, the commit is on GitHub and v<version> is new;
#   2. builds release/UniHub-<version>.dmg and its .sha256 (scripts/make-dmg.sh);
#   3. shows what it will publish and asks;
#   4. creates the release as a draft, uploads both files, then publishes it, so the
#      updater never sees a release without its files. GitHub creates the tag.
#
# Usage: scripts/publish-release.sh [--notes-file FILE] [--skip-build] [--dry-run] [--yes]
#   --notes-file FILE  release notes (Markdown); default: the commits since the last release
#   --skip-build       publish the DMG already in release/
#   --dry-run          check and show everything, publish nothing (no token needed)
#   --yes              do not ask before publishing
#
# Sign-in: GitHub lets scripts publish only with a token; a passkey or password cannot be
# used here. The script uses $GH_TOKEN or $GITHUB_TOKEN, GitHub CLI's login, or a token git
# keeps for github.com. With none, it asks: sign in in the browser with GitHub CLI (works
# with a passkey; offers to install it with Homebrew), or paste a personal access token
# (fine-grained: this repository, Contents read and write; classic: "public_repo").
set -euo pipefail
cd "$(dirname "$0")/.."

NOTES_FILE=""
SKIP_BUILD=0
DRY_RUN=0
ASSUME_YES=0
while [ $# -gt 0 ]; do
  case "$1" in
    --notes-file) NOTES_FILE=${2:?--notes-file needs a file}; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --yes | -y) ASSUME_YES=1; shift ;;
    -h | --help) awk 'NR > 1 && /^#/ { sub(/^# ?/, ""); print; next } NR > 1 { exit }' "$0"; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }
PROBLEMS=0
# A failed check stops a real run; a dry run reports it and goes on.
problem() {
  if [ "$DRY_RUN" = 1 ]; then echo "problem: $*" >&2; PROBLEMS=$((PROBLEMS + 1)); else die "$*"; fi
}
command -v python3 >/dev/null || die "python3 is needed (it comes with the Xcode command line tools)"

# --- What to publish -------------------------------------------------------------
REMOTE=$(git remote get-url origin)
SLUG=$(printf '%s' "$REMOTE" | sed -E 's#^(https://([^@/]+@)?github\.com/|git@github\.com:|ssh://git@github\.com/)##; s#\.git$##; s#/$##')
[[ "$SLUG" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "origin is not a GitHub repository: $REMOTE"
if ! grep -q "repos/$SLUG/releases" crates/unihub-core/src/updates.rs; then
  echo "warning: the updater (crates/unihub-core/src/updates.rs) does not read releases of $SLUG" >&2
fi

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$VERSION" ] || die "no version in Cargo.toml"
TAG="v$VERSION"
BRANCH=$(git branch --show-current)
HEAD_SHA=$(git rev-parse HEAD)

# This script itself does not end up in the DMG, so its own edits do not count.
[ -z "$(git status --porcelain -- . ':(exclude)scripts/publish-release.sh')" ] ||
  problem "the working tree has uncommitted changes; commit them first (the release is built from the committed code)"
git fetch --quiet origin || problem "could not fetch from origin"
[ -n "$(git branch -r --contains "$HEAD_SHA" 2>/dev/null)" ] ||
  problem "commit ${HEAD_SHA:0:7} is not on GitHub yet; push it first: git push origin ${BRANCH:-HEAD}"
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  problem "$TAG already exists on GitHub; bump version in Cargo.toml (now $VERSION)"
fi

# Release notes: the given file, or the commit subjects since the last v* tag.
if [ -n "$NOTES_FILE" ]; then
  NOTES=$(cat "$NOTES_FILE")
else
  PREV=$(git describe --tags --abbrev=0 --match 'v*' HEAD 2>/dev/null || true)
  CHANGES=$(git log --no-merges --pretty='- %s' "${PREV:+$PREV..}HEAD")
  NOTES="## Changes${PREV:+ since $PREV}

$CHANGES

## Install

Download UniHub-$VERSION.dmg, open it and drag UniHub to Applications. UniHub is not
notarized: the first time, open System Settings → Privacy & Security and click Open Anyway.
Installed copies update themselves (UniHub → Check for Updates…)."
fi

# --- Build -----------------------------------------------------------------------
DMG="release/UniHub-$VERSION.dmg"
SUM="$DMG.sha256"
if [ "$SKIP_BUILD" = 0 ]; then
  ./scripts/make-dmg.sh
fi
# An ad-hoc signed release makes macOS ask every user for permissions again after updating,
# and cannot be checked against the signer of the copy being updated.
if codesign -dv dist/UniHub.app 2>&1 | grep -q '^Signature=adhoc'; then
  problem "dist/UniHub.app is ad-hoc signed; put a code-signing identity in .signing-identity (see README) and build again"
fi
if [ -f "$DMG" ] && [ -f "$SUM" ]; then
  (cd release && shasum -a 256 -c "$(basename "$SUM")" >/dev/null) || die "$DMG does not match $SUM"
else
  problem "$DMG or $SUM is missing (run without --skip-build)"
fi

echo
echo "Release $TAG of $SLUG"
echo "  commit  ${HEAD_SHA:0:7} on ${BRANCH:-a detached HEAD}"
if [ -f "$DMG" ]; then echo "  files   $DMG ($(du -h "$DMG" | cut -f1)), $SUM"; fi
echo "  notes"
printf '%s\n' "$NOTES" | sed 's/^/    | /'
echo

if [ "$DRY_RUN" = 1 ]; then
  if [ "$PROBLEMS" -gt 0 ]; then echo "Dry run: $PROBLEMS problem(s) above would stop a real run."; else echo "Dry run: all checks passed; nothing was published."; fi
  exit 0
fi
if [ "$ASSUME_YES" != 1 ]; then
  read -r -p "Publish $TAG on GitHub? [y/N] " answer
  [[ "$answer" =~ ^[Yy] ]] || die "cancelled"
fi

# --- Token -----------------------------------------------------------------------
# A token that is already there, without prompting: environment, GitHub CLI, or one that
# git keeps for github.com (Keychain, Git Credential Manager).
stored_token() {
  if [ -n "${GH_TOKEN:-}" ]; then printf '%s' "$GH_TOKEN"; return; fi
  if [ -n "${GITHUB_TOKEN:-}" ]; then printf '%s' "$GITHUB_TOKEN"; return; fi
  if command -v gh >/dev/null 2>&1 && gh auth token >/dev/null 2>&1; then gh auth token; return; fi
  local user
  user=$(printf '%s' "$REMOTE" | sed -nE 's#^https://([^@/:]+)@github\.com/.*#\1#p')
  printf 'protocol=https\nhost=github.com\n%s\n' "${user:+username=$user}" |
    GIT_TERMINAL_PROMPT=0 GCM_INTERACTIVE=never git credential fill 2>/dev/null | sed -n 's/^password=//p' || true
}

# True when GitHub accepts the token.
token_works() {
  [ -n "$1" ] || return 1
  local code
  code=$(curl -sS -o /dev/null -w '%{http_code}' -K - https://api.github.com/user \
    <<<"header = \"Authorization: Bearer $1\"") || return 1
  [ "$code" = 200 ]
}

# Prompts read from the terminal, so they also work inside $(...).
ask() {
  local reply
  read -r -p "$1" reply </dev/tty
  printf '%s' "$reply"
}

# Browser sign-in through GitHub CLI; passkeys work there. Prints the token.
login_with_gh() {
  if ! command -v gh >/dev/null 2>&1; then
    command -v brew >/dev/null 2>&1 ||
      die "GitHub CLI is not installed; install it from https://cli.github.com and run this again"
    answer=$(ask "GitHub CLI is not installed. Install it now with Homebrew (brew install gh)? [Y/n] ")
    if [[ "$answer" =~ ^[Nn] ]]; then die "cancelled"; fi
    brew install gh >/dev/tty 2>&1
  fi
  echo "GitHub CLI copies a one-time code to the clipboard and opens github.com/login/device:" >&2
  echo "sign in there (your passkey works), paste the code and authorize. This continues by itself." >&2
  # Without its own prompts: GitHub CLI's interactive questions break in some terminals
  # ("unexpected escape sequence from terminal"); the git question is asked below instead.
  GH_PROMPT_DISABLED=1 NO_COLOR=1 gh auth login --hostname github.com --git-protocol https --web \
    </dev/null >&2 2>&1 || die "GitHub CLI sign-in did not finish"
  answer=$(ask "Let git push use this GitHub login too, instead of asking for a password? [Y/n] ")
  if [[ ! "$answer" =~ ^[Nn] ]]; then
    gh auth setup-git --hostname github.com >&2 && echo "git now signs in to github.com through GitHub CLI." >&2
  fi
  gh auth token
}

paste_token() {
  echo "Create a token at https://github.com/settings/personal-access-tokens/new" >&2
  echo "(sign in with your passkey): Repository access: Only select repositories: $SLUG;" >&2
  echo "Permissions: Contents: Read and write." >&2
  local token
  read -r -s -p "Paste the token (it is not shown): " token </dev/tty
  echo >&2
  printf '%s' "$token"
}

TOKEN=$(stored_token)
if [ -n "$TOKEN" ] && ! token_works "$TOKEN"; then
  echo "The saved GitHub token was refused (expired or revoked)." >&2
  TOKEN=""
fi
if [ -z "$TOKEN" ]; then
  ( : </dev/tty ) 2>/dev/null ||
    die "no GitHub token. Run this in a terminal to sign in, or set GH_TOKEN (see --help)"
  echo >&2
  echo "Publishing needs a GitHub token: GitHub does not let scripts use a passkey or password." >&2
  echo "How do you want to sign in?" >&2
  echo "  1) In the browser with GitHub CLI (works with your passkey; recommended)" >&2
  echo "  2) Paste a personal access token" >&2
  choice=$(ask "Choose 1 or 2 [1]: ")
  case "${choice:-1}" in
    1) TOKEN=$(login_with_gh) ;;
    2) TOKEN=$(paste_token) ;;
    *) die "cancelled" ;;
  esac
  token_works "$TOKEN" || die "GitHub did not accept that token"
fi

# The token goes to curl through a config on a pipe, never on the command line.
AUTH_CONFIG=$(printf 'header = "Authorization: Bearer %s"\nheader = "Accept: application/vnd.github+json"\nheader = "X-GitHub-Api-Version: 2022-11-28"\n' "$TOKEN")

# api METHOD URL [JSON] → prints the response body; dies on an HTTP error.
api() {
  local out code
  out=$(curl -sS -w '\n%{http_code}' -X "$1" -K - ${3:+-H "Content-Type: application/json" --data "$3"} "$2" <<<"$AUTH_CONFIG") ||
    die "could not reach GitHub"
  code=${out##*$'\n'}
  out=${out%$'\n'*}
  if [ "$code" -lt 200 ] || [ "$code" -ge 300 ]; then
    message=$(printf '%s' "$out" | python3 -c 'import json,sys
try: d=json.load(sys.stdin); print(d.get("message",""), "".join(" / "+e.get("message",e.get("code","")) for e in d.get("errors",[]) if isinstance(e,dict)))
except Exception: print("")')
    case "$code" in
      401 | 403 | 404) message="$message (the token must be allowed to write $SLUG: Contents read and write)" ;;
    esac
    die "GitHub answered $code to $1 $2: $message"
  fi
  printf '%s' "$out"
}
field() { python3 -c 'import json,sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"; }

# --- Publish ---------------------------------------------------------------------
REQUEST=$(TAG="$TAG" SHA="$HEAD_SHA" NOTES="$NOTES" VERSION="$VERSION" python3 -c 'import json,os
print(json.dumps({"tag_name": os.environ["TAG"], "target_commitish": os.environ["SHA"],
                  "name": "UniHub " + os.environ["VERSION"], "body": os.environ["NOTES"], "draft": True}))')
echo "Creating draft release ${TAG}…"
RELEASE=$(api POST "https://api.github.com/repos/$SLUG/releases" "$REQUEST")
RELEASE_ID=$(printf '%s' "$RELEASE" | field id)
# Remove the draft again if anything below fails.
trap 'echo "Removing the unfinished draft…" >&2; api DELETE "https://api.github.com/repos/$SLUG/releases/$RELEASE_ID" >/dev/null || true' ERR

# Uploads send the file itself as the body, so they call curl directly.
upload() {
  local file=$1 type=$2 out code
  echo "Uploading $(basename "$file") ($(du -h "$file" | cut -f1))…"
  out=$(curl -sS -w '\n%{http_code}' -X POST -K - -H "Content-Type: $type" --data-binary @"$file" \
    "https://uploads.github.com/repos/$SLUG/releases/$RELEASE_ID/assets?name=$(basename "$file")" <<<"$AUTH_CONFIG")
  code=${out##*$'\n'}
  [ "$code" -ge 200 ] && [ "$code" -lt 300 ] || { echo "error: upload of $file failed ($code)" >&2; return 1; }
}
upload "$DMG" application/x-apple-diskimage
upload "$SUM" text/plain

echo "Publishing…"
PUBLISHED=$(api PATCH "https://api.github.com/repos/$SLUG/releases/$RELEASE_ID" '{"draft": false}')
trap - ERR
git fetch --quiet --tags origin || true
echo "Published $(printf '%s' "$PUBLISHED" | field html_url)"
