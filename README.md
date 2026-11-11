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
| webkit2gtk-4.1 | distro package (Arch: `webkit2gtk-4.1`) |
| gtk3 | distro package (Arch: `gtk3`) |
