#!/usr/bin/env bash
# Write the Tauri updater manifest (latest.json) for a release.
#   updater-manifest.sh <release-dir> <version> <notes-file> > latest.json
#
# <release-dir> holds the renamed desktop bundles and their .sig files (see the
# desktop job in release.yml). tauri-plugin-updater 2.14 looks up
# "{os}-{arch}-{installer}" first, then "{os}-{arch}"; so every bundle gets its own
# key, and the plain keys point at the AppImage (Linux) and NSIS installer
# (Windows), which installs from before per-installer keys existed also understand.
set -euo pipefail

dir=$1
version=$2
notes_file=$3
base="https://github.com/Hellsparks/Foreman/releases/download/v${version}"
# Pre-releases have no .rpm (see release.yml).
rpm=yes
[[ $version == *-* ]] && rpm=no

# key|file|required
entries=(
  "windows-x86_64|Foreman_${version}_windows_x64-setup.exe|yes"
  "windows-x86_64-nsis|Foreman_${version}_windows_x64-setup.exe|yes"
  "linux-x86_64|Foreman_${version}_linux_x86_64.AppImage|yes"
  "linux-x86_64-appimage|Foreman_${version}_linux_x86_64.AppImage|yes"
  "linux-x86_64-deb|Foreman_${version}_linux_amd64.deb|yes"
  "linux-x86_64-rpm|Foreman_${version}_linux_x86_64.rpm|$rpm"
  "linux-aarch64|Foreman_${version}_linux_aarch64.AppImage|yes"
  "linux-aarch64-appimage|Foreman_${version}_linux_aarch64.AppImage|yes"
  "linux-aarch64-deb|Foreman_${version}_linux_arm64.deb|yes"
  "linux-aarch64-rpm|Foreman_${version}_linux_aarch64.rpm|$rpm"
)

platforms='{}'
missing=0
for e in "${entries[@]}"; do
  IFS='|' read -r key file required <<<"$e"
  if [[ ! -f "$dir/$file" || ! -s "$dir/$file.sig" ]]; then
    echo "updater-manifest: missing $file or $file.sig for $key" >&2
    [[ $required == yes ]] && missing=1
    continue
  fi
  platforms=$(jq --arg k "$key" --arg url "$base/$file" --rawfile sig "$dir/$file.sig" \
    '. + {($k): {signature: ($sig | rtrimstr("\n")), url: $url}}' <<<"$platforms")
done
[[ $missing == 0 ]] || exit 1

jq -n --arg v "$version" --rawfile notes "$notes_file" \
  --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --argjson p "$platforms" \
  '{version: $v, notes: ($notes | rtrimstr("\n")), pub_date: $date, platforms: $p}'
