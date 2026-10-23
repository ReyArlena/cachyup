//! Small config-file editors for tweaks. All writes go through
//! [`crate::paths::write_file`], so symlinked dotfiles stay symlinks.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::catalog::{SetOp, Tweak};
use crate::paths::{Paths, write_file};

/// The value at a dotted key, or `None` when the file, key or JSON is missing.
pub fn json_get(path: &Path, key: &str) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let mut node = &root;
    for part in key.split('.') {
        node = node.get(part)?;
    }
    Some(node.clone())
}

/// Set a dotted key, creating the file and intermediate objects as needed.
/// An existing file that is not valid JSON is left alone (error).
pub fn json_set(path: &Path, key: &str, new: &Value) -> Result<()> {
    let mut root = if path.exists() {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        if text.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            serde_json::from_str(&text)
                .with_context(|| format!("{} is not valid JSON", path.display()))?
        }
    } else {
        Value::Object(Default::default())
    };
    let parts: Vec<&str> = key.split('.').collect();
    let mut node = &mut root;
    for (i, part) in parts.iter().enumerate() {
        let Some(map) = node.as_object_mut() else {
            bail!("{}: {key} is inside a non-object", path.display());
        };
        if i == parts.len() - 1 {
            map.insert(part.to_string(), new.clone());
            break;
        }
        node = map
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(Default::default()));
    }
    let mut text = serde_json::to_string_pretty(&root)?;
    text.push('\n');
    write_file(path, &text).with_context(|| format!("writing {}", path.display()))
}

/// The value of `key` under `[section]`, or `None`.
pub fn ini_get(path: &Path, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let header = format!("[{section}]");
    let prefix = format!("{key}=");
    let mut in_section = false;
    for line in text.lines() {
        if line.starts_with('[') {
            in_section = line == header;
            continue;
        }
        if in_section && let Some(v) = line.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
    }
    None
}

/// Set `key=value` under `[section]`: replaced in place if present, appended
/// to the section otherwise, section (and file) created if missing. Comments
/// and other sections are untouched.
pub fn ini_set(path: &Path, section: &str, key: &str, value: &str) -> Result<()> {
    let text = if path.exists() {
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
    } else {
        String::new()
    };
    let header = format!("[{section}]");
    let prefix = format!("{key}=");
    let entry = format!("{key}={value}");
    let mut out: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut done = false;
    for line in text.lines() {
        if line.starts_with('[') {
            if in_section && !done {
                // Append at the end of the section, before any trailing blanks.
                let at = out
                    .iter()
                    .rposition(|l| !l.trim().is_empty())
                    .map_or(0, |i| i + 1);
                out.insert(at, entry.clone());
                done = true;
            }
            in_section = line == header;
            out.push(line.to_string());
            continue;
        }
        if in_section && line.starts_with(&prefix) {
            if !done {
                out.push(entry.clone());
                done = true;
            }
            continue;
        }
        out.push(line.to_string());
    }
    if !done {
        if !in_section {
            if !out.is_empty() {
                out.push(String::new());
            }
            out.push(header);
        }
        out.push(entry);
    }
    let mut result = out.join("\n");
    result.push('\n');
    write_file(path, &result).with_context(|| format!("writing {}", path.display()))
}

/// Whether one value of a tweak is already in place.
pub fn op_applied(op: &SetOp, paths: &Paths) -> bool {
    match op {
        SetOp::Json { file, key, value } => {
            json_get(&paths.expand(file), key).as_ref() == Some(value)
        }
        SetOp::Ini {
            file,
            section,
            key,
            value,
        } => ini_get(&paths.expand(file), section, key).as_deref() == Some(value.as_str()),
    }
}

/// Whether every value of a tweak is in place.
pub fn tweak_applied(tweak: &Tweak, paths: &Paths) -> bool {
    tweak.sets.iter().all(|op| op_applied(op, paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_set_creates_and_preserves() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/settings.json");
        assert_eq!(json_get(&path, "SKIP_HOST_UPDATE"), None);
        json_set(&path, "SKIP_HOST_UPDATE", &Value::Bool(true)).unwrap();
        assert_eq!(json_get(&path, "SKIP_HOST_UPDATE"), Some(Value::Bool(true)));
        std::fs::write(&path, r#"{"keep": 1, "SKIP_HOST_UPDATE": false}"#).unwrap();
        json_set(&path, "SKIP_HOST_UPDATE", &Value::Bool(true)).unwrap();
        json_set(&path, "nested.deep", &Value::from(2)).unwrap();
        assert_eq!(json_get(&path, "keep"), Some(Value::from(1)));
        assert_eq!(json_get(&path, "nested.deep"), Some(Value::from(2)));
        std::fs::write(&path, "not json").unwrap();
        assert!(json_set(&path, "x", &Value::Null).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
    }

    #[test]
    fn ini_set_replaces_appends_and_creates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mimeapps.list");
        ini_set(
            &path,
            "Default Applications",
            "x-scheme-handler/http",
            "a.desktop",
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[Default Applications]\nx-scheme-handler/http=a.desktop\n"
        );
        std::fs::write(
            &path,
            "# top\n[Added Associations]\nx-scheme-handler/http=z.desktop\n\n[Default Applications]\ntext/plain=ed.desktop\nx-scheme-handler/http=old.desktop\n\n[Other]\nk=v\n",
        )
        .unwrap();
        ini_set(
            &path,
            "Default Applications",
            "x-scheme-handler/http",
            "a.desktop",
        )
        .unwrap();
        ini_set(
            &path,
            "Default Applications",
            "x-scheme-handler/https",
            "a.desktop",
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# top\n[Added Associations]\nx-scheme-handler/http=z.desktop\n\n[Default Applications]\ntext/plain=ed.desktop\nx-scheme-handler/http=a.desktop\nx-scheme-handler/https=a.desktop\n\n[Other]\nk=v\n"
        );
        assert_eq!(
            ini_get(&path, "Added Associations", "x-scheme-handler/http").as_deref(),
            Some("z.desktop")
        );
        assert_eq!(
            ini_get(&path, "Default Applications", "x-scheme-handler/https").as_deref(),
            Some("a.desktop")
        );
    }
}
