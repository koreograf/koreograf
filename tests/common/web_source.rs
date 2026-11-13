#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};

pub fn source() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("web")
        .join("js");
    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();
    let mut out = String::new();
    for file in files {
        out.push_str(&std::fs::read_to_string(&file).expect("read web source"));
        out.push('\n');
    }
    out
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).expect("read web source directory");
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("js") {
            files.push(path);
        }
    }
}
