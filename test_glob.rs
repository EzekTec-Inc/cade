use ignore::{WalkBuilder, WalkState};
use globset::{Glob, GlobSetBuilder};
use std::sync::mpsc;
use std::path::Path;

fn test() {
    let glob = Glob::new("*.rs").unwrap();
    let mut builder = GlobSetBuilder::new();
    builder.add(glob);
    let globset = builder.build().unwrap();

    let root = Path::new(".");
    let (tx, rx) = mpsc::channel();
    WalkBuilder::new(root)
        .hidden(false)
        .build_parallel()
        .run(|| {
            let tx = tx.clone();
            let root = root.to_path_buf();
            let globset = globset.clone();
            Box::new(move |result| {
                if let Ok(entry) = result {
                    if entry.file_type().map_or(false, |ft| ft.is_file()) {
                        let path = entry.path();
                        let rel = path.strip_prefix(&root).unwrap_or(path);
                        if globset.is_match(rel) || globset.is_match(path.file_name().unwrap_or_default()) {
                            let mtime = entry
                                .metadata()
                                .ok()
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                            let _ = tx.send((mtime, path.display().to_string()));
                        }
                    }
                }
                WalkState::Continue
            })
        });
    drop(tx);
    let matches: Vec<_> = rx.into_iter().collect();
}
