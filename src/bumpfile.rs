use crate::bump::{BumpError, ensure_directory_exists};
use crate::version::{Version, VersionMode};
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Key, Table, Value, value};

const INIT_TEMPLATE_TIMESTAMP: &str = "1970-01-01 00:00:00 UTC";

pub struct BumpFile {
    path: PathBuf,
    doc: DocumentMut,
}

fn bumpfile_parse_error(path: &Path, message: impl fmt::Display) -> BumpError {
    BumpError::ParseError(format!(
        "{message} in {}. Recreate your bumpfile with 'bump init'.",
        path.display()
    ))
}

fn table<'a>(doc: &'a DocumentMut, section: &str, path: &Path) -> Result<&'a Table, BumpError> {
    doc.get(section)
        .and_then(|item| item.as_table())
        .ok_or_else(|| bumpfile_parse_error(path, format!("'{section}' table not found")))
}

fn table_mut<'a>(
    doc: &'a mut DocumentMut,
    section: &str,
    path: &Path,
) -> Result<&'a mut Table, BumpError> {
    doc.get_mut(section)
        .and_then(|item| item.as_table_mut())
        .ok_or_else(|| bumpfile_parse_error(path, format!("'{section}' table not found")))
}

fn set<V: Into<Value>>(
    table: &mut Table,
    key: &str,
    val: V,
    section: &str,
    path: &Path,
) -> Result<(), BumpError> {
    if !table.contains_key(key) {
        return Err(bumpfile_parse_error(
            path,
            format!("Expected key '{key}' not found in [{section}]"),
        ));
    }
    table[key] = value(val);
    Ok(())
}

fn set_or_remove(
    table: &mut Table,
    key: &str,
    val: Option<u32>,
    section: &str,
    path: &Path,
) -> Result<(), BumpError> {
    match val {
        Some(n) => set(table, key, i64::from(n), section, path),
        None => {
            table.remove(key);
            Ok(())
        }
    }
}

fn warn_mode_key_mismatch(path: &Path, doc: &DocumentMut) -> Result<(), BumpError> {
    let base = table(doc, "base", path)?;

    let mode = base
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or(VersionMode::Semver.as_str());
    let has_calver_keys = ["year", "month", "day"]
        .iter()
        .any(|key| base.contains_key(key));

    if mode == VersionMode::Semver.as_str() && has_calver_keys {
        println!(
            "bump warning: [base].mode is semver, but found calver keys (year/month/day) in {}. \
            \nThey will be treated as major/minor/patch and rewritten on save.",
            path.display()
        );
    }

    Ok(())
}

/// Rename `from` to `to` in place, carrying over the original key's decor so
/// any comment attached to the line survives the rename.
///
/// Used to migrate `major/minor/patch` <-> `year/month/day` when [base].mode
/// changes. No-op when `from` is absent or `to` already exists.
fn rename_key(table: &mut Table, from: &str, to: &str) {
    if from == to || !table.contains_key(from) {
        return;
    }
    if table.contains_key(to) {
        table.remove(from);
        return;
    }
    let Some(old_key) = table.key(from).cloned() else {
        return;
    };
    let Some(item) = table.remove(from) else {
        return;
    };
    let new_key = Key::new(to).with_leaf_decor(old_key.leaf_decor().clone());
    table.insert_formatted(&new_key, item);
}

fn write_base(doc: &mut DocumentMut, version: &Version, path: &Path) -> Result<(), BumpError> {
    let base = table_mut(doc, "base", path)?;

    set(base, "mode", version.base.mode.as_str(), "base", path)?;
    set(base, "delimiter", &version.base.delimiter, "base", path)?;

    let (major_key, minor_key, patch_key, old_major, old_minor, old_patch) =
        if version.base.mode == VersionMode::Calver {
            ("year", "month", "day", "major", "minor", "patch")
        } else {
            ("major", "minor", "patch", "year", "month", "day")
        };

    // Migrate leftover keys from the other mode *first*. Writing the new key
    // and deleting the old one cannot work here, because `set` requires the
    // key to already exist -- which is exactly what a mode switch guarantees
    // it does not.
    rename_key(base, old_major, major_key);
    rename_key(base, old_minor, minor_key);
    rename_key(base, old_patch, patch_key);

    set(base, major_key, i64::from(version.base.major), "base", path)?;
    set_or_remove(base, minor_key, version.base.minor, "base", path)?;
    set_or_remove(base, patch_key, version.base.patch, "base", path)?;

    Ok(())
}

fn write_version_into_doc(
    doc: &mut DocumentMut,
    version: &Version,
    path: &Path,
) -> Result<(), BumpError> {
    if !doc.contains_key("prefix") {
        return Err(bumpfile_parse_error(
            path,
            "Expected key 'prefix' not found in [(root)]",
        ));
    }
    doc["prefix"] = value(&version.prefix);

    let timestamp = table_mut(doc, "timestamp", path)?;
    set(
        timestamp,
        "format",
        &version.timestamp.format,
        "timestamp",
        path,
    )?;
    set(
        timestamp,
        "last",
        &version.timestamp.last,
        "timestamp",
        path,
    )?;

    write_base(doc, version, path)?;

    let phase = table_mut(doc, "phase", path)?;
    set(phase, "separator", &version.phase.separator, "phase", path)?;
    set(phase, "name", &version.phase.name, "phase", path)?;
    set(phase, "delimiter", &version.phase.delimiter, "phase", path)?;
    set(
        phase,
        "distance",
        i64::from(version.phase.distance),
        "phase",
        path,
    )?;

    let suffix = table_mut(doc, "suffix", path)?;
    set(suffix, "mode", version.suffix.mode.as_str(), "suffix", path)?;
    set(
        suffix,
        "separator",
        &version.suffix.separator,
        "suffix",
        path,
    )?;

    let label = table_mut(doc, "label", path)?;
    set(
        label,
        "position",
        version.label.position.as_str(),
        "label",
        path,
    )?;

    Ok(())
}

impl BumpFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, BumpError> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|err| {
            if err.kind() == io::ErrorKind::NotFound {
                BumpError::LogicError(format!(
                    "Configuration file not found at '{}'. Create one with 'bump init'",
                    path.display()
                ))
            } else {
                BumpError::IoError(err)
            }
        })?;

        let doc = content
            .parse::<DocumentMut>()
            .map_err(|e| BumpError::ParseError(format!("Failed to parse TOML document: {e}")))?;

        warn_mode_key_mismatch(path, &doc)?;

        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    pub fn create(path: impl AsRef<Path>) -> Result<Self, BumpError> {
        let path = path.as_ref();
        ensure_directory_exists(path)?;

        let template = include_str!("templates/bump.toml");
        let template_version: Version = {
            let content = template.replace("{timestamp}", INIT_TEMPLATE_TIMESTAMP);
            toml::from_str(&content).expect("init template must deserialize")
        };
        let current_timestamp = crate::version::format_timestamp(
            chrono::Utc::now(),
            &template_version.timestamp.format,
        )?;
        let content = template.replace("{timestamp}", &current_timestamp);

        fs::write(path, &content).map_err(BumpError::IoError)?;
        let doc = content
            .parse::<DocumentMut>()
            .map_err(|e| BumpError::ParseError(format!("Failed to parse TOML document: {e}")))?;

        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn version(&self) -> Result<Version, BumpError> {
        toml::from_str(&self.doc.to_string()).map_err(|err| {
            BumpError::ParseError(format!(
                "Failed to parse version from '{}': {err}. \
                Recreate your bumpfile with 'bump init'.",
                self.path.display()
            ))
        })
    }

    pub fn save(&mut self, version: &Version) -> Result<(), BumpError> {
        write_version_into_doc(&mut self.doc, version, &self.path)?;
        fs::write(&self.path, self.doc.to_string()).map_err(BumpError::IoError)
    }
}
