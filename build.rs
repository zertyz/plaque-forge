use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

// The curated-font parser is shared verbatim so embedding and listing can
// never disagree about what the curated list means.
#[path = "src/media/curated.rs"]
mod curated;

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

    if std::env::var_os("CARGO_FEATURE_BUNDLE_MEDIA").is_some() {
        bundle_media(&root);
    }
}

/// Repository directories embedded wholesale by `bundle-media`.
const BUNDLED_DIRECTORIES: &[&str] = &[
    "assets/scenes",
    "assets/analysis",
    "assets/plaques",
    "assets/textures",
    "styles",
];

/// Homologation evidence stays an on-disk, CI-gated responsibility; the
/// segmentation policy is already embedded through `include_str!`.
const EMBED_EXCLUDED_PREFIXES: &[&str] = &["assets/homologation/", "assets/segmentation/"];

fn bundle_media(root: &Path) {
    let mut entries: BTreeMap<String, PathBuf> = BTreeMap::new();
    for directory in BUNDLED_DIRECTORIES {
        collect_directory(root, directory, &mut entries);
    }
    for video in videos(root) {
        let relative = relative_string(root, &video);
        insert_entry(&mut entries, relative, video);
    }

    let curated = curated_entries(root, &mut entries);

    // Rebuild whenever any embedded input changes.
    println!(
        "cargo:rerun-if-changed={}",
        root.join("styles/curated_fonts").display()
    );
    for directory in BUNDLED_DIRECTORIES {
        println!("cargo:rerun-if-changed={}", root.join(directory).display());
    }
    for path in entries.values() {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    emit_bundle(root, entries, curated);
}

fn collect_directory(root: &Path, directory: &str, entries: &mut BTreeMap<String, PathBuf>) {
    let absolute = root.join(directory);
    if !absolute.is_dir() {
        panic!("bundle-media requires {directory} to exist; run from a full repository checkout");
    }
    let mut pending = vec![absolute];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(&path)
            .unwrap_or_else(|error| panic!("failed to inspect {}: {error}", path.display()))
        {
            let entry_path = entry.expect("readable bundle entry").path();
            if entry_path.is_dir() {
                pending.push(entry_path);
                continue;
            }
            let relative = relative_string(root, &entry_path);
            if EMBED_EXCLUDED_PREFIXES
                .iter()
                .any(|prefix| relative.starts_with(prefix))
            {
                continue;
            }
            insert_entry(entries, relative, entry_path);
        }
    }
}

fn videos(root: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(root.join("assets"))
        .expect("assets directory must exist")
        .map(|entry| entry.expect("readable asset entry").path())
        .filter(|path| {
            path.is_file() && path.extension().and_then(|value| value.to_str()) == Some("mp4")
        })
        .collect();
    found.sort();
    found
}

fn insert_entry(entries: &mut BTreeMap<String, PathBuf>, relative: String, absolute: PathBuf) {
    assert!(
        entries.insert(relative.clone(), absolute).is_none(),
        "duplicate bundle path: {relative}"
    );
}

struct CuratedEmbedding {
    pattern: String,
    repository_file: bool,
    bundle_path: String,
    sha256: String,
    /// Family name fontconfig answered at build time; listing labels prefer it
    /// over re-resolving the pattern on whichever machine runs the binary.
    resolved_family: Option<String>,
}

/// Resolve every curated entry into something embeddable: repository files
/// join the entry map directly; family patterns resolve through `fc-match` on
/// the building machine and record their provenance for later auditing.
fn curated_entries(root: &Path, entries: &mut BTreeMap<String, PathBuf>) -> Vec<CuratedEmbedding> {
    let list_path = root.join("styles/curated_fonts");
    let source = fs::read_to_string(&list_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", list_path.display()));
    let curated = curated::parse_curated_fonts(&source).unwrap_or_else(|error| panic!("{error:#}"));

    let mut embeddings = Vec::new();
    let mut resolved_paths: BTreeMap<String, (String, String, String)> = BTreeMap::new();
    for font in curated {
        match font {
            curated::CuratedFont::Repository { path } => {
                let absolute = root.join(&path);
                assert!(
                    absolute.is_file(),
                    "curated repository font {} is missing",
                    absolute.display()
                );
                // Pinned files are bundle entries like any other media so a
                // bundled binary can serve their exact bytes anywhere.
                insert_entry(entries, path.clone(), absolute.clone());
                embeddings.push(CuratedEmbedding {
                    pattern: path.clone(),
                    repository_file: true,
                    bundle_path: path.clone(),
                    sha256: file_sha256(&absolute),
                    resolved_family: None,
                });
            }
            curated::CuratedFont::Family { pattern } => {
                if let Some((bundle_path, sha256, family)) =
                    resolved_paths.get(&pattern.to_lowercase())
                {
                    embeddings.push(CuratedEmbedding {
                        pattern,
                        repository_file: false,
                        bundle_path: bundle_path.clone(),
                        sha256: sha256.clone(),
                        resolved_family: Some(family.clone()),
                    });
                    continue;
                }
                let matched = fc_match(&pattern);
                let extension = matched
                    .file
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or("ttf");
                let bundle_path = format!("fonts/resolved/{}.{}", slugify(&pattern), extension);
                let sha256 = file_sha256(&matched.file);
                insert_entry(entries, bundle_path.clone(), matched.file);
                resolved_paths.insert(
                    pattern.to_lowercase(),
                    (bundle_path.clone(), sha256.clone(), matched.family.clone()),
                );
                embeddings.push(CuratedEmbedding {
                    pattern,
                    repository_file: false,
                    bundle_path,
                    sha256,
                    resolved_family: Some(matched.family),
                });
            }
        }
    }
    embeddings
}

struct MatchedFont {
    file: PathBuf,
    /// Primary family name fontconfig reports for the matched file.
    family: String,
}

fn fc_match(pattern: &str) -> MatchedFont {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{file}\n%{family}\n", pattern])
        .output()
        .unwrap_or_else(|error| {
            panic!("bundle-media resolves curated family {pattern:?} through fontconfig: {error}")
        });
    assert!(
        output.status.success(),
        "fc-match failed for curated family {pattern:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let file = lines.next().unwrap_or_default().trim().to_string();
    assert!(
        !file.is_empty(),
        "fontconfig resolved curated family {pattern:?} to no file"
    );
    let path = PathBuf::from(&file);
    assert!(
        path.is_file(),
        "fontconfig resolved curated family {pattern:?} to missing file {}",
        path.display()
    );
    // `%{family}` may report several comma-separated names; the first is
    // fontconfig's canonical answer for this pattern.
    let reported = lines.next().unwrap_or_default();
    let primary = reported.split(',').next().unwrap_or_default().trim();
    let family = if primary.is_empty() {
        pattern.to_string()
    } else {
        primary.to_string()
    };
    MatchedFont { file: path, family }
}

fn slugify(pattern: &str) -> String {
    let slug: String = pattern
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = slug.trim_matches('-');
    assert!(
        !trimmed.is_empty(),
        "curated family {pattern:?} has no usable slug"
    );
    trimmed.to_string()
}

fn file_sha256(path: &Path) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    hex(&Sha256::digest(&bytes))
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn relative_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or_else(|_| panic!("{} lives outside the crate root", path.display()))
        .to_string_lossy()
        .replace('\\', "/")
}

fn emit_bundle(root: &Path, entries: BTreeMap<String, PathBuf>, curated: Vec<CuratedEmbedding>) {
    // Content digests bind the bundle identity to exact artifact bytes, so an
    // equal-length edit can never reuse a stale blob or a stale cache.
    let content_hashes: Vec<String> = entries
        .values()
        .map(|absolute| file_sha256(absolute))
        .collect();
    let mut identity = Sha256::new();
    identity.update(b"plaque-forge.bundle-media/2\0");
    for ((relative, absolute), sha256) in entries.iter().zip(&content_hashes) {
        identity.update(relative.as_bytes());
        identity.update(0u8.to_le_bytes());
        identity.update((fs::metadata(absolute).expect("bundle metadata").len()).to_le_bytes());
        identity.update(0u8.to_le_bytes());
        identity.update(sha256.as_bytes());
    }
    let bundle_id = hex(&identity.finalize());

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let blob_length = prepare_blob(&out_dir, root, &entries, &content_hashes);

    // Only offset tables pass through rustc; the bytes themselves arrive via
    // the linked object file produced by `ld -r -b binary`.
    let mut generated = String::new();
    generated.push_str("// @generated by build.rs under the bundle-media feature. Do not edit.\n");
    let index_type = |name: &str| format!("crate::media::index::{name}");
    generated.push_str(&format!("pub const BUNDLE_ID: &str = {bundle_id:?};\n"));
    generated.push_str(&format!(
        "pub static ENTRIES: &[{}] = &[\n",
        index_type("EmbeddedAsset")
    ));
    let mut offset = 0usize;
    for relative in entries.keys() {
        let length = entry_length(root, entries.get(relative).expect("entry path"));
        generated.push_str(&format!(
            "    {} {{ path: {relative:?}, offset: {offset}, len: {length} }},\n",
            index_type("EmbeddedAsset")
        ));
        offset += length;
    }
    assert_eq!(
        offset as u64, blob_length,
        "offset table must cover the whole blob"
    );
    generated.push_str("];\n");
    generated.push_str(&format!(
        "pub static CURATED_FONT_EMBEDDINGS: &[{}] = &[\n",
        index_type("CuratedFontEmbedding")
    ));
    for font in &curated {
        generated.push_str(&format!(
            "    {} {{ pattern: {:?}, repository_file: {}, bundle_path: {:?}, sha256: {:?}, resolved_family: {:?} }},\n",
            index_type("CuratedFontEmbedding"),
            font.pattern,
            font.repository_file,
            font.bundle_path,
            font.sha256,
            font.resolved_family,
        ));
    }
    generated.push_str("];\n");

    let destination = out_dir.join("bundled_media.rs");
    fs::write(&destination, generated)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", destination.display()));

    // Link the payload object into this package's binary only: test and
    // benchmark executables compile the kilobyte tables but never the blob.
    println!(
        "cargo:rustc-link-arg-bin=plaque-forge={}",
        out_dir.join("bundle_blob.o").display()
    );
    println!(
        "cargo:bundle-media-assets={} files, {}-byte blob",
        entries.len(),
        blob_length
    );
}

/// Ensure `OUT_DIR/bundle_blob.bin` and its linked object cover exactly the
/// current embedded inputs, skipping both expensive steps when a previous
/// run already recorded success for this exact input set. The freshness
/// marker covers file names *and* content digests, so an equal-length edit
/// always regenerates the payload. The marker sidecar is written only after
/// the object exists, so a crash never leaves the pair half-built while
/// claiming freshness.
fn prepare_blob(
    out_dir: &Path,
    root: &Path,
    entries: &BTreeMap<String, PathBuf>,
    content_hashes: &[String],
) -> u64 {
    let mut digest = Sha256::new();
    for (absolute, sha256) in entries.values().zip(content_hashes) {
        digest.update(absolute.to_string_lossy().as_bytes());
        digest.update(0u8.to_le_bytes());
        digest.update(sha256.as_bytes());
    }
    let inputs_digest = hex(&digest.finalize());
    let sidecar = out_dir.join("bundle_blob.inputs");
    if fs::read_to_string(&sidecar)
        .map(|previous| previous.trim() == inputs_digest)
        .unwrap_or(false)
    {
        return fs::metadata(out_dir.join("bundle_blob.bin"))
            .expect("recorded blob is missing")
            .len();
    }

    let mut blob = Vec::with_capacity(total_embedded_bytes(root, entries) as usize);
    for absolute in entries.values() {
        blob.extend(
            fs::read(absolute)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", absolute.display())),
        );
    }
    fs::write(out_dir.join("bundle_blob.bin"), &blob)
        .unwrap_or_else(|error| panic!("failed to write bundle blob: {error}"));
    make_blob_object(out_dir);
    fs::write(&sidecar, &inputs_digest)
        .unwrap_or_else(|error| panic!("failed to record blob inputs: {error}"));
    blob.len() as u64
}

/// Turn the raw blob into a relocatable object exposing the standard
/// `_binary_*_start/_end` symbols. Symbol names come from the input path as
/// spelled on the command line, so the linker runs inside `OUT_DIR` against
/// the bare file name to keep them deterministic. Prefer lld for its small
/// memory footprint; fall back to whatever `ld` exists.
fn make_blob_object(out_dir: &Path) {
    let object = out_dir.join("bundle_blob.o");
    for candidate in ["ld.lld", "ld"] {
        let status = std::process::Command::new(candidate)
            .args(["-r", "-b", "binary"])
            .arg("bundle_blob.bin")
            .arg("-o")
            .arg(&object)
            .current_dir(out_dir)
            .status();
        match status {
            Ok(status) if status.success() => {
                println!("cargo:warning=bundle blob object built with {candidate}");
                return;
            }
            Ok(status) => {
                eprintln!("cargo:warning=blob object creation with {candidate} failed: {status}")
            }
            Err(_) => continue,
        }
    }
    panic!("no working linker found to convert the bundle blob into an object file");
}

fn entry_length(root: &Path, absolute: &Path) -> usize {
    fs::metadata(root.join(absolute))
        .expect("bundle metadata")
        .len() as usize
}

fn total_embedded_bytes(root: &Path, entries: &BTreeMap<String, PathBuf>) -> u64 {
    entries
        .values()
        .map(|path| {
            fs::metadata(root.join(path))
                .map(|meta| meta.len())
                .unwrap_or(0)
        })
        .sum()
}
