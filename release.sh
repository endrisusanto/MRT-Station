#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

BUMP="${1:-patch}"
REMOTE_URL="https://github.com/endrisusanto/MRT-Station.git"
AUTO_COMMIT_MESSAGE="${AUTO_COMMIT_MESSAGE:-chore: auto commit before release}"
STATION_DIR="apps/station"

if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "This folder is not a git repository. Run: git init"
  exit 1
fi

if ! git remote get-url origin >/dev/null 2>&1; then
  git remote add origin "$REMOTE_URL"
elif [[ "$(git remote get-url origin)" != "$REMOTE_URL" ]]; then
  echo "Refusing to release to unexpected origin: $(git remote get-url origin)"
  echo "Expected: $REMOTE_URL"
  exit 1
fi

BRANCH="$(git branch --show-current)"
if [[ -z "$BRANCH" ]]; then
  echo "Cannot release from a detached HEAD. Check out a branch first."
  exit 1
fi

if [[ -n "$(git status --porcelain)" ]]; then
  echo "Auto-committing current workspace changes..."
  git add -A
  git commit -m "$AUTO_COMMIT_MESSAGE"
fi

echo "Syncing $BRANCH with origin..."
git fetch origin
if git show-ref --verify --quiet "refs/remotes/origin/$BRANCH"; then
  git pull --rebase origin "$BRANCH"
fi

NEW_VERSION="$(node - "$BUMP" "$STATION_DIR/package.json" <<'NODE'
const fs = require("fs");
const bump = process.argv[2];
const packagePath = process.argv[3];
const pkg = JSON.parse(fs.readFileSync(packagePath, "utf8"));
const current = String(pkg.version || "0.0.0");
const match = current.match(/^(\d+)\.(\d+)\.(\d+)$/);
if (!match) throw new Error(`Unsupported current version: ${current}`);

let [major, minor, patch] = match.slice(1).map(Number);
if (/^\d+\.\d+\.\d+$/.test(bump)) {
  console.log(bump);
  process.exit(0);
}

switch (bump) {
  case "major": major += 1; minor = 0; patch = 0; break;
  case "minor": minor += 1; patch = 0; break;
  case "patch": patch += 1; break;
  default: throw new Error(`Use patch, minor, major, or x.y.z. Got: ${bump}`);
}
console.log(`${major}.${minor}.${patch}`);
NODE
)"

TAG="v${NEW_VERSION}"
if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Tag $TAG already exists locally."
  exit 1
fi
if git ls-remote --exit-code --tags origin "refs/tags/$TAG" >/dev/null 2>&1; then
  echo "Tag $TAG already exists on origin."
  exit 1
fi

node - "$NEW_VERSION" <<'NODE'
const fs = require("fs");
const version = process.argv[2];

function updateJson(path) {
  const value = JSON.parse(fs.readFileSync(path, "utf8"));
  value.version = version;
  fs.writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

updateJson("apps/station/package.json");
updateJson("apps/station/src-tauri/tauri.conf.json");

const cargoPath = "Cargo.toml";
const cargo = fs.readFileSync(cargoPath, "utf8").replace(
  /(\[workspace\.package\][\s\S]*?\nversion = ")[^"]+("\n)/,
  `$1${version}$2`
);
fs.writeFileSync(cargoPath, cargo);
NODE

echo "Validating release $TAG..."
npm install --package-lock-only --prefix "$STATION_DIR"
cargo check --workspace
cargo test --workspace
npm run build --prefix "$STATION_DIR"

git add Cargo.toml Cargo.lock \
  "$STATION_DIR/package.json" \
  "$STATION_DIR/package-lock.json" \
  "$STATION_DIR/src-tauri/tauri.conf.json"
git commit -m "chore(release): ${TAG}"
git tag -a "$TAG" -m "$TAG"
git push origin "HEAD:$BRANCH" "$TAG"

echo "Released $TAG. GitHub Actions will build and publish the installers."

