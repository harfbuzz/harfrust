#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <new-version>" >&2
  exit 1
fi

new_version="$1"

if [[ ! "$new_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "error: version must match <major>.<minor>.<patch>" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"

cargo_toml="$repo_root/Cargo.toml"
hr_shape_toml="$repo_root/hr-shape/Cargo.toml"
changelog="$repo_root/CHANGELOG.md"

current_version="$(
  perl -ne 'print "$1\n" if /^\s*version = "([^"]+)"\s*$/' "$cargo_toml" | head -n1
)"

if [[ -z "$current_version" ]]; then
  echo "error: failed to read current workspace version from $cargo_toml" >&2
  exit 1
fi

if [[ "$current_version" == "$new_version" ]]; then
  echo "error: version is already $new_version" >&2
  exit 1
fi

today="$(date +%F)"

OLD_VERSION="$current_version" NEW_VERSION="$new_version" perl -0pi -e '
  my $old = $ENV{OLD_VERSION};
  my $new = $ENV{NEW_VERSION};
  s/^version = "\Q$old\E"$/version = "$new"/m
    or die "failed to update workspace version\n";
' "$cargo_toml"

NEW_VERSION="$new_version" perl -0pi -e '
  my $new = $ENV{NEW_VERSION};
  s/version = "=[^"]+"/version = "=$new"/
    or die "failed to update hr-shape harfrust dependency\n";
' "$hr_shape_toml"

OLD_VERSION="$current_version" NEW_VERSION="$new_version" TODAY="$today" perl -0pi -e '
  my $old = $ENV{OLD_VERSION};
  my $new = $ENV{NEW_VERSION};
  my $today = $ENV{TODAY};

  s{^## \[Unreleased\]\n}{## [Unreleased]\n\n## [$new] - $today\n\n}m
    or die "failed to add changelog release heading\n";

  s{^\[Unreleased\]: https://github\.com/harfbuzz/harfrust/compare/\Q$old\E\.\.\.HEAD$}{[Unreleased]: https://github.com/harfbuzz/harfrust/compare/$new...HEAD\n[$new]: https://github.com/harfbuzz/harfrust/compare/$old...$new}m
    or die "failed to update changelog compare links\n";
' "$changelog"

echo "bumped version: $current_version -> $new_version"
