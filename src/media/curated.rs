//! Curated font selection shared by listings and `bundle-media` embedding.
//!
//! The authoritative list lives in `styles/curated_fonts`. A repository-pinned
//! entry names a file below `fonts/`; any other entry is a system fontconfig
//! family pattern resolved by the listing backend (or, for bundled builds, by
//! the building machine at compile time).

use std::path::{Component, Path};

use anyhow::{Result, bail};

/// One validated entry of the curated font list, in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CuratedFont {
    /// A repository-pinned font file below `fonts/`.
    Repository { path: String },
    /// A fontconfig family pattern resolved against installed fonts.
    Family { pattern: String },
}

impl CuratedFont {
    /// Stable identifier used as the display label and dedupe key.
    pub fn label(&self) -> &str {
        match self {
            Self::Repository { path } => path,
            Self::Family { pattern } => pattern,
        }
    }
}

/// Parse curated-font entries, rejecting anything unsafe to embed or ambiguous.
pub fn parse_curated_fonts(source: &str) -> Result<Vec<CuratedFont>> {
    let mut entries = Vec::new();
    let mut seen_labels = std::collections::BTreeSet::new();
    for (index, raw_line) in source.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let entry = if line.contains('/') || line.contains('\\') {
            CuratedFont::validate_repository_path(line)?;
            CuratedFont::Repository {
                path: line.to_string(),
            }
        } else {
            CuratedFont::Family {
                pattern: line.to_string(),
            }
        };
        let lowered = entry.label().to_lowercase();
        if !seen_labels.insert(lowered) {
            bail!(
                "duplicate curated font entry on line {}: {}",
                index + 1,
                line
            );
        }
        entries.push(entry);
    }
    Ok(entries)
}

impl CuratedFont {
    fn validate_repository_path(line: &str) -> Result<()> {
        let invalid = |reason: &str| -> Result<()> {
            bail!("curated font path {line:?} is invalid: {reason}")
        };
        if line.starts_with('/') || Path::new(line).is_absolute() {
            return invalid("absolute paths are not portable");
        }
        if line.contains('\\') {
            return invalid("backslash separators are not portable");
        }
        let mut components = Vec::new();
        for component in Path::new(line).components() {
            match component {
                Component::Normal(name) => components.push(name.to_string_lossy().into_owned()),
                _ => return invalid("only plain `fonts/<file>` paths are accepted"),
            }
        }
        if components.len() != 2 || components[0] != "fonts" {
            return invalid("repository entries must live directly below fonts/");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Result<Vec<CuratedFont>> {
        parse_curated_fonts(source)
    }

    #[test]
    fn parses_hybrid_entries_in_order_ignoring_comments_and_blanks() {
        let entries = parse(
            "# header comment\n\
             \n\
             fonts/NotoSerif-Regular.ttf\n\
             Noto Serif\n  \n\
             Noto Sans Mono   # trailing words are part of family patterns\n",
        )
        .unwrap();
        assert_eq!(
            entries,
            vec![
                CuratedFont::Repository {
                    path: "fonts/NotoSerif-Regular.ttf".into()
                },
                CuratedFont::Family {
                    pattern: "Noto Serif".into()
                },
                CuratedFont::Family {
                    pattern: "Noto Sans Mono   # trailing words are part of family patterns".into()
                },
            ]
        );
    }

    #[test]
    fn rejects_unportable_or_outside_repository_paths() {
        for bad in [
            "/usr/share/fonts/noto/NotoSerif-Regular.ttf",
            "fonts/../secrets.key",
            "assets/textures/gilded-marble.png",
            "styles\\backslash.ttf",
            "fonts/too/deep/font.ttf",
        ] {
            let result = parse(bad);
            assert!(result.is_err(), "expected {bad:?} to be rejected");
        }
    }

    #[test]
    fn rejects_duplicate_entries_case_insensitively_per_kind() {
        let duplicated_path = "fonts/A.ttf\nfonts/a.ttf\n";
        assert!(parse(duplicated_path).is_err());
        let duplicated_family = "Noto Serif\nnoto serif\n";
        assert!(parse(duplicated_family).is_err());
    }

    #[test]
    fn empty_and_comment_only_lists_are_valid_but_empty() {
        assert_eq!(parse("").unwrap(), vec![]);
        assert_eq!(parse("# nothing curated yet\n").unwrap(), vec![]);
    }
}
