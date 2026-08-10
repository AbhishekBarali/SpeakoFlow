#!/usr/bin/env bash
#
# Audit a macOS .app bundle for dependencies it does not actually carry.
#
# WHY THIS EXISTS (issue #19): the Intel macOS job used to verify only the app
# executable's OWN load commands — `otool -L Contents/MacOS/speakoflow` — and
# then declared the bundle sound. That check cannot see one level down. The
# Homebrew libonnxruntime.dylib we bundle brings ~84 absolute /usr/local/opt
# references of its own (onnx, onnx_proto, protobuf-lite, utf8_range, re2 and
# dozens of versioned abseil libs), none of which were copied in. The .dmg
# installed fine, passed CI, and then died in dyld before main() on the first
# real Intel Mac it reached, with:
#
#   Library not loaded: /usr/local/opt/onnx/lib/libonnx.dylib
#   Referenced from: SpeakoFlow.app/Contents/Frameworks/libonnxruntime.dylib
#
# So this walks the WHOLE graph instead: start from everything executable in
# Contents/MacOS plus every dylib in Contents/Frameworks, follow each load
# command, resolve @rpath / @loader_path / @executable_path to the real file
# inside the bundle, and keep walking. Anything that points outside the bundle
# and is not a macOS-guaranteed system library is a build failure, at any depth.
#
# Also asserts the honest minimum macOS version when one is supplied: the real
# floor is whichever bundled Mach-O demands the most, NOT what
# tauri.conf.json's minimumSystemVersion claims. A .dmg that installs on an
# older Mac and then refuses to launch is the same class of bug as the one
# above, just moved from dyld to the OS version check.
#
# Usage: audit-macos-bundle.sh <path-to-.app> [declared-minimum-macos]
#   <path-to-.app>           the built bundle, e.g. .../bundle/macos/SpeakoFlow.app
#   [declared-minimum-macos] optional, e.g. 14.0 — fails if any bundled binary
#                            needs a NEWER macOS than this
#
# macOS only (uses otool/vtool). Written for /bin/bash 3.2, which is what the
# GitHub macOS runners still ship, so: no associative arrays, no mapfile, no
# ${var,,}. The worklist is a plain file for exactly that reason.
set -euo pipefail

APP="${1:?usage: audit-macos-bundle.sh <path-to-.app> [declared-minimum-macos]}"
DECLARED_MIN_OS="${2:-}"

if [ ! -d "$APP" ]; then
  echo "ERROR: not a directory: $APP" >&2
  exit 1
fi

FRAMEWORKS="${APP}/Contents/Frameworks"
MACOS_DIR="${APP}/Contents/MacOS"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
QUEUE="$WORK/queue"
SEEN="$WORK/seen"
: > "$QUEUE"
: > "$SEEN"

violations=0

# Seed the walk with every Mach-O the bundle ships itself.
if [ -d "$MACOS_DIR" ]; then
  find "$MACOS_DIR" -type f -perm -u+x >> "$QUEUE" 2>/dev/null || true
fi
if [ -d "$FRAMEWORKS" ]; then
  find "$FRAMEWORKS" -type f \( -name '*.dylib' -o -perm -u+x \) >> "$QUEUE" 2>/dev/null || true
fi

if [ ! -s "$QUEUE" ]; then
  echo "ERROR: found no Mach-O files to audit under ${APP}" >&2
  exit 1
fi

# Resolve a dyld load path to a concrete file inside the bundle.
#   @rpath           -> Contents/Frameworks (the only rpath src-tauri/build.rs
#                       emits for this target is @executable_path/../Frameworks)
#   @loader_path     -> relative to the file doing the referencing
#   @executable_path -> Contents/MacOS
resolve_ref() {
  ref_from="$1"
  ref_dep="$2"
  case "$ref_dep" in
    @rpath/*)           echo "${FRAMEWORKS}/${ref_dep#@rpath/}" ;;
    @loader_path/*)     echo "$(dirname "$ref_from")/${ref_dep#@loader_path/}" ;;
    @executable_path/*) echo "${MACOS_DIR}/${ref_dep#@executable_path/}" ;;
    *)                  echo "$ref_dep" ;;
  esac
}

echo "--- recursive dependency audit: $(basename "$APP") ---"

while [ -s "$QUEUE" ]; do
  obj="$(head -1 "$QUEUE")"
  tail -n +2 "$QUEUE" > "${QUEUE}.next" && mv "${QUEUE}.next" "$QUEUE"

  # Cheap cycle guard: abseil's libraries reference each other heavily, so
  # without this the walk never terminates.
  if grep -qxF "$obj" "$SEEN" 2>/dev/null; then
    continue
  fi
  echo "$obj" >> "$SEEN"

  if [ ! -r "$obj" ]; then
    echo "ERROR: cannot read ${obj}" >&2
    violations=$((violations + 1))
    continue
  fi

  # Indented lines are the load commands; the un-indented ones are the file
  # name and, for a fat binary, the per-architecture headers.
  otool -L "$obj" 2>/dev/null | grep '^[[:space:]]' | awk '{print $1}' > "$WORK/deps" || true

  while IFS= read -r dep; do
    [ -n "$dep" ] || continue
    case "$dep" in
      # Guaranteed present on every macOS install.
      /usr/lib/*|/System/Library/*)
        continue
        ;;
      @rpath/*|@loader_path/*|@executable_path/*)
        target="$(resolve_ref "$obj" "$dep")"
        if [ ! -f "$target" ]; then
          echo "ERROR: $(basename "$obj") needs ${dep}, which does not exist in the bundle" >&2
          echo "       expected at: ${target}" >&2
          violations=$((violations + 1))
        else
          echo "$target" >> "$QUEUE"
        fi
        ;;
      # Anything else absolute is a build-machine path that will not exist on a
      # user's Mac: /usr/local/..., /opt/homebrew/..., a Cellar path, or the
      # runner's own workspace.
      *)
        echo "ERROR: $(basename "$obj") references a path outside the bundle:" >&2
        echo "       ${dep}" >&2
        violations=$((violations + 1))
        ;;
    esac
  done < "$WORK/deps"
done

audited="$(wc -l < "$SEEN" | tr -d ' ')"
echo "Walked ${audited} Mach-O file(s)."

if [ "$violations" -ne 0 ]; then
  echo "" >&2
  echo "ERROR: ${violations} dependency problem(s) — this bundle would fail to launch." >&2
  echo "Every non-system library must be copied into Contents/Frameworks and have" >&2
  echo "its load paths rewritten (see the ONNX Runtime staging step in build.yml)." >&2
  exit 1
fi

# --- honest minimum macOS -----------------------------------------------------
# vtool reports each Mach-O's LC_BUILD_VERSION minos. The bundle's real floor is
# the highest one present.
echo "--- minimum macOS ---"
: > "$WORK/minos"
while IFS= read -r obj; do
  v="$(vtool -show-build "$obj" 2>/dev/null | awk '/minos/ {print $2; exit}')" || true
  if [ -n "${v:-}" ]; then
    printf '%s\t%s\n' "$v" "$(basename "$obj")" >> "$WORK/minos"
  fi
done < "$SEEN"

if [ ! -s "$WORK/minos" ]; then
  echo "WARNING: could not read a minimum macOS version from any binary" >&2
else
  sort -t. -k1,1n -k2,2n "$WORK/minos" | tail -5
  worst="$(sort -t. -k1,1n -k2,2n "$WORK/minos" | tail -1 | cut -f1)"
  worst_file="$(sort -t. -k1,1n -k2,2n "$WORK/minos" | tail -1 | cut -f2)"
  echo "Real minimum macOS for this bundle: ${worst} (set by ${worst_file})"

  if [ -n "$DECLARED_MIN_OS" ]; then
    # If the highest requirement sorts above the declared floor, the bundle
    # advertises support it does not have.
    highest="$(printf '%s\n%s\n' "$worst" "$DECLARED_MIN_OS" | sort -V | tail -1)"
    if [ "$highest" != "$DECLARED_MIN_OS" ]; then
      echo "ERROR: bundle declares minimumSystemVersion ${DECLARED_MIN_OS} but ${worst_file} needs macOS ${worst}" >&2
      echo "       On an older Mac this .dmg installs and then fails to launch." >&2
      exit 1
    fi
    echo "Declared minimumSystemVersion ${DECLARED_MIN_OS} is consistent with the binaries."
  fi
fi

echo "macOS bundle audit passed"
