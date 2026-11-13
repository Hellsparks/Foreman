#!/usr/bin/env bash
# Release body and updater notes for a tag.
#   release-notes.sh <tag> <ref> <summary-file> <out-dir>
# Writes <out-dir>/body.md (GitHub Release body) and <out-dir>/notes.md (the
# summary only, shown in the app's update dialog).
#
# The summary is the annotated tag's message (git tag -a v0.4.0 -F notes.md);
# a workflow_dispatch dry run passes it as an input instead. Below it come the
# commits since the previous tag: the previous full release for a full release
# (so v0.4.0 lists everything since v0.3.x, rc commits included), the previous
# tag of any kind for a pre-release.
set -euo pipefail

tag=$1
ref=$2
summary_file=$3
out=$4
version=${tag#v}
repo_url="https://github.com/Hellsparks/Foreman"
mkdir -p "$out"

if [[ $version == *-* ]]; then
  prev=$(git describe --tags --abbrev=0 --match 'v*' "$ref^" 2>/dev/null || true)
else
  prev=$(git describe --tags --abbrev=0 --match 'v*' --exclude 'v*-*' "$ref^" 2>/dev/null || true)
fi

summary=$(sed -e '/-----BEGIN PGP SIGNATURE-----/,$d' "$summary_file" | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}')
[[ -n ${summary//[[:space:]]/} ]] || summary="_No summary for this release._"
printf '%s\n\nFull notes: %s/releases/tag/%s\n' "$summary" "$repo_url" "$tag" >"$out/notes.md"

{
  printf '%s\n\n' "$summary"
  if [[ -n $prev ]]; then
    printf '## Changes since %s\n\n' "$prev"
    range="$prev..$ref"
  else
    printf '## Changes\n\n'
    range="$ref"
  fi
  git log --no-merges --format='- %s (%h)' "$range"
  if [[ -n $prev ]]; then
    printf '\nFull diff: %s/compare/%s...%s\n' "$repo_url" "$prev" "$tag"
  fi

  cat <<EOF

## Downloads

| | File |
|---|---|
| Windows (x64) | \`Foreman_${version}_windows_x64-setup.exe\` |
| Linux desktop (x86_64 / ARM64) | \`Foreman_${version}_linux_x86_64.AppImage\`, \`.deb\`, \`.rpm\` (and the \`aarch64\` / \`arm64\` ones) |
| Headless server, Debian/Ubuntu/Raspberry Pi OS | \`foreman-server_${version}_amd64.deb\` / \`_arm64.deb\`: systemd service \`foreman\` on port 9090, data in \`/var/lib/foreman\` |
| Headless server, any Linux | \`foreman-server-x86_64-unknown-linux-gnu.tar.xz\` / \`aarch64\`: binary + \`frontend/\` folder, keep them together |
| Docker (amd64 / arm64) | \`docker run -d -p 9090:9090 -p 7912:7912 -v foreman-data:/data ghcr.io/hellsparks/foreman:${version}\` |

The desktop app updates itself from Settings → About. Every installer is signed for
the updater; the app refuses an update whose signature doesn't match the key built
into it.

## Verify your download

\`SHA256SUMS\` lists the SHA-256 checksum of every file in this release.

Linux (in the folder you downloaded to):

\`\`\`
sha256sum --ignore-missing -c SHA256SUMS
\`\`\`

Each file you have should print \`OK\`.

Windows (PowerShell): print the hash, then compare it with the line for that file in \`SHA256SUMS\`:

\`\`\`
(Get-FileHash .\\Foreman_${version}_windows_x64-setup.exe -Algorithm SHA256).Hash
\`\`\`

(PowerShell prints upper case, \`SHA256SUMS\` lower case; the letters must match.)

Docker: \`docker buildx imagetools inspect ghcr.io/hellsparks/foreman:${version}\` shows the image digest.
EOF
} >"$out/body.md"

echo "previous tag: ${prev:-none}" >&2
