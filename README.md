# Foreman

## Dev quick-start (fish shell)

```fish
# Headless server (port 9090)
cargo server

# Desktop shell (Tauri 2)
cargo tauri-dev
# or: cd crates/foreman-desktop && cargo tauri dev
```

`cargo tauri-dev` starts Vite (port 5173), builds the Rust desktop binary, and
opens a window pointing at the Vite dev server.  The embedded Foreman server
(API + WebSocket) runs on port 9090 inside the same process.

## Desktop app: tray, start with the computer, updates

- **Closing the window** hides Foreman to the tray while **Settings → This app → Keep
  running in the background** is on (default: on when this computer is the server,
  off when connected to another server). The built-in server keeps answering on the
  LAN. The tray menu has *Show Foreman*, the server's status and address, and *Quit
  Foreman*, which stops the server cleanly (listeners closed, databases closed) before
  exiting. If the desktop has no tray (some Linux setups without a StatusNotifier
  host), closing quits.
- **Start with the computer** (same page, default off) adds an OS login entry:
  `~/.config/autostart/Foreman.desktop` on Linux, a `Foreman` value under
  `…\CurrentVersion\Run` on Windows. It is launched with `--autostart`. It only keeps
  your fleet reachable when *Keep running in the background* is on — then the app
  starts hidden in the tray with the built-in server running. With it off, the window
  just opens at login and closing it quits.
- **Updates** come from GitHub Releases (`releases/latest/download/latest.json`). The
  app checks on launch and from **Settings → About → Check now**, shows the release
  notes, and installs only when you press *Install*. The app warns (never blocks)
  when its version and the server's differ by more than a patch release.
- **Beta updates** (Settings → About, default off) also offers pre-releases
  (`v0.4.0-rc.1`), from `beta.json` on the `update-channels` branch. Turning it off
  never downgrades; the next full release moves you back.

### Updater signing key (maintainer)

Update bundles must be signed; the app only accepts files signed by the key whose
public half is in `crates/foreman-desktop/tauri.conf.json` (`plugins.updater.pubkey`).

- Private key: `~/.tauri/foreman-updater.key` on the maintainer's machine, encrypted
  with the passphrase in `~/.tauri/foreman-updater.key.password`. **Never commit
  either.** Keep an offline copy of both in your password manager — if the key or
  its passphrase is lost, installed apps can never update again (a new key needs a
  manual reinstall).
- Release builds (CI) read them from two repository secrets. Set them once; `gh`
  reads the value from stdin, so nothing lands in shell history:

  ```
  gh secret set TAURI_SIGNING_PRIVATE_KEY --repo Hellsparks/Foreman < ~/.tauri/foreman-updater.key
  gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo Hellsparks/Foreman < ~/.tauri/foreman-updater.key.password
  ```

  PowerShell: `Get-Content -Raw $HOME\.tauri\foreman-updater.key | gh secret set TAURI_SIGNING_PRIVATE_KEY --repo Hellsparks/Foreman`
  (same for the password file).
- Regenerate (only if lost/compromised; then update `pubkey` and both secrets):
  `npm --prefix frontend exec -- tauri signer generate --ci -f -w ~/.tauri/foreman-updater.key -p <passphrase>`

## Releases (maintainer)

Pushing a tag `v*` runs `.github/workflows/release.yml` and publishes a GitHub Release
with everything below. A tag with a `-` (`v0.4.0-rc.1`) publishes as a **pre-release**:
not "latest", so stable installs ignore it; beta-channel installs get it.

The version comes from the tag (`cargo set-version` writes it to the workspace,
the desktop app and the frontend before building). The release body is the tag's
annotation (your summary) followed by the commits since the previous tag, the
downloads table and checksum instructions.

```
git switch main
git pull
git tag -a v0.4.0-rc.1 -m "Summary of this release, shown at the top of the notes and in the app's update dialog."
git push origin v0.4.0-rc.1
```

Dry run (builds everything, publishes nothing, uploads a `release-preview` artifact
with every file, `latest.json` and the body): **Actions → Release → Run workflow**
with *publish* off, or `gh workflow run release.yml -f version=0.4.0-rc.1`.

| File | What |
|---|---|
| `Foreman_<v>_windows_x64-setup.exe` | Windows installer (NSIS) |
| `Foreman_<v>_linux_{x86_64,aarch64}.AppImage`, `_{amd64,arm64}.deb`, `_{x86_64,aarch64}.rpm` | Linux desktop app |
| `*.sig`, `latest.json` | Updater signatures and manifest |
| `foreman-server_<v>_{amd64,arm64}.deb` | Headless server as a systemd service |
| `foreman-server-{x86_64,aarch64}-unknown-linux-gnu.tar.xz` (+ `.sha256`) | Headless server binary + web UI (built with dist) |
| `SHA256SUMS` | Checksums of every file |
| `ghcr.io/hellsparks/foreman:<v>` | Docker image, linux/amd64 + linux/arm64 (`latest` = newest full release) |

The first push creates the GHCR package as private: make it public once under
GitHub → Packages → foreman → Package settings.

## Headless server (Pi, NAS, always-on PC)

`foreman-server` listens on **all interfaces** (`0.0.0.0` and `[::]`, port 9090) by
default, so it is reachable on the LAN and over Tailscale with no extra settings.
Until you create the admin account it answers only `/api/health`, `/api/info` and
`/api/auth/setup`; the setup code is printed in the log. After that every request
needs a login or an API key.

To keep it on this machine only, set `host = "127.0.0.1"` in
`~/.local/share/foreman/config.toml` (Windows: `%APPDATA%\foreman\config.toml`) or
the `FOREMAN__HOST` environment variable.

The desktop app's "This computer" mode is different: it stays on localhost until you
turn on **Settings → Security → Let other devices connect**.

### Debian / Ubuntu / Raspberry Pi OS (.deb)

```
sudo apt install ./foreman-server_<version>_arm64.deb    # or _amd64.deb
journalctl -u foreman                                     # the setup code
```

The package installs the systemd service `foreman` (enabled and started), running as
the system user `foreman` with its data in `/var/lib/foreman` and listening on all
interfaces. Settings go in `/etc/default/foreman` (e.g. `FOREMAN__HOST=127.0.0.1`)
or `/etc/foreman/config.toml`; then `sudo systemctl restart foreman`.
`apt purge foreman-server` also deletes `/var/lib/foreman` and the user.

### Docker

```
docker run -d --name foreman --restart unless-stopped -p 9090:9090 -p 7912:7912 -v foreman-data:/data ghcr.io/hellsparks/foreman:latest
docker logs foreman    # the setup code
```

For mDNS discovery by the desktop app, run with `--network host` instead of `-p`.

### Any Linux (archive)

Unpack `foreman-server-<target>.tar.xz` and keep `foreman-server` and its `frontend/`
folder together (the web UI is served from there, or from `FOREMAN__FRONTEND_DIR`).
A unit file for a hand install:

```ini
[Unit]
Description=Foreman print farm server
After=network-online.target
Wants=network-online.target

[Service]
User=foreman
ExecStart=/opt/foreman/foreman-server
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Then `sudo systemctl enable --now foreman` and `journalctl -u foreman` for the setup
code. No `FOREMAN__HOST` is needed.

### Firewall

Open port **9090** (Foreman) and, if Moonraker or slicers use the built-in Spoolman,
**7912**. Limit them to your LAN, for example with ufw:

```
sudo ufw allow from 10.0.0.0/24 to any port 9090 proto tcp
sudo ufw allow in on tailscale0 to any port 9090 proto tcp
```

Windows: allow `foreman-server.exe` on Private networks when Windows Defender
Firewall asks.

## Backup and restore

Settings → Backup: folder, schedule (off / daily / weekly), how many to keep, Library opt-in, passphrase encryption, Run now, Restore.
Each backup is `foreman-backup-<UTC time>.zip` (or `.zip.age` when encrypted) plus a `.manifest.json` beside it.

**A backup contains `secret.key`**, which decrypts the SSH key and printer passwords stored in Foreman. Keep backups private.

From a terminal (works on Linux and Windows; stop Foreman first for `restore`):

```fish
foreman-server backup                      # into the saved folder
foreman-server backup ~/foreman-backups    # into another folder
foreman-server restore ~/foreman-backups/foreman-backup-20261112-002400Z.zip
```

`restore` saves the current state as `<data dir>/restore-snapshots/foreman-pre-restore-….zip` first; restore that file to undo.

**By hand, if Foreman will not start at all:**

1. Stop Foreman. If the file ends in `.age`, decrypt it: `age -d -o backup.zip foreman-backup-….zip.age` (age or rage, any OS).
2. Unzip it somewhere. It holds `manifest.json`, `foreman.db`, `spoolman.db`, `secret.key`, and maybe `config.toml`, `custom.css`, `fonts/`, `themes/`, `library/`.
3. Into the Foreman data dir (`~/.local/share/foreman` on Linux, `%APPDATA%\foreman` on Windows) copy `foreman.db`, `secret.key` and the rest; delete `foreman.db-wal` and `foreman.db-shm` there first.
4. Copy `spoolman.db` into the Spoolman data dir (`SPOOLMAN_DIR_DATA`, default `~/.local/share/spoolman` or `%APPDATA%\spoolman`); delete `spoolman.db-wal` / `-shm` there first.
5. Start Foreman.

## Linux + NVIDIA notes

WebKitGTK's DMA-BUF renderer can deadlock or freeze on NVIDIA drivers under
Wayland.  The desktop binary sets `WEBKIT_DISABLE_DMABUF_RENDERER=1`
automatically at startup when the variable is not already set.

If you still see a black window or the app hangs after setting that variable,
try these additional environment variables before launching:

```fish
set -x WEBKIT_DISABLE_COMPOSITING_MODE 1
set -x __NV_DISABLE_EXPLICIT_SYNC 1
cargo tauri-dev
```

## Prerequisites

| Tool | Install |
|------|---------|
| Rust stable | `rustup update stable` |
| Node / npm | package manager |
| tauri-cli 2.x | `cargo install tauri-cli --version "^2"` |
| tray (Linux) | distro package (Arch: `libayatana-appindicator`; Ubuntu: `libayatana-appindicator3-dev`) |
| webkit2gtk-4.1 | distro package (Arch: `webkit2gtk-4.1`) |
| gtk3 | distro package (Arch: `gtk3`) |
