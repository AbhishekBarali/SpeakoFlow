# Build Instructions

This guide covers how to set up the development environment and build SpeakoFlow from source across different platforms.

## Prerequisites

### All Platforms

- [Rust](https://rustup.rs/) (latest stable)
- [Bun](https://bun.sh/) package manager
- [Tauri Prerequisites](https://tauri.app/start/prerequisites/)

### Platform-Specific Requirements

#### macOS

- Xcode Command Line Tools
- Install with: `xcode-select --install`

##### Intel Mac (x86_64)

Intel Macs build **CPU-only**. `Cargo.toml` arch-gates the Metal GPU backend to
Apple Silicon, because ggml's Metal backend targets Apple Silicon and upstream
whisper.cpp/llama.cpp carry open reports of it (and MoltenVK-Vulkan) producing
corrupted output or failing to build on Intel Macs. Transcription still works;
small Whisper models are usable, large ones are slow.

Intel Macs also need one extra step. Prebuilt ONNX Runtime binaries are not
available for `x86_64-apple-darwin`, because pykeio's `ort` dropped that target, so
ONNX Runtime has to be installed separately and linked dynamically:

```bash
brew install onnxruntime
ORT_LIB_LOCATION=$(brew --prefix onnxruntime)/lib ORT_PREFER_DYNAMIC_LINK=1 bun run tauri dev
```

The same environment variables apply for production builds:

```bash
ORT_LIB_LOCATION=$(brew --prefix onnxruntime)/lib ORT_PREFER_DYNAMIC_LINK=1 bun run tauri build
```

> **This produces an app that only runs on your machine.** Homebrew's dylib
> records an absolute path into your Cellar as its install name, so the `.app`
> will fail to start on any Mac without that exact formula installed. That is
> fine for building for yourself, and it is why CI does it differently: the
> "Stage ONNX Runtime for Intel macOS" step in
> [.github/workflows/build.yml](.github/workflows/build.yml) copies the dylib
> into `src-tauri/onnx-libs/`, rewrites its install name to `@rpath/...` _before_
> linking, and ships it inside `SpeakoFlow.app/Contents/Frameworks`. Don't
> distribute a locally built Intel `.dmg`; use a CI artifact.

Intel macOS is currently built in `test-build.yml` only, and is not yet part of
any release. See [issue #19](https://github.com/AbhishekBarali/SpeakoFlow/issues/19).

#### Windows

- Microsoft C++ Build Tools
- Visual Studio 2019/2022 with C++ development tools
- Or Visual Studio Build Tools 2019/2022

#### Linux

- Build essentials
- ALSA development libraries
- Install with:

  ```bash
  # Ubuntu/Debian
  sudo apt update
  sudo apt install build-essential libasound2-dev pkg-config libssl-dev libvulkan-dev vulkan-tools glslc libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libgtk-layer-shell0 libgtk-layer-shell-dev patchelf cmake

  # Fedora/RHEL
  sudo dnf groupinstall "Development Tools"
  sudo dnf install alsa-lib-devel pkgconf openssl-devel vulkan-devel \
    gtk3-devel webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel \
    gtk-layer-shell gtk-layer-shell-devel \
    cmake

  # Arch Linux
  sudo pacman -S base-devel alsa-lib pkgconf openssl vulkan-devel \
    gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg gtk-layer-shell \
    cmake
  ```

## Setup Instructions

### 1. Clone the Repository

```bash
git clone git@github.com:AbhishekBarali/SpeakoFlow.git
cd SpeakoFlow
```

### 2. Install Dependencies

```bash
bun install
```

### 3. Start Dev Server

```bash
bun tauri dev
```

### 4. Build for Production

```bash
bun run tauri build
```

This compiles a release binary and generates platform-specific bundles (deb, rpm, AppImage on Linux; dmg on macOS; msi on Windows).

## Linux Install (from source)

The raw binary (`src-tauri/target/release/speakoflow`) cannot run standalone — it needs Tauri resource files (tray icons, sounds, VAD model) to be co-located at the expected path.

**Install from the deb bundle** (works on any Linux distro):

```bash
cd /tmp
ar x /path/to/SpeakoFlow/src-tauri/target/release/bundle/deb/SpeakoFlow_*_amd64.deb data.tar.gz
tar xzf data.tar.gz
sudo cp usr/bin/speakoflow /usr/bin/
sudo cp -r usr/lib/SpeakoFlow /usr/lib/
sudo cp -r usr/share/icons/hicolor/* /usr/share/icons/hicolor/
sudo cp usr/share/applications/SpeakoFlow.desktop /usr/share/applications/
```

After subsequent rebuilds, only the binary needs re-copying:

```bash
sudo cp src-tauri/target/release/speakoflow /usr/bin/
```

Resources only need re-copying if they change upstream (new icons, sounds, etc.).

## Troubleshooting

### AppImage build fails on Arch / rolling-release distros

`linuxdeploy` bundles its own `strip` binary which is too old to process system libraries built with newer toolchains on rolling-release distros (Arch, CachyOS, Manjaro, EndeavourOS).

The error from Tauri:

```
Bundling SpeakoFlow_*_amd64.AppImage
failed to bundle project `failed to run linuxdeploy`
```

Tauri swallows the real linuxdeploy error. To see it, run linuxdeploy manually:

```bash
cd src-tauri/target/release/bundle/appimage
~/.cache/tauri/linuxdeploy-x86_64.AppImage --appimage-extract-and-run \
  --appdir SpeakoFlow.AppDir --plugin gtk --output appimage
```

**Workaround:** The binary, deb, and rpm bundles all build fine — only the AppImage step fails. To skip it:

```bash
bun run tauri build -- --bundles deb
```

Then install using the deb extraction method above.
