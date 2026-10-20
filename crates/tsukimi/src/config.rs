pub const GETTEXT_PACKAGE: &str = "tsukimi";

pub const VERSION: &str = compile_version();

pub const LOCALEDIR: &str = match option_env!("TSUKIMI_LOCALEDIR") {
    Some(path) => path,
    None => "/usr/share/locale",
};

pub const PKGDATADIR: &str = match option_env!("TSUKIMI_PKGDATADIR") {
    Some(path) => path,
    None => "/usr/share/tsukimi",
};

/// Directory the build script compiled the gettext message catalogs into.
/// Only set on Windows; empty elsewhere.
pub const BUILD_LOCALEDIR: &str = match option_env!("TSUKIMI_LOCALE_BUILD_DIR") {
    Some(path) => path,
    None => "",
};

/// Directory the build script compiled the GSettings schemas into.
/// Only set on Windows; empty elsewhere.
pub const BUILD_SCHEMADIR: &str = match option_env!("TSUKIMI_SCHEMA_BUILD_DIR") {
    Some(path) => path,
    None => "",
};

pub const fn compile_version() -> &'static str {
    const fn fallback(version: Option<&'static str>) -> &'static str {
        match version {
            Some(version) => version,
            None => env!("CARGO_PKG_VERSION"),
        }
    }

    if option_env!("TSUKIMI_RELEASE").is_none() {
        return fallback(option_env!("TSUKIMI_GIT_VERSION"));
    };

    fallback(option_env!("TSUKIMI_VERSION"))
}

pub const fn version() -> &'static str {
    VERSION
}
