#[cfg(target_os = "windows")]
use std::path::Path;
use std::{
    env,
    sync::LazyLock,
};

mod app;
mod arg;
mod config;
mod gstl;
mod macros;

#[cfg(target_os = "linux")]
mod mpris_common;
mod ui;
mod utils;

pub mod client;

pub use arg::Args;
use clap::Parser;
pub use config::*;
use gettextrs::*;
use gtk::prelude::*;

pub use ui::Window;

pub use app::TsukimiApplication as Application;

use crate::client::runtime::runtime;

pub static USER_AGENT: LazyLock<String> =
    LazyLock::new(|| format!("{}/{} - {}", CLIENT_ID, version(), env::consts::OS));

pub const APP_ID: &str = "moe.tsuna.tsukimi";
pub const CLIENT_ID: &str = "Tsukimi";
const APP_RESOURCE_PATH: &str = "/moe/tsuna/tsukimi";
#[cfg(not(target_os = "windows"))]
const GRESOURCE_FILE: &str = "tsukimi.gresource";

pub fn run() -> gtk::glib::ExitCode {
    Args::parse().init();

    #[cfg(target_os = "windows")]
    bootstrap_windows_runtime();

    // Initialize gettext
    unsafe { setlocale(LocaleCategory::LcAll, String::new()) };
    // mpv's mp_create() refuses to run unless LC_NUMERIC is "C" (it prints
    // "Non-C locale detected" and returns NULL, which used to abort the whole
    // application before the window could appear). Restore it after the
    // LC_ALL="" environment lookup above, as libmpv's documentation requires.
    unsafe { setlocale(LocaleCategory::LcNumeric, "C") };
    bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8").expect("Failed to set textdomain codeset");
    bindtextdomain(GETTEXT_PACKAGE, locale_dir())
        .expect("Invalid argument passed to bindtextdomain");

    textdomain(GETTEXT_PACKAGE).expect("Invalid string passed to textdomain");

    adw::init().expect("Failed to initialize Adwaita");
    mutsumi::init();

    register_gio_resources();

    ui::init();

    gtk::glib::set_application_name(CLIENT_ID);

    let _tokio_guard = runtime().enter();
    Application::new().run_with_args::<&str>(&[])
}

fn register_gio_resources() {
    // Non-Windows: the build system installs resources as a bundle.
    #[cfg(not(target_os = "windows"))]
    {
        let path = std::path::Path::new(PKGDATADIR).join(GRESOURCE_FILE);
        let resources = gtk::gio::Resource::load(path).expect("Failed to load resources.");
        gtk::gio::resources_register(&resources);
    }

    // Windows: Cargo embeds resources into the binary at build time.
    #[cfg(target_os = "windows")]
    gtk::gio::resources_register_include!("tsukimi.gresource")
        .expect("Failed to register resources.");
}

/// Directory containing the compiled gettext message catalogs.
///
/// Non-Windows platforms use the install prefix provided by the build
/// system. Windows has no install prefix, so a portable `share/locale`
/// directory next to the executable is preferred, falling back to the
/// catalogs compiled into the build output so that `cargo run` also picks
/// up translations.
pub fn locale_dir() -> &'static str {
    static FLOCALEDIR: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    FLOCALEDIR.get_or_init(|| {
        #[cfg(not(target_os = "windows"))]
        {
            LOCALEDIR
        }

        #[cfg(target_os = "windows")]
        {
            if let Ok(exe_path) = std::env::current_exe()
                && let Some(install_root) = exe_path.ancestors().nth(2)
            {
                let locale_path = install_root.join("share").join("locale");
                if locale_path.is_dir() {
                    return Box::leak(locale_path.into_boxed_path())
                        .to_str()
                        .expect("Failed to convert locale dir to str");
                }
            }

            if !BUILD_LOCALEDIR.is_empty() && Path::new(BUILD_LOCALEDIR).is_dir() {
                return BUILD_LOCALEDIR;
            }

            LOCALEDIR
        }
    })
}

/// Prepare the Windows runtime before GTK/GLib caches any path: GSettings
/// has no system-wide schema directory on Windows, so point it at the
/// compiled schemas shipped next to the executable (or built into OUT_DIR
/// for `cargo run`) and select the keyfile backend so preferences persist.
#[cfg(target_os = "windows")]
fn bootstrap_windows_runtime() {
    // mpv (built against UCRT) refuses to initialize unless LC_NUMERIC=="C":
    // it prints "Non-C locale detected" and returns NULL from mpv_create(),
    // which used to abort the whole process before the window appeared.
    //
    // On the windows-gnu target Rust's own setlocale() only mutates msvcrt's
    // locale, which UCRT (used by the MSYS2 builds of mpv and GLib) ignores,
    // and the LC_ALL environment variable is only consulted by UCRT at dll
    // initialization, so we call UCRT's setlocale() directly to set its
    // process-global locale state.
    force_ucrt_c_locale();

    if env::var_os("GSETTINGS_SCHEMA_DIR").is_none()
        && let Some(dir) = windows_schema_dir()
    {
        unsafe { env::set_var("GSETTINGS_SCHEMA_DIR", dir) };
    }

    // The keyfile backend stores settings under the user config directory;
    // the default (memory) backend would forget everything on exit.
    if env::var_os("GSETTINGS_BACKEND").is_none() {
        unsafe { env::set_var("GSETTINGS_BACKEND", "keyfile") };
    }
}

/// Force UCRT's process-global locale to "C" by calling `setlocale()` in the
/// `ucrtbase.dll` that the MSYS2 builds of mpv and GLib already loaded.
///
/// mpv's `mp_create()` returns NULL unless `LC_NUMERIC == "C"`. Rust's own
/// `setlocale()` on the `windows-gnu` target targets msvcrt, whose locale
/// state is independent of UCRT's, and setting the `LC_ALL` environment
/// variable only takes effect when UCRT reads it at dll init — neither helps a
/// process that is already running. Calling UCRT's `setlocale(LC_ALL, "C")`
/// directly mutates the same global `ucrtbase.dll` state that mpv queries, so
/// it is the only reliable fix here.
#[cfg(target_os = "windows")]
fn force_ucrt_c_locale() {
    use std::{
        ffi::c_void,
        os::raw::{
            c_char,
            c_int,
        },
    };

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(lp_file_name: *const u16) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *const c_void;
    }

    unsafe {
        let name: Vec<u16> = "ucrtbase.dll\0".encode_utf16().collect();
        let module = LoadLibraryW(name.as_ptr());
        if module.is_null() {
            return;
        }
        let proc = GetProcAddress(module, b"setlocale\0".as_ptr() as *const u8);
        if proc.is_null() {
            return;
        }
        type SetLocale = unsafe extern "C" fn(c_int, *const c_char) -> *mut c_char;
        let setlocale = core::mem::transmute::<*const c_void, SetLocale>(proc);
        // LC_ALL == 6 on Windows; setting it to "C" also resets LC_NUMERIC.
        setlocale(6, b"C\0".as_ptr() as *const c_char);
    }
}

#[cfg(target_os = "windows")]
fn windows_schema_dir() -> Option<std::path::PathBuf> {
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(install_root) = exe_path.ancestors().nth(2)
    {
        let dir = install_root.join("share").join("glib-2.0").join("schemas");
        if dir.join("gschemas.compiled").exists() {
            return Some(dir);
        }
    }

    if !BUILD_SCHEMADIR.is_empty() {
        let dir = Path::new(BUILD_SCHEMADIR);
        if dir.join("gschemas.compiled").exists() {
            return Some(dir.to_path_buf());
        }
    }

    None
}
