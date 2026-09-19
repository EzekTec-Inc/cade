#[cfg(test)]
use crate::dev::{init_plugin, pack_plugin, validate_plugin};
use crate::manifest::PluginManifest;
use crate::marketplace::compute_sha256;

#[test]
fn test_load_toml_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let toml_content = r#"
name = "demo-toml-plugin"
version = "1.2.3"
description = "A test plugin in TOML"
author = "Dev <dev@example.com>"
skills = ["skills/test.md"]
"#;
    std::fs::write(tmp.path().join("cade-plugin.toml"), toml_content).unwrap();

    let manifest = PluginManifest::load(tmp.path()).expect("must load toml manifest");
    assert_eq!(manifest.name, "demo-toml-plugin");
    assert_eq!(manifest.version.as_deref(), Some("1.2.3"));
    assert_eq!(
        manifest.description.as_deref(),
        Some("A test plugin in TOML")
    );
    assert_eq!(manifest.skills.len(), 1);
    assert_eq!(manifest.skills[0].to_str().unwrap(), "skills/test.md");
}

#[test]
fn test_load_json_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let json_content = serde_json::json!({
        "name": "demo-json-plugin",
        "version": "2.0.0",
        "description": "A test plugin in JSON",
        "skills": ["skills/json.md"]
    });
    std::fs::write(
        tmp.path().join("cade-plugin.json"),
        json_content.to_string(),
    )
    .unwrap();

    let manifest = PluginManifest::load(tmp.path()).expect("must load json manifest");
    assert_eq!(manifest.name, "demo-json-plugin");
    assert_eq!(manifest.version.as_deref(), Some("2.0.0"));
    assert_eq!(manifest.skills.len(), 1);
}

#[test]
fn test_plugin_init_and_validate() {
    let tmp = tempfile::tempdir().unwrap();
    let created = init_plugin(tmp.path(), "my-sample-plugin", true).expect("init plugin");
    assert!(created.exists());
    assert!(created.join("cade-plugin.toml").exists());
    assert!(created.join("skills/hello/SKILL.md").exists());
    assert!(created.join("README.md").exists());

    let report = validate_plugin(&created).expect("validate plugin");
    assert!(
        report.is_valid,
        "scaffolded plugin must be valid: {:?}",
        report.issues
    );
    assert_eq!(report.name, "my-sample-plugin");
    assert_eq!(report.version, "0.1.0");
    assert!(report.issues.is_empty());
}

#[test]
fn test_plugin_validate_missing_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let toml_content = r#"
name = "broken-plugin"
version = "0.1.0"
skills = ["skills/nonexistent.md"]
subagents = ["subagents/ghost.toml"]
"#;
    std::fs::write(tmp.path().join("cade-plugin.toml"), toml_content).unwrap();

    let report = validate_plugin(tmp.path()).expect("validate plugin");
    assert!(!report.is_valid, "broken plugin must fail validation");
    assert_eq!(report.issues.len(), 2);
    assert!(report.issues[0].contains("skills/nonexistent.md"));
    assert!(report.issues[1].contains("subagents/ghost.toml"));
}

#[test]
fn test_plugin_pack_and_sha256_computation() {
    let tmp = tempfile::tempdir().unwrap();
    let created = init_plugin(tmp.path(), "packable-plugin", true).expect("init plugin");
    let packed = pack_plugin(&created, None).expect("pack plugin");

    assert!(packed.archive_path.exists());
    assert!(packed.archive_path.to_str().unwrap().ends_with(".tar.gz"));
    assert_eq!(packed.sha256.len(), 64, "SHA-256 must be 64 hex characters");
    assert!(packed.file_size_bytes > 0);

    let bytes = std::fs::read(&packed.archive_path).unwrap();
    let recomputed = compute_sha256(&bytes);
    assert_eq!(packed.sha256, recomputed);
}

#[test]
fn test_sha256_checksum_verification() {
    let data = b"Hello CADE Plugin Integrity Verification";
    let expected = compute_sha256(data);
    assert_eq!(expected.len(), 64);

    let mismatch = "0000000000000000000000000000000000000000000000000000000000000000";
    assert_ne!(expected, mismatch);
}
