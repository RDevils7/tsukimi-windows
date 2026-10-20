#!/usr/bin/env bash
# Build a portable Windows bundle for tsukimi from a UCRT64 release build.
#
# Usage:
#   bash build-aux/windows/package.sh [TARGET] [OUTPUT_DIR]
#
#   TARGET      Rust target triple (default: x86_64-pc-windows-gnu)
#   OUTPUT_DIR  Where the final zip is placed (default: dist)
#
# Environment overrides:
#   MINGW_PREFIX     MSYS2 UCRT64 prefix (default: /ucrt64)
#   CARGO_TARGET_DIR Cargo target directory (default: target)
#   STAGE_NAME       Name of the bundled top-level directory
#                    (default: tsukimi-x86_64-windows)
#
# The bundle layout matches what the runtime expects (see
# crates/tsukimi/src/lib.rs):
#
#   tsukimi-x86_64-windows/
#   ├── bin/            tsukimi.exe, its DLL closure, GLib helper executables
#   ├── etc/fonts/      fontconfig configuration (pulled in by mpv/librsvg)
#   ├── lib/
#   │   ├── gdk-pixbuf-2.0/2.10.0/loaders/   + a relocatable loaders.cache
#   │   ├── gio/modules/                      GIO extension modules
#   │   └── gstreamer-1.0/                    GStreamer plugins (music player)
#   └── share/
#       ├── glib-2.0/schemas/                 gschemas.compiled
#       ├── icons/                            Adwaita + hicolor icon themes
#       └── locale/                           compiled tsukimi .mo catalogs
#
# All shared-library paths are discovered relative to the executable (GLib's
# relocatable lookup), so the directory can be copied or zipped anywhere.

set -euo pipefail

TARGET="${1:-x86_64-pc-windows-gnu}"
OUTPUT_DIR="${2:-dist}"
MINGW_PREFIX="${MINGW_PREFIX:-/ucrt64}"
TARGET_DIR="${CARGO_TARGET_DIR:-target}"
STAGE_NAME="${STAGE_NAME:-tsukimi-x86_64-windows}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
STAGE="$REPO_ROOT/$STAGE_NAME"
BIN="$STAGE/bin"
PIXBUF_LOADERS="$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders"
EXE="$REPO_ROOT/$TARGET_DIR/$TARGET/release/tsukimi.exe"

log() { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mwarning:\033[0m %s\n' "$*" >&2; }
die() {
  printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
  exit 1
}

[ -f "$EXE" ] || die "executable not found: $EXE (run cargo build --release --target $TARGET first)"
[ -d "$MINGW_PREFIX" ] || die "MSYS2 prefix not found: $MINGW_PREFIX"

# ---------------------------------------------------------------------------
# Staging layout
# ---------------------------------------------------------------------------
log "Creating staging directory $STAGE_NAME"
rm -rf "$STAGE"
mkdir -p "$BIN"
mkdir -p "$STAGE/etc/fonts"
mkdir -p "$STAGE/lib/gio/modules"
mkdir -p "$PIXBUF_LOADERS"
mkdir -p "$STAGE/lib/gstreamer-1.0"
mkdir -p "$STAGE/share/glib-2.0/schemas"
mkdir -p "$STAGE/share/locale"
mkdir -p "$STAGE/share/icons"

cp "$EXE" "$BIN/tsukimi.exe"

# ---------------------------------------------------------------------------
# Dynamically loaded modules (not part of the exe's link closure)
# ---------------------------------------------------------------------------

# GIO extension modules (TLS backends, network monitor, volume monitor, ...).
# GLib locates them at <install-root>/lib/gio/modules.
if [ -d "$MINGW_PREFIX/lib/gio/modules" ]; then
  cp -r "$MINGW_PREFIX/lib/gio/modules/." "$STAGE/lib/gio/modules/"
else
  warn "no GIO modules found in $MINGW_PREFIX/lib/gio/modules"
fi

# gdk-pixbuf image loaders (PNG/JPEG/SVG/...). GLib locates them at
# <install-root>/lib/gdk-pixbuf-2.0/2.10.0/loaders.
if [ -d "$MINGW_PREFIX/lib/gdk-pixbuf-2.0/2.10.0/loaders" ]; then
  cp "$MINGW_PREFIX/lib/gdk-pixbuf-2.0/2.10.0/loaders/"*.dll "$PIXBUF_LOADERS/"
else
  warn "no gdk-pixbuf loaders found in $MINGW_PREFIX"
fi

# GStreamer plugins for the music player (playbin3 -> uridecodebin -> audio
# sink). GStreamer locates them at <install-root>/lib/gstreamer-1.0.
if [ -d "$MINGW_PREFIX/lib/gstreamer-1.0" ]; then
  cp "$MINGW_PREFIX/lib/gstreamer-1.0/"*.dll "$STAGE/lib/gstreamer-1.0/"
else
  warn "no GStreamer plugins found in $MINGW_PREFIX"
fi

# ---------------------------------------------------------------------------
# DLL closure
# ---------------------------------------------------------------------------
# Copy every DLL the bundle needs into bin/. The closure must be computed over
# the executable *and* every staged DLL: GStreamer plugins, GIO modules and
# gdk-pixbuf loaders are dlopen()ed at runtime and pull in their own
# dependencies (libavcodec, librsvg, libsoup, ...).

log "Resolving DLL closure"
collect_dlls() {
  local changed=1
  local file dll name
  while [ "$changed" -eq 1 ]; do
    changed=0
    while IFS= read -r file; do
      [ -f "$file" ] || continue
      while IFS= read -r dll; do
        name="${dll##*/}"
        if [ ! -e "$BIN/$name" ] && cp "$dll" "$BIN/$name" 2>/dev/null; then
          changed=1
        fi
      done < <(ldd "$file" 2>/dev/null | grep -oE "$MINGW_PREFIX/[^ ]+\.dll" | sort -u || true)
    done < <(find "$STAGE" \( -name '*.exe' -o -name '*.dll' \) | sort)
  done
}
collect_dlls
log "Collected $(find "$BIN" -name '*.dll' | wc -l) DLLs"

# GLib spawns these helpers for subprocess handling and D-Bus tooling; they
# must sit next to the GLib DLLs (i.e. in bin/).
for helper in gspawn-win64-helper.exe gdbus.exe; do
  if [ -f "$MINGW_PREFIX/bin/$helper" ]; then
    cp "$MINGW_PREFIX/bin/$helper" "$BIN/"
  else
    warn "$helper not found in $MINGW_PREFIX/bin"
  fi
done

# ---------------------------------------------------------------------------
# gdk-pixbuf loaders.cache (regenerated with relocatable, relative paths)
# ---------------------------------------------------------------------------
# The cache shipped by MSYS2 contains absolute paths. Because the Windows
# build of gdk-pixbuf is always relocatable, running the query tool from
# inside the staged bundle rewrites every entry relative to the bundle root.

log "Regenerating gdk-pixbuf loaders.cache"
QUERY_LOADERS="$MINGW_PREFIX/bin/gdk-pixbuf-query-loaders.exe"
if [ -f "$QUERY_LOADERS" ]; then
  cp "$QUERY_LOADERS" "$BIN/"
  # Run it from the bundle root so that its notion of the installation
  # directory is the bundle itself (the trailing bin/ is stripped).
  (cd "$STAGE" && ./bin/gdk-pixbuf-query-loaders.exe > "$PIXBUF_LOADERS/../loaders.cache")
  if grep -q "$MINGW_PREFIX" "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"; then
    die "loaders.cache still contains absolute paths; the bundle would not be portable"
  fi
else
  warn "gdk-pixbuf-query-loaders.exe not found; shipping the system cache instead"
  cp "$MINGW_PREFIX/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/" 2>/dev/null ||
    warn "no loaders.cache available; image loading will fail at runtime"
fi

# ---------------------------------------------------------------------------
# GSettings schemas
# ---------------------------------------------------------------------------
# bootstrap_windows_runtime() points GSETTINGS_SCHEMA_DIR at
# <install-root>/share/glib-2.0/schemas when it contains gschemas.compiled.
log "Compiling GSettings schemas"
SCHEMA_SRC="$REPO_ROOT/crates/tsukimi/resources/moe.tsuna.tsukimi.gschema.xml"
if [ -f "$SCHEMA_SRC" ]; then
  cp "$SCHEMA_SRC" "$STAGE/share/glib-2.0/schemas/"
  glib-compile-schemas "$STAGE/share/glib-2.0/schemas"
else
  die "schema source not found: $SCHEMA_SRC"
fi

# ---------------------------------------------------------------------------
# Translations
# ---------------------------------------------------------------------------
# The build script compiles po/*.po into OUT_DIR/locale; locale_dir() prefers
# <install-root>/share/locale over that baked-in build path.
log "Copying message catalogs"
BUILD_LOCALE="$(find "$REPO_ROOT/$TARGET_DIR/$TARGET/release/build" \
  -maxdepth 3 -path '*/out/locale' -type d 2>/dev/null | sort | head -n 1)"
if [ -n "$BUILD_LOCALE" ]; then
  cp -r "$BUILD_LOCALE/." "$STAGE/share/locale/"
  log "Installed $(find "$STAGE/share/locale" -name '*.mo' | wc -l) message catalogs"
else
  warn "compiled catalogs not found under $TARGET_DIR/$TARGET/release/build/*/out/locale"
fi

# ---------------------------------------------------------------------------
# Icon themes
# ---------------------------------------------------------------------------
log "Copying icon themes"
for theme in Adwaita hicolor; do
  if [ -d "$MINGW_PREFIX/share/icons/$theme" ]; then
    cp -r "$MINGW_PREFIX/share/icons/$theme" "$STAGE/share/icons/"
  else
    warn "icon theme $theme not found"
  fi
done

# The application icon, so the icon theme can resolve moe.tsuna.tsukimi.
HICOLOR_APPS="$STAGE/share/icons/hicolor/scalable/apps"
if [ -d "$HICOLOR_APPS" ]; then
  cp "$REPO_ROOT/crates/tsukimi/resources/icons/moe.tsuna.tsukimi.svg" "$HICOLOR_APPS/"
fi

for theme in Adwaita hicolor; do
  if [ -f "$STAGE/share/icons/$theme/index.theme" ]; then
    gtk-update-icon-cache --force --quiet "$STAGE/share/icons/$theme" ||
      warn "gtk-update-icon-cache failed for $theme (not fatal)"
  fi
done

# ---------------------------------------------------------------------------
# fontconfig configuration
# ---------------------------------------------------------------------------
# mpv (and possibly librsvg) link fontconfig, which resolves its configuration
# relative to the DLL installation directory: <install-root>/etc/fonts.
if [ -f "$MINGW_PREFIX/etc/fonts/fonts.conf" ]; then
  cp "$MINGW_PREFIX/etc/fonts/fonts.conf" "$STAGE/etc/fonts/"
  if find "$BIN" -iname '*fontconfig*' | grep -q .; then
    log "fontconfig detected; shipped etc/fonts/fonts.conf"
  fi
else
  warn "no fontconfig configuration found in $MINGW_PREFIX/etc/fonts"
fi

# ---------------------------------------------------------------------------
# Application icon (for the installer / shortcuts)
# ---------------------------------------------------------------------------
cp "$REPO_ROOT/crates/tsukimi/resources/icons/tsukimi.ico" "$STAGE/tsukimi.ico"

# ---------------------------------------------------------------------------
# Zip
# ---------------------------------------------------------------------------
log "Creating archive"
mkdir -p "$REPO_ROOT/$OUTPUT_DIR"
ARCHIVE="$REPO_ROOT/tsukimi-x86_64-windows.zip"
rm -f "$ARCHIVE"
(cd "$REPO_ROOT" && zip -r -9 -q "$ARCHIVE" "$STAGE_NAME")
mv "$ARCHIVE" "$REPO_ROOT/$OUTPUT_DIR/"

# ---------------------------------------------------------------------------
# Sanity checks
# ---------------------------------------------------------------------------
log "Verifying bundle"
fail=0
check() {
  if [ ! -e "$2" ]; then
    warn "missing: $2"
    fail=1
  fi
}
check "executable" "$BIN/tsukimi.exe"
check "loaders.cache" "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
check "gschemas.compiled" "$STAGE/share/glib-2.0/schemas/gschemas.compiled"
check "app icon" "$STAGE/share/icons/hicolor/scalable/apps/moe.tsuna.tsukimi.svg"
check "gspawn helper" "$BIN/gspawn-win64-helper.exe"
if ! grep -q "$MINGW_PREFIX" "$STAGE/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" 2>/dev/null; then
  :
else
  warn "loaders.cache contains absolute paths"
  fail=1
fi
if [ "$(find "$STAGE/share/locale" -name '*.mo' | wc -l)" -eq 0 ]; then
  warn "no message catalogs were staged"
fi
if [ "$(find "$STAGE/lib/gstreamer-1.0" -name '*.dll' | wc -l)" -eq 0 ]; then
  warn "no GStreamer plugins were staged; music playback will not work"
fi
[ "$fail" -eq 0 ] || die "bundle verification failed"

log "Bundle ready: $REPO_ROOT/$OUTPUT_DIR/$STAGE_NAME.zip"
du -sh "$STAGE" "$REPO_ROOT/$OUTPUT_DIR/$STAGE_NAME.zip"
