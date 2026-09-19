# EmberClick

A fast, lightweight auto clicker for Windows 11 and Linux, written in Rust.

## Features

- Adjustable interval from 0.1 ms to one hour
- Left, right, and middle mouse buttons
- Single or double click mode
- Run until stopped or for a fixed number of actions
- Global **F6** start/stop hotkey (Windows and Linux/X11)
- Responsive click engine on a dedicated worker thread
- Dark black/orange interface

## Run

```sh
cargo run --release
```

The optimized executable will be at `target/release/emberclick.exe` on Windows or
`target/release/emberclick` on Linux.

## Linux notes

The GUI supports both X11 and Wayland. Global hotkeys are supported on X11; on a
Wayland session, F6 remains available while the app is focused because Wayland
compositors intentionally restrict global key capture. Mouse simulation on Wayland
may require your desktop's Remote Desktop/Input Capture permission prompt.

On Debian/Ubuntu, building may require the normal native development packages used
by desktop Rust applications:

```sh
sudo apt install build-essential pkg-config libx11-dev libxi-dev libgl1-mesa-dev \
  libwayland-dev libxkbcommon-dev
```

## Safety

Move focus back to EmberClick and press F6 if another application blocks the global
hotkey. The click worker stops immediately when EmberClick exits.
