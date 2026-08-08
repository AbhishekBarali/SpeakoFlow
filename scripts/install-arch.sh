#!/usr/bin/env bash

# Build SpeakoFlow from this checkout and install it for the current user.
# The default layout is self-contained under ~/.local:
#
#   bin/speak -> ../lib/speakoflow/bin/speakoflow
#   lib/speakoflow/lib/{libtranscribe,libggml*,SpeakoFlow/resources}
#
# Keeping the executable and libraries in that shape satisfies the
# $ORIGIN/../lib rpath embedded by src-tauri/build.rs without putting generic
# libggml names into the system-wide library directory.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-arch.sh [options]

Options:
  --prefix PATH  Install below PATH instead of the current user's ~/.local
  --skip-deps    Do not check or install Arch build dependencies
  --skip-build   Install an existing release build without rebuilding it
  -h, --help     Show this help

After installation, run SpeakoFlow with: speak
EOF
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"
user_home_dir="$(getent passwd "$(id -un)" | cut -d: -f6)"
if [[ -z "$user_home_dir" || "$user_home_dir" == / ]]; then
  echo "ERROR: could not resolve a safe home directory for the current user" >&2
  exit 1
fi
install_prefix="$user_home_dir/.local"
check_dependencies=true
build_release=true

while (($#)); do
  case "$1" in
    --prefix)
      if (($# < 2)); then
        echo "ERROR: --prefix requires a path" >&2
        exit 2
      fi
      install_prefix="${2%/}"
      shift 2
      ;;
    --skip-deps)
      check_dependencies=false
      shift
      ;;
    --skip-build)
      build_release=false
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ -z "$install_prefix" || "$install_prefix" != /* || "$install_prefix" == / ]]; then
  echo "ERROR: the install prefix must be a safe absolute path other than /" >&2
  exit 2
fi

if [[ "$(uname -s)" != Linux ]] || ! command -v pacman >/dev/null 2>&1; then
  echo "ERROR: this installer supports Arch Linux and Arch-based distributions" >&2
  exit 1
fi

arch_dependencies=(
  alsa-lib
  base-devel
  bun
  cmake
  glslang
  gtk-layer-shell
  gtk3
  libappindicator
  libdrm
  libglvnd
  librsvg
  libx11
  libxrandr
  libxtst
  openssl
  patchelf
  pipewire
  pkgconf
  rustup
  shaderc
  spirv-headers
  vulkan-headers
  vulkan-icd-loader
  wayland
  webkit2gtk-4.1
)

if $check_dependencies; then
  mapfile -t missing_dependencies < <(pacman -T "${arch_dependencies[@]}" 2>/dev/null || true)
  if ((${#missing_dependencies[@]})); then
    echo "Installing missing Arch dependencies: ${missing_dependencies[*]}"
    sudo pacman -S --needed -- "${missing_dependencies[@]}"
  fi
fi

for command_name in bun cargo cmake pkg-config; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "ERROR: required build command is unavailable: $command_name" >&2
    exit 1
  fi
done

vad_model="$repo_root/src-tauri/resources/models/silero_vad_v4.onnx"
if [[ ! -s "$vad_model" ]]; then
  echo "ERROR: missing VAD model: $vad_model" >&2
  echo "Download it with the model setup command documented in BUILD.md." >&2
  exit 1
fi

release_binary="$repo_root/src-tauri/target/release/speakoflow"
if $build_release; then
  echo "Building the SpeakoFlow frontend..."
  (
    cd "$repo_root"
    bun_scratch_dir="$(mktemp -d /tmp/speakoflow-bun.XXXXXX)"
    trap 'rm -rf -- "$bun_scratch_dir"' EXIT
    mkdir -p "$bun_scratch_dir/install" "$bun_scratch_dir/tmp"
    BUN_INSTALL="$bun_scratch_dir/install" \
      BUN_TMPDIR="$bun_scratch_dir/tmp" \
      bun install --frozen-lockfile --ignore-scripts
    bun run build
  )

  echo "Building the SpeakoFlow release binary..."
  (
    cd "$repo_root/src-tauri"
    # CMake 4 removed compatibility with policies used by a few native speech
    # dependencies. This keeps those dependencies buildable on rolling Arch.
    # `custom-protocol` embeds frontendDist instead of loading build.devUrl.
    CMAKE_POLICY_VERSION_MINIMUM=3.5 \
      cargo build --release --locked --features custom-protocol
  )
fi

if [[ ! -x "$release_binary" ]]; then
  echo "ERROR: release binary not found: $release_binary" >&2
  exit 1
fi

transcribe_lib_dir="$repo_root/src-tauri/transcribe-libs"
if [[ ! -d "$transcribe_lib_dir" ]]; then
  echo "ERROR: staged speech-engine libraries not found: $transcribe_lib_dir" >&2
  exit 1
fi

shopt -s nullglob
transcribe_libraries=("$transcribe_lib_dir"/libtranscribe.so*)
cpu_backends=("$transcribe_lib_dir"/libggml-cpu*.so*)
shopt -u nullglob
if ((${#transcribe_libraries[@]} == 0 || ${#cpu_backends[@]} == 0)); then
  echo "ERROR: the release build did not stage a complete speech engine" >&2
  ls -la "$transcribe_lib_dir" >&2
  exit 1
fi

app_root="$install_prefix/lib/speakoflow"
app_bin_dir="$app_root/bin"
app_lib_dir="$app_root/lib"
resource_dir="$app_lib_dir/SpeakoFlow"

echo "Installing SpeakoFlow below $install_prefix..."
install -Dm755 "$release_binary" "$app_bin_dir/speakoflow"
install -d "$app_lib_dir" "$resource_dir" "$install_prefix/bin"
bash "$repo_root/scripts/ci/stage-transcribe-libs.sh" "$transcribe_lib_dir" "$app_lib_dir"
cp -a "$repo_root/src-tauri/resources" "$resource_dir/"

ln -sfn ../lib/speakoflow/bin/speakoflow "$install_prefix/bin/speak"
ln -sfn ../lib/speakoflow/bin/speakoflow "$install_prefix/bin/speakoflow"

desktop_dir="$install_prefix/share/applications"
icon_root="$install_prefix/share/icons/hicolor"
install -Dm644 "$repo_root/packaging/linux/speakoflow.desktop" \
  "$desktop_dir/speakoflow.desktop"
install -Dm644 "$repo_root/src-tauri/icons/32x32.png" \
  "$icon_root/32x32/apps/speakoflow.png"
install -Dm644 "$repo_root/src-tauri/icons/128x128.png" \
  "$icon_root/128x128/apps/speakoflow.png"
install -Dm644 "$repo_root/src-tauri/icons/128x128@2x.png" \
  "$icon_root/256x256/apps/speakoflow.png"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t "$icon_root" >/dev/null 2>&1 || true
fi

case ":$PATH:" in
  *":$install_prefix/bin:"*) ;;
  *)
    echo
    echo "Add $install_prefix/bin to PATH before using the terminal command."
    echo "For Bash:  export PATH=\"$install_prefix/bin:\$PATH\""
    ;;
esac

echo
echo "SpeakoFlow installed successfully. Run it with: speak"
