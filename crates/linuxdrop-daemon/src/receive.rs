use anyhow::{bail, Result};
use linuxdrop_core::{ReceiveOptions, Transfer};
use serde_json::Value;
use std::{collections::HashSet, path::PathBuf};

/// Treat a remote display name as a label, never as a directory path.
fn sender_component(name: &str) -> String {
    let value: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
        .take(64)
        .collect();
    let value = value.trim();
    if value.is_empty() {
        "Unknown device".into()
    } else {
        value.into()
    }
}

pub fn options(
    settings: &Value,
    transfer: &Transfer,
    patch: Option<Value>,
) -> Result<ReceiveOptions> {
    let mut options = ReceiveOptions {
        directory: Some(PathBuf::from(
            settings["receive"]["directory"]
                .as_str()
                .unwrap_or_default(),
        )),
        selected_indices: None,
        collision_policy: serde_json::from_value(settings["receive"]["collision_policy"].clone())?,
    };
    let explicit_directory = patch
        .as_ref()
        .is_some_and(|v| v.get("directory").is_some_and(|v| !v.is_null()));
    if let Some(patch) = patch {
        if !patch.is_object() {
            bail!("Receive options must be an object");
        }
        let mut merged = serde_json::to_value(&options)?;
        for (key, value) in patch.as_object().unwrap() {
            merged[key] = value.clone();
        }
        options = serde_json::from_value(merged)?;
    }
    let directory = options
        .directory
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("Choose a receive directory"))?;
    if !directory.is_absolute()
        || directory
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        bail!("Receive directory must be an absolute path without parent traversal");
    }
    if !explicit_directory {
        let subfolders = settings["receive"]["subfolders"].as_str().unwrap_or("none");
        if matches!(subfolders, "sender" | "sender_date") {
            directory.push(sender_component(&transfer.peer_name));
        }
        if matches!(subfolders, "date" | "sender_date") {
            directory.push(chrono::Local::now().format("%Y-%m-%d").to_string());
        }
    }
    if let Some(indices) = &options.selected_indices {
        let unique: HashSet<_> = indices.iter().collect();
        if indices.is_empty()
            || unique.len() != indices.len()
            || indices.iter().any(|&i| i >= transfer.files.len())
        {
            bail!("Choose at least one valid file without duplicates");
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_name_cannot_create_paths() {
        assert_eq!(sender_component("../../hello\\world\n"), "helloworld");
        assert_eq!(sender_component("../"), "Unknown device");
    }
}
