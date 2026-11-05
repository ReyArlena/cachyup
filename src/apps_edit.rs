//! Edits to the user's `apps.toml` that keep its comments and layout
//! (`toml_edit`). Used to merge an old `user-apps.toml` in, and later to add
//! entries cachyup ships.

use anyhow::{Context, Result};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Value};

/// Package list keys, and the key that identifies an entry in each
/// array-of-tables section.
const LISTS: [&str; 3] = ["pacman", "aur", "flatpak"];
const SECTIONS: [(&str, &str); 2] = [("installer", "name"), ("tweak", "id")];

/// `base` plus every entry of `extra` it does not already have, as text.
/// Entries are matched by name (lists, installers) or id (tweaks). Both
/// inputs must be valid TOML; nothing in `base` is reformatted.
pub fn append(base: &str, extra: &str) -> Result<String> {
    let mut doc: DocumentMut = base.parse().context("apps.toml")?;
    let extra: DocumentMut = extra.parse().context("file to merge")?;

    for key in LISTS {
        let Some(items) = extra.get(key).and_then(Item::as_array) else {
            continue;
        };
        let target = doc
            .entry(key)
            .or_insert_with(|| Item::Value(Value::Array(Array::new())));
        let target = target
            .as_array_mut()
            .with_context(|| format!("`{key}` is not a list"))?;
        for item in items.iter() {
            let Some(name) = list_name(item) else {
                continue;
            };
            if target
                .iter()
                .any(|v| list_name(v).as_deref() == Some(&name))
            {
                continue;
            }
            push_value(target, item.clone());
        }
    }

    // New tables go after everything already in the file, not wherever
    // toml_edit would slot them.
    let mut position = doc
        .iter()
        .flat_map(|(_, item)| match item {
            Item::Table(t) => vec![t.position()],
            Item::ArrayOfTables(a) => a.iter().map(|t| t.position()).collect(),
            _ => Vec::new(),
        })
        .flatten()
        .max()
        .unwrap_or(0);
    for (key, id) in SECTIONS {
        let Some(tables) = extra.get(key).and_then(Item::as_array_of_tables) else {
            continue;
        };
        let target = doc
            .entry(key)
            .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
        let target = target
            .as_array_of_tables_mut()
            .with_context(|| format!("`[[{key}]]` is not a list of tables"))?;
        for table in tables.iter() {
            let Some(name) = table.get(id).and_then(Item::as_str) else {
                continue;
            };
            if target
                .iter()
                .any(|t| t.get(id).and_then(Item::as_str) == Some(name))
            {
                continue;
            }
            let mut table = table.clone();
            position += 1;
            table.set_position(Some(position));
            target.push(table);
        }
    }

    Ok(doc.to_string())
}

/// The name of a package list entry: a bare string or `{ name = ... }`.
fn list_name(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.value().trim().to_string()),
        Value::InlineTable(t) => t.get("name").and_then(Value::as_str).map(str::to_string),
        _ => None,
    }
}

/// Append to a list, matching the indent of its last entry so a multi-line
/// list stays one entry per line. A one-line list stays one line.
fn push_value(target: &mut Array, mut value: Value) {
    // An entry's prefix holds everything since the previous comma, including
    // any comment on the previous line; keep only the indent after the newline.
    let indent = target
        .iter()
        .last()
        .and_then(|v| v.decor().prefix().and_then(|p| p.as_str()))
        .and_then(|p| p.rsplit_once('\n').map(|(_, indent)| indent.to_string()));
    match indent {
        Some(indent) => {
            value.decor_mut().set_prefix(format!("\n{indent}"));
            value.decor_mut().set_suffix("");
            target.push_formatted(value);
            target.set_trailing_comma(true);
        }
        None => target.push(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_missing_entries_and_keeps_layout() {
        let base = "\
# My apps.
pacman = [
    \"ark\", # archiver
    \"htop\",
]
aur = [\"yay\"]

[[tweak]]
id = \"a\"
name = \"A\"
installs = [\"x\"]

[[installer]]
name = \"first\"
check = \"first\"
install = \"true\"
";
        let extra = "\
pacman = [\"htop\", \"btop\", { name = \"junction\", unless_cmd = \"dms\" }]
flatpak = [\"org.gnome.Chess\"]

[[installer]]
name = \"tool\"
check = \"tool\"
install = \"true\"

[[tweak]]
id = \"a\"
name = \"Clash\"
installs = [\"y\"]

# Second tweak.
[[tweak]]
id = \"b\"
name = \"B\"
installs = [\"z\"]
";
        let out = append(base, extra).unwrap();
        let expected = "\
# My apps.
pacman = [
    \"ark\", # archiver
    \"htop\",
    \"btop\",
    { name = \"junction\", unless_cmd = \"dms\" },
]
aur = [\"yay\"]
flatpak = [\"org.gnome.Chess\"]

[[tweak]]
id = \"a\"
name = \"A\"
installs = [\"x\"]

[[installer]]
name = \"first\"
check = \"first\"
install = \"true\"

[[installer]]
name = \"tool\"
check = \"tool\"
install = \"true\"

# Second tweak.
[[tweak]]
id = \"b\"
name = \"B\"
installs = [\"z\"]
";
        assert_eq!(out, expected);
        // The result parses as a catalog.
        let file: Result<toml::Value, _> = toml::from_str(&out);
        assert!(file.is_ok());
    }

    #[test]
    fn bad_input_is_an_error() {
        assert!(append("pacman = [", "").is_err());
        assert!(append("", "pacman = [").is_err());
        assert_eq!(append("", "").unwrap(), "");
    }
}
