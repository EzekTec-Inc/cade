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

/// A local one-shot HTTP source exercises the production download/checksum path.
async fn serve_archive(bytes: Vec<u8>, status: u16) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let bytes_read = stream.read(&mut request).await.unwrap();
        assert!(
            bytes_read > 0,
            "HTTP fixture must receive a request before responding"
        );
        let header = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        stream.write_all(header.as_bytes()).await.unwrap();
        stream.write_all(&bytes).await.unwrap();
    });
    format!("http://{address}/plugin.tar.gz")
}

fn archive(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::default(),
    ));
    for (name, content, mode) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(*mode);
        header.set_cksum();
        builder.append_data(&mut header, name, *content).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

#[cfg(unix)]
fn script_package(script: &[u8]) -> Vec<u8> {
    archive(&[
        ("package/cade-plugin.json", br#"{"name":"Demo","version":"1.2.3","tools":[{"schema":"schemas/arbitrary-file.json","handler":"bin/echo"},{"schema":"schemas/declaration.json"}]}"#, 0o644),
        ("package/schemas/arbitrary-file.json", br#"{"name":"demo_echo","description":"Echo JSON","parameters":{"type":"object"}}"#, 0o644),
        ("package/schemas/declaration.json", br#"{"name":"declared_only","parameters":{"type":"object"}}"#, 0o644),
        ("package/bin/echo", script, 0o755),
    ])
}

#[cfg(unix)]
#[tokio::test]
async fn install_catalog_invoke_delete_invalidates_all_engine_instances() {
    use crate::{NativePluginEngine, PluginEngine};
    let temp = tempfile::tempdir().unwrap();
    let engine = NativePluginEngine::new(vec![temp.path().into()], temp.path().into());
    let observer = NativePluginEngine::new(vec![temp.path().into()], temp.path().into());
    let bytes = script_package(b"#!/bin/sh\ncat\n");
    let checksum = compute_sha256(&bytes);
    let url = serve_archive(bytes, 200).await;
    let report = engine
        .install_with_checksum(&url, "demo", Some(&checksum))
        .await
        .unwrap();
    assert_eq!(report.tools_count, 1);
    let tools = observer.list_tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "demo_echo");
    assert!(tools[0].handler.as_ref().unwrap().is_file());
    let args = serde_json::json!({"text":"real script output"});
    assert_eq!(
        observer.dispatch("demo_echo", &args).await.unwrap(),
        args.to_string()
    );
    assert!(engine.dispatch("declared_only", &args).await.is_err());
    assert!(engine.uninstall("../demo").is_err());
    assert_eq!(engine.uninstall("demo").unwrap().status, "removed");
    assert!(observer.list_tools().is_empty());
    assert!(observer.dispatch("demo_echo", &args).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn failed_checksum_validation_or_download_preserves_existing_ready_plugin() {
    use crate::{NativePluginEngine, PluginEngine};
    let temp = tempfile::tempdir().unwrap();
    let engine = NativePluginEngine::new(vec![temp.path().into()], temp.path().into());
    let bytes = script_package(b"#!/bin/sh\ncat >/dev/null\nprintf original\n");
    engine
        .install(&serve_archive(bytes.clone(), 200).await, "demo")
        .await
        .unwrap();
    let url = serve_archive(bytes, 200).await;
    let failure = engine
        .install_with_checksum(&url, "demo", Some(&"0".repeat(64)))
        .await
        .unwrap_err();
    assert!(matches!(failure, crate::Error::IntegrityError { .. }));
    let bad = archive(&[
        (
            "cade-plugin.json",
            br#"{"name":"Broken","tools":[{"schema":"bad.json","handler":"missing"}]}"#,
            0o644,
        ),
        (
            "bad.json",
            br#"{"name":"broken","parameters":{"type":"object"}}"#,
            0o644,
        ),
    ]);
    assert!(
        engine
            .install(&serve_archive(bad, 200).await, "demo")
            .await
            .is_err()
    );
    assert!(
        engine
            .install(
                &serve_archive(b"not an archive".to_vec(), 404).await,
                "demo"
            )
            .await
            .is_err()
    );
    assert_eq!(engine.list_tools().len(), 1);
    assert_eq!(
        engine
            .dispatch("demo_echo", &serde_json::json!({}))
            .await
            .unwrap(),
        "original"
    );
    assert_eq!(
        std::fs::read_dir(temp.path()).unwrap().count(),
        1,
        "failed installs clean staging directories"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn real_script_failure_and_missing_handler_are_not_successes() {
    use crate::{NativePluginEngine, PluginEngine};
    let temp = tempfile::tempdir().unwrap();
    let engine = NativePluginEngine::new(vec![temp.path().into()], temp.path().into());
    let bytes = script_package(b"#!/bin/sh\ncat >/dev/null\nprintf failure >&2\nexit 7\n");
    engine
        .install(&serve_archive(bytes, 200).await, "demo")
        .await
        .unwrap();
    assert!(
        engine
            .dispatch("demo_echo", &serde_json::json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("failure")
    );
    std::fs::remove_file(temp.path().join("demo/bin/echo")).unwrap();
    assert!(engine.list_tools().is_empty());
    assert!(
        engine
            .dispatch("demo_echo", &serde_json::json!({}))
            .await
            .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn handler_escape_and_wasm_are_never_advertised_as_native_scripts() {
    use crate::{NativePluginEngine, PluginEngine};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("demo");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(
        root.join("cade-plugin.json"),
        br#"{"name":"Demo","tools":[{"schema":"schema.json","handler":"escape"}]}"#,
    )
    .unwrap();
    std::fs::write(
        root.join("schema.json"),
        br#"{"name":"demo_echo","parameters":{"type":"object"}}"#,
    )
    .unwrap();
    std::os::unix::fs::symlink("/bin/sh", root.join("escape")).unwrap();
    let engine = NativePluginEngine::new(vec![temp.path().into()], temp.path().into());
    assert!(engine.list_tools().is_empty());
    assert!(!validate_plugin(&root).unwrap().is_valid);
    std::fs::write(root.join("module.wasm"), b"\0asm").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        root.join("module.wasm"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(
        root.join("cade-plugin.json"),
        br#"{"name":"Demo","tools":[{"schema":"schema.json","handler":"module.wasm"}]}"#,
    )
    .unwrap();
    assert!(engine.list_tools().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn catalogue_and_dispatch_use_the_same_deterministic_collision_winner() {
    use crate::{NativePluginEngine, PluginEngine};
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let global = temp.path().join("global");
    let project_engine = NativePluginEngine::new(vec![project.clone()], project.clone());
    let global_engine = NativePluginEngine::new(vec![global.clone()], global.clone());
    project_engine
        .install(
            &serve_archive(
                script_package(b"#!/bin/sh\ncat >/dev/null\nprintf project\n"),
                200,
            )
            .await,
            "demo",
        )
        .await
        .unwrap();
    global_engine
        .install(
            &serve_archive(
                script_package(b"#!/bin/sh\ncat >/dev/null\nprintf global\n"),
                200,
            )
            .await,
            "demo",
        )
        .await
        .unwrap();
    let engine = NativePluginEngine::new(vec![project, global], temp.path().join("project"));
    assert_eq!(engine.load_all().unwrap().len(), 1);
    assert_eq!(engine.list_tools().len(), 1);
    assert_eq!(
        engine
            .dispatch("demo_echo", &serde_json::json!({}))
            .await
            .unwrap(),
        "project"
    );
    engine.uninstall("demo").unwrap();
    assert_eq!(
        engine
            .dispatch("demo_echo", &serde_json::json!({}))
            .await
            .unwrap(),
        "global"
    );
}
