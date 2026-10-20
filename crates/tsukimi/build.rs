//! Windows-only build steps.
//!
//! On Linux the Meson build installs the compiled UI resources, message
//! catalogs and GSettings schemas into well-known prefixes, and the binary
//! loads them from `PKGDATADIR` / `LOCALEDIR` at runtime. Windows has no
//! such install step, so the same artifacts are produced here and embedded
//! into (or referenced from) the binary:
//!
//! - `resources.gresource` → embedded via `resources_register_include!`
//! - `po/*.po`            → compiled to `.mo` under `OUT_DIR/locale`
//! - `*.gschema.xml`      → compiled to `gschemas.compiled` under `OUT_DIR`
//! - application manifest + icon embedded through `tsukimi_manifest.rc`

use std::{
    env,
    fs,
    path::PathBuf,
    process::Command,
};

fn main() {
    // These variables are passed by Meson, see src/config.rs.
    // We track them here to ensure Meson and direct Cargo calls can share the
    // same target directory safely.
    println!("cargo:rerun-if-env-changed=TSUKIMI_VERSION");
    println!("cargo:rerun-if-env-changed=TSUKIMI_LOCALEDIR");
    println!("cargo:rerun-if-env-changed=TSUKIMI_PKGDATADIR");
    println!("cargo:rerun-if-changed=resources");
    println!("cargo:rerun-if-changed=po");
    println!("cargo:rerun-if-changed=tsukimi_manifest.rc");

    #[cfg(windows)]
    {
        compile_gresources();
        compile_gettext();
        compile_gschemas();
        embed_manifest();
    }
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"))
}

fn out_dir() -> PathBuf {
    PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is not set"))
}

/// Compile `resources/resources.gresource.xml` into a single binary
/// `tsukimi.gresource` inside OUT_DIR, where the
/// `resources_register_include!` macro picks it up.
#[cfg(windows)]
fn compile_gresources() {
    let manifest_dir = manifest_dir();
    let target = out_dir().join("tsukimi.gresource");

    let output = Command::new("glib-compile-resources")
        .arg("--sourcedir")
        .arg(manifest_dir.join("resources"))
        .arg("--target")
        .arg(&target)
        .arg(manifest_dir.join("resources/resources.gresource.xml"))
        .output()
        .expect("Failed to execute glib-compile-resources");

    if !output.status.success() {
        panic!(
            "glib-compile-resources failed with {}:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Compile every `po/*.po` into `OUT_DIR/locale/<lang>/LC_MESSAGES/tsukimi.mo`
/// and point `TSUKIMI_LOCALE_BUILD_DIR` at the result so `locale_dir()` can
/// find the catalogs when running from the build output.
#[cfg(windows)]
fn compile_gettext() {
    let po_dir = manifest_dir().join("po");
    let locale_root = out_dir().join("locale");

    let mut compiled_any = false;

    if let Ok(entries) = fs::read_dir(&po_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("po") {
                continue;
            }

            let Some(lang) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };

            let mo_dir = locale_root.join(lang).join("LC_MESSAGES");
            fs::create_dir_all(&mo_dir).expect("Failed to create locale output directory");
            let mo_path = mo_dir.join("tsukimi.mo");

            match Command::new("msgfmt")
                .arg("--check")
                .arg("--output-file")
                .arg(&mo_path)
                .arg(&path)
                .output()
            {
                Ok(output) if output.status.success() => compiled_any = true,
                Ok(output) => {
                    println!(
                        "cargo:warning=msgfmt failed for {}: {}",
                        path.display(),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                Err(error) => {
                    println!(
                        "cargo:warning=msgfmt is not available ({error}); translations will not be built"
                    );
                    break;
                }
            }
        }
    }

    if compiled_any {
        println!(
            "cargo:rustc-env=TSUKIMI_LOCALE_BUILD_DIR={}",
            locale_root.display()
        );
    }
}

/// Compile the GSettings schema into `OUT_DIR/schemas/gschemas.compiled` so
/// `bootstrap_windows_runtime()` can point `GSETTINGS_SCHEMA_DIR` at it when
/// no portable install is present. Non-fatal: without schemas GSettings
/// falls back to schema defaults.
#[cfg(windows)]
fn compile_gschemas() {
    let schema_dir = out_dir().join("schemas");
    fs::create_dir_all(&schema_dir).expect("Failed to create schema output directory");

    let source = manifest_dir()
        .join("resources")
        .join("moe.tsuna.tsukimi.gschema.xml");
    fs::copy(
        &source,
        schema_dir.join("moe.tsuna.tsukimi.gschema.xml"),
    )
    .expect("Failed to copy gschema");

    match Command::new("glib-compile-schemas").arg(&schema_dir).output() {
        Ok(output) if output.status.success() => {
            println!(
                "cargo:rustc-env=TSUKIMI_SCHEMA_BUILD_DIR={}",
                schema_dir.display()
            );
        }
        Ok(output) => {
            println!(
                "cargo:warning=glib-compile-schemas failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(error) => {
            println!(
                "cargo:warning=glib-compile-schemas is not available ({error}); GSettings will use defaults"
            );
        }
    }
}

/// Embed the application manifest and icon into the executable through the
/// `embed-resource` crate.
#[cfg(windows)]
fn embed_manifest() {
    let manifest = manifest_dir().join("tsukimi_manifest.rc");
    embed_resource::compile(manifest, embed_resource::NONE);
}
