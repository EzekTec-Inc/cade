use std::path::{Path, PathBuf};

fn ensure_within_root(root: &Path, raw_path: &str) -> Result<(), String> {
    let p = Path::new(raw_path);
    let abs = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };

    let mut parts: Vec<std::path::Component> = Vec::new();
    for c in abs.components() {
        match c {
            std::path::Component::ParentDir => {
                if let Some(last) = parts.last() {
                    match last {
                        std::path::Component::Normal(_) => { parts.pop(); }
                        _ => { parts.push(std::path::Component::ParentDir); }
                    }
                } else {
                    parts.push(std::path::Component::ParentDir);
                }
            }
            std::path::Component::CurDir => {}
            other => parts.push(other),
        }
    }
    let normalized: PathBuf = parts.iter().collect();

    let mut current = normalized.as_path();
    let mut non_existent = Vec::new();

    while !current.exists() && current.parent().is_some() {
        if let Some(name) = current.file_name() {
            non_existent.push(name.to_os_string());
        }
        current = current.parent().unwrap();
    }

    let mut resolved = std::fs::canonicalize(current)
        .unwrap_or_else(|_| current.to_path_buf());

    for comp in non_existent.into_iter().rev() {
        resolved.push(comp);
    }

    if !resolved.starts_with(root) {
        return Err(format!("outside root: {}", resolved.display()));
    }
    Ok(())
}

fn main() {
    let root = Path::new("/tmp/cade_test_root");
    std::fs::create_dir_all(root).unwrap();
    std::os::unix::fs::symlink("/etc", root.join("etc_symlink")).unwrap_or(());
    
    match ensure_within_root(root, "etc_symlink/passwd") {
        Ok(_) => println!("VULNERABLE: etc_symlink/passwd allowed!"),
        Err(e) => println!("SAFE: etc_symlink/passwd blocked: {}", e),
    }

    match ensure_within_root(root, "new_file.txt") {
        Ok(_) => println!("SAFE: new_file.txt allowed"),
        Err(e) => println!("VULNERABLE: new_file.txt blocked: {}", e),
    }
}
