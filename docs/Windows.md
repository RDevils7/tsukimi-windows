# Running Tsukimi on Windows

Tsukimi is primarily developed on Linux (Arch), but the application also
builds and runs on Windows. This document describes what the Windows port
uses, how to build it, how to package a portable bundle, and what to expect
at runtime.

## Platform backends

The workspace is platform-agnostic wherever a portable choice exists:

| Subsystem | Linux | Windows |
| --- | --- | --- |
| Video embedding | `MutsumiVideoSink` (wl-proxy dmabuf) | `MPVGLArea` (mpv render API on a GTK `GLArea`) |
| Image loading | glycin | gdk-pixbuf |
| Desktop integration | MPRIS | — (stubbed) |
| Audio (music player) | GStreamer (`playbin3`) | GStreamer (`playbin3`) |
| HTTP client | reqwest + rustls | reqwest + rustls (no OpenSSL required) |
| UI resources / translations / schemas | Meson install into prefix | embedded / `<install-root>/share/…` |

Key implementation notes:

- `mutsumi` selects its video backend at compile time: the wl-proxy/dmabuf
  sink is Linux-only; every other platform gets the GLArea + libmpv render
  API backend (`vo=libmpv` with advanced control enabled).
- `crates/tsukimi/build.rs` compiles `resources.gresource.xml`, the
  `po/*.po` message catalogs and the GSettings schema into `OUT_DIR` and
  embeds the application manifest + icon via `embed-resource`.
- `crates/tsukimi/src/lib.rs` bootstraps the Windows runtime: it points
  `GSETTINGS_SCHEMA_DIR` at the compiled schemas (bundled or built) and
  selects the `keyfile` backend so preferences persist in the user config
  directory.

## Building

Requirements:

- [MSYS2](https://www.msys2.org/) with the `UCRT64` environment
  (`C:\msys64\ucrt64.exe`). Everything below runs inside that shell.
  (MSYS2 deprecated the old `MINGW64` environment in March 2026 — new
  packages are only published for UCRT64/CLANG64.)
- A `x86_64-pc-windows-gnu` Rust toolchain via
  [rustup](https://rustup.rs/) (the MSYS2 CRT must match — do **not** mix
  the `pc-windows-msvc` toolchain with the UCRT64 libraries).

Install the dependencies:

```bash
pacman -S --needed \
  mingw-w64-ucrt-x86_64-toolchain \
  mingw-w64-ucrt-x86_64-pkgconf \
  mingw-w64-ucrt-x86_64-gtk4 \
  mingw-w64-ucrt-x86_64-libadwaita \
  mingw-w64-ucrt-x86_64-mpv \
  mingw-w64-ucrt-x86_64-libepoxy \
  mingw-w64-ucrt-x86_64-adwaita-icon-theme \
  mingw-w64-ucrt-x86_64-gettext \
  mingw-w64-ucrt-x86_64-blueprint-compiler \
  mingw-w64-ucrt-x86_64-gstreamer \
  mingw-w64-ucrt-x86_64-gst-plugins-base \
  mingw-w64-ucrt-x86_64-gst-plugins-good \
  mingw-w64-ucrt-x86_64-gst-plugins-bad \
  mingw-w64-ucrt-x86_64-gst-plugins-ugly \
  mingw-w64-ucrt-x86_64-gst-libav \
  mingw-w64-ucrt-x86_64-nsis \
  zip
```

`blueprint-compiler` is required by `crates/mutsumi/build.rs` to compile
the `.blp` templates at build time; install it from MSYS2. The MSYS2
package ships it as an extensionless Python script
(`/ucrt64/bin/blueprint-compiler`), which `CreateProcess` cannot execute
directly — Rust's `Command` only probes `*.exe` names — so `build.rs` runs
the bundled `blueprintcompiler` module through the Python interpreter
installed next to it (the same interpreter the script's shebang points
at). A pip-installed `blueprint-compiler` is deliberately not used: it is
not on the MSYS2 `PATH` and would additionally require PyGObject.

```bash
rustup toolchain install stable-x86_64-pc-windows-gnu
rustup default stable-x86_64-pc-windows-gnu
```

Build and run from the repository root:

```bash
# -L native lets the linker find libmpv and friends in /ucrt64/lib
RUSTFLAGS="-L native=/ucrt64/lib" \
  cargo run --release --target x86_64-pc-windows-gnu
```

When run straight from `cargo`, resources are embedded in the binary,
translations and schemas come from the build output directory, and GSettings
persists to `%APPDATA%\gtk-4.0`-adjacent keyfiles — no extra setup is needed.

## Packaging a portable bundle

After a release build, run:

```bash
bash build-aux/windows/package.sh x86_64-pc-windows-gnu dist
```

This produces `dist/tsukimi-x86_64-windows.zip` containing a relocatable
directory tree:

```
tsukimi-x86_64-windows/
├── bin/                 tsukimi.exe + full DLL closure + gspawn/gdbus helpers
├── etc/fonts/           fontconfig configuration (mpv/librsvg)
├── lib/
│   ├── gdk-pixbuf-2.0/2.10.0/loaders/   image loaders + relocatable cache
│   ├── gio/modules/                     GIO extension modules
│   └── gstreamer-1.0/                   GStreamer plugins (music playback)
├── share/
│   ├── glib-2.0/schemas/                gschemas.compiled
│   ├── icons/                           Adwaita + hicolor icon themes
│   └── locale/                          compiled tsukimi .mo catalogs
└── tsukimi.ico
```

How the paths resolve (all relative to the executable, via GLib's relocatable
lookup):

- **DLLs** live in `bin/` next to the executable.
- **GIO modules** are found at `<root>/lib/gio/modules`.
- **gdk-pixbuf loaders** are found at `<root>/lib/gdk-pixbuf-2.0/2.10.0/loaders`.
  The `loaders.cache` is regenerated from *inside* the bundle
  (`gdk-pixbuf-query-loaders.exe` is run from `<root>/bin/`), which rewrites
  every entry as a path relative to `<root>` — the bundle stays valid no
  matter where it is copied to.
- **GStreamer plugins** are found at `<root>/lib/gstreamer-1.0`.
- **GSettings schemas** are found at `<root>/share/glib-2.0/schemas`.
- **Translations** are found at `<root>/share/locale`.

The zip can be unpacked anywhere and started with `bin\tsukimi.exe`.

### Installer

An optional NSIS installer (per-user, no administrator rights required) is
built with:

```bash
VERSION="$(grep -m1 '^version' crates/tsukimi/Cargo.toml | cut -d '"' -f 2)"
makensis "-DAPP_VERSION=$VERSION" installer/tsukimi.nsi
```

It installs the bundle into `%LOCALAPPDATA%\Tsukimi`, creates Start Menu and
desktop shortcuts, and registers an uninstaller entry. The setup executable
is written to the repository root as `tsukimi-x86_64-windows-setup.exe`
(makensis resolves the script's relative paths against `installer/`, the
directory holding the `.nsi` file, not against the invocation directory).

## Runtime notes and troubleshooting

- **Renderer**: GSK defaults to the Cairo renderer on Windows. If you see
  rendering glitches (or want GPU compositing), start the app with
  `GSK_RENDERER=gl` (or `GSK_RENDERER=cairo` to force the opposite).
- **Video playback**: video is embedded through the mpv render API on a GTK
  `GLArea`, which requires a working OpenGL driver (all modern GPUs provide
  one). On systems where GL is unavailable the video area stays black while
  audio continues to play. If you hit GL driver bugs, try a different
  hardware-decoding mode in `%APPDATA%\mpv\mpv.conf` (`hwdec=auto`,
  `hwdec=no`, or `gpu-context` variations) before disabling GL entirely.
- **Music playback** uses GStreamer; the bundle ships the base/good/bad/ugly
  and libav plugin sets. Missing codec plugins show up as GStreamer
  "missing plugin" errors in the log (`RUST_LOG=debug`).
- **MPV config**: mpv reads its configuration from `%APPDATA%\mpv\`
  (`mpv.conf`, `input.conf`), as documented in the
  [mpv manual](https://mpv.io/manual/master/#files).
- **Settings storage**: GSettings uses the keyfile backend, so preferences
  persist in `%APPDATA%\glib-2.0\settings\keyfile` (GLib's user config
  directory).
- **Translations of GTK/GLib themselves** are not bundled; only tsukimi's
  own message catalogs are shipped, so some system dialogs may appear in
  English.

## Continuous integration

`.github/workflows/build_windows.yml` runs the same steps on
`windows-latest` (MSYS2 UCRT64 + `stable-x86_64-pc-windows-gnu`), builds
the portable zip and the NSIS installer, and uploads both as the
`tsukimi-x86_64-windows` artifact.
