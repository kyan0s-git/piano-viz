//! Designs: the look, separable from the session and shareable as files.
//!
//! See docs/09-design-format.md.

mod color;
mod schema;

pub use color::{Color, srgb_to_linear};
pub use schema::*;

use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum DesignError {
    #[error("could not read {path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("{0}")]
    Parse(String),
    #[error("asset path {0:?} must stay inside the Design bundle")]
    AssetEscape(String),
}

/// A parsed Design plus everything worth telling the user about it.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub design: Design,
    /// Unknown keys, clamped values, version notes. Never fatal.
    pub warnings: Vec<String>,
    /// Directory that relative asset paths resolve against.
    pub bundle_dir: Option<PathBuf>,
}

/// Parse a Design from TOML source.
///
/// Lenient by design: unknown keys and out-of-range values produce warnings,
/// not errors, so a Design saved by a newer build still mostly works on an
/// older one. Only malformed TOML or wrongly-typed values fail.
pub fn from_toml(src: &str) -> Result<Loaded, DesignError> {
    let raw: toml::Table =
        src.parse().map_err(|e: toml::de::Error| DesignError::Parse(e.to_string()))?;
    let mut design: Design = toml::from_str(src).map_err(|e| DesignError::Parse(e.to_string()))?;

    let mut warnings = Vec::new();
    if design.meta.version > SCHEMA_VERSION {
        warnings.push(format!(
            "made with a newer version of piano-viz (schema {} > {SCHEMA_VERSION}); some settings may be ignored",
            design.meta.version
        ));
    }
    migrate(&mut design);

    // Anything in the file that doesn't survive a round trip through the
    // schema was not understood.
    if let Ok(known) = toml::Table::try_from(&design) {
        unknown_keys(&raw, &known, "", &mut warnings);
    }
    warnings.extend(design.sanitize());
    Ok(Loaded { design, warnings, bundle_dir: None })
}

/// Load a Design from a `design.toml` file or a bundle directory holding one.
pub fn load(path: impl AsRef<Path>) -> Result<Loaded, DesignError> {
    let path = path.as_ref();
    let (file, dir) = if path.is_dir() {
        (path.join("design.toml"), path.to_path_buf())
    } else {
        (path.to_path_buf(), path.parent().map(Path::to_path_buf).unwrap_or_default())
    };
    let src = std::fs::read_to_string(&file)
        .map_err(|source| DesignError::Io { path: file.display().to_string(), source })?;
    let mut loaded = from_toml(&src).map_err(|e| match e {
        DesignError::Parse(m) => DesignError::Parse(format!("{}: {m}", file.display())),
        e => e,
    })?;
    loaded.bundle_dir = Some(dir);
    Ok(loaded)
}

/// Serialize a Design to TOML.
pub fn to_toml(design: &Design) -> String {
    toml::to_string_pretty(design).unwrap_or_default()
}

/// Resolve an asset path from a Design against its bundle directory.
///
/// Designs are files people download from strangers, so a path may not be
/// absolute or climb out of the bundle with `..`.
pub fn resolve_asset(bundle_dir: &Path, rel: &str) -> Result<PathBuf, DesignError> {
    let p = Path::new(rel);
    let escapes = p.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
    if rel.is_empty() || escapes {
        return Err(DesignError::AssetEscape(rel.to_owned()));
    }
    let full = bundle_dir.join(p);
    // Catch escapes through symlinks, where the file exists.
    if let (Ok(real), Ok(root)) = (full.canonicalize(), bundle_dir.canonicalize())
        && !real.starts_with(&root)
    {
        return Err(DesignError::AssetEscape(rel.to_owned()));
    }
    Ok(full)
}

/// Upgrade older schema versions in place, one step at a time. There is only
/// version 1 so far; each future bump adds one arm here and one test.
fn migrate(design: &mut Design) {
    if design.meta.version < SCHEMA_VERSION {
        design.meta.version = SCHEMA_VERSION;
    }
}

fn unknown_keys(raw: &toml::Table, known: &toml::Table, prefix: &str, out: &mut Vec<String>) {
    for (k, v) in raw {
        let path = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
        match (v, known.get(k)) {
            (_, None) => out.push(format!("unknown setting `{path}` was ignored")),
            (toml::Value::Table(a), Some(toml::Value::Table(b))) => unknown_keys(a, b, &path, out),
            (toml::Value::Array(a), Some(toml::Value::Array(b))) => {
                for (i, (x, y)) in a.iter().zip(b).enumerate() {
                    if let (toml::Value::Table(x), toml::Value::Table(y)) = (x, y) {
                        unknown_keys(x, y, &format!("{path}[{i}]"), out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The Designs that ship with the app, as `(id, source)`.
pub const BUILTIN: &[(&str, &str)] = &[
    ("ember-classic", include_str!("../designs/ember-classic.toml")),
    ("aurora", include_str!("../designs/aurora.toml")),
    ("neon", include_str!("../designs/neon.toml")),
    ("minimal", include_str!("../designs/minimal.toml")),
    ("print", include_str!("../designs/print.toml")),
];

/// A built-in Design by id, falling back to the default.
pub fn builtin(id: &str) -> Design {
    BUILTIN
        .iter()
        .find(|(i, _)| *i == id)
        .and_then(|(_, src)| from_toml(src).ok())
        .map(|l| l.design)
        .unwrap_or_default()
}
