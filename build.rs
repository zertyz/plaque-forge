use std::{
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

fn source_files(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to inspect {}: {error}", directory.display()))
        .map(|entry| entry.expect("failed to inspect source entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            source_files(root, &path, output);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            output.push(
                path.strip_prefix(root)
                    .expect("source is below crate root")
                    .to_path_buf(),
            );
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("crate root"));
    let mut files = vec![
        PathBuf::from("Cargo.toml"),
        PathBuf::from("Cargo.lock"),
        PathBuf::from("build.rs"),
    ];
    source_files(&root, &root.join("src"), &mut files);
    files.sort();

    let mut digest = Sha256::new();
    digest.update(b"plaque-forge.renderer-source/1\0");
    for relative in files {
        println!("cargo:rerun-if-changed={}", relative.display());
        let name = relative.to_string_lossy();
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        digest.update(
            fs::read(root.join(&relative))
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", relative.display())),
        );
    }
    let identity = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    println!("cargo:rustc-env=PLAQUE_FORGE_RENDERER_SOURCE_SHA256={identity}");
}
