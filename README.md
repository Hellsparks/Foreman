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

### Updater signing key (maintainer)

Update bundles must be signed; the app only accepts files signed by the key whose
public half is in `crates/foreman-desktop/tauri.conf.json` (`plugins.updater.pubkey`).

- Private key: `~/.tauri/foreman-updater.key` on the maintainer's machine (no
  password). **Never commit it.** Keep an offline copy in your password manager — if
  it is lost, installed apps can never update again (a new key needs a manual reinstall).
- For release builds (CI), store it as repository secrets:
  `TAURI_SIGNING_PRIVATE_KEY` = the file's content, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
  = empty. Tauri reads these when `bundle.createUpdaterArtifacts` is on.
- Regenerate (only if lost/compromised; then update `pubkey`):
  `npm --prefix frontend exec tauri -- signer generate -w ~/.tauri/foreman-updater.key`

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

### systemd (Linux)

`/etc/systemd/system/foreman.service`:

```ini
[Unit]
Description=Foreman print farm server
After=network-online.target
Wants=network-online.target

[Service]
User=foreman
ExecStart=/usr/local/bin/foreman-server
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
