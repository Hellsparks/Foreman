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
