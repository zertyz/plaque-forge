//! Bundled-media path materialization.
//!
//! `bundle-media` binaries carry repository media inside the binary and expose
//! it through a content-addressed mirror on the filesystem. The rendering
//! pipeline (OpenCV, ffmpeg, texture loading) consumes real file paths, so
//! every read-side argument that names a canonical repository location
//! (`assets/…`, `styles/…`, `fonts/…`) is rewritten onto that mirror before the
//! workflow runs. Write paths, external executables, and homologation evidence
//! (deliberately on-disk only per `docs/BUNDLING.md`) are left untouched.
//!
//! This module owns the *reasoning* about which paths to rewrite and which
//! prefixes to pre-extract. `src/cli.rs` is a thin adapter that delegates here
//! so the same preparation can be shared with the interactive showcase and
//! any future UI without duplicating prefix knowledge.

use std::path::PathBuf;

use anyhow::Result;

use crate::cli::Command;
use crate::media::bundled::production_materializer;
use crate::media::index::{EmbeddedIndex, Materializer};

type EmbeddedIndexAlias = EmbeddedIndex;

fn file(index: &EmbeddedIndexAlias, cache: &Materializer, raw: &mut PathBuf) -> Result<()> {
    *raw = index.remap_file(cache, raw)?;
    Ok(())
}

fn optional(
    index: &EmbeddedIndexAlias,
    cache: &Materializer,
    raw: &mut Option<PathBuf>,
) -> Result<()> {
    if let Some(path) = raw {
        file(index, cache, path)?;
    }
    Ok(())
}

/// Remap an explicit analysis-directory argument onto the mirror.
fn directory(index: &EmbeddedIndexAlias, cache: &Materializer, raw: &mut PathBuf) -> Result<()> {
    if let Some(relative) = index.normalize_relative(raw) {
        let prefix = format!("{relative}/");
        index.extract_prefix(cache, &prefix)?;
        *raw = cache.root().join(&prefix);
    }
    Ok(())
}

/// Remap one source-video argument and pre-extract its scene intent plus
/// analysis cache so derived default paths resolve inside the mirror without
/// further lookups.
fn video(index: &EmbeddedIndexAlias, cache: &Materializer, input: &mut PathBuf) -> Result<()> {
    let embedded = index.lookup(input).map(|asset| asset.path);
    file(index, cache, input)?;
    if let Some(relative) = embedded {
        let stem = relative
            .strip_prefix("assets/")
            .and_then(|rest| rest.strip_suffix(".mp4"))
            .unwrap_or(relative);
        index.extract_prefix(cache, &format!("assets/scenes/{stem}/"))?;
        index.extract_prefix(cache, &format!("assets/analysis/{stem}/"))?;
    }
    Ok(())
}

fn scene_intent(
    index: &EmbeddedIndexAlias,
    cache: &Materializer,
    input: &mut PathBuf,
    scene: &mut Option<PathBuf>,
) -> Result<()> {
    video(index, cache, input)?;
    optional(index, cache, scene)
}

/// Materialize every embedded style texture; style programs reference them
/// relative to their own file inside the mirror.
fn textures(index: &EmbeddedIndexAlias, cache: &Materializer) -> Result<()> {
    index.extract_prefix(cache, "assets/textures/")?;
    Ok(())
}

/// Rewrite every read-side path in `command` onto the bundled mirror.
///
/// This is the sole place that knows which commands need which prefixes.
/// `src/cli.rs` delegates here so the CLI stays a thin adapter and the
/// interactive showcase can reuse the same preparation.
pub fn materialize_command(command: &mut Command) -> Result<()> {
    // `list` reads only generated tables, and homologation commands stay an
    // on-disk responsibility: neither may create a materialization cache as a
    // side effect (see `docs/BUNDLING.md`).
    if matches!(
        command,
        Command::List(_) | Command::Homologate(_) | Command::HomologationCoverage(_)
    ) {
        return Ok(());
    }

    let index = crate::media::bundled::index();
    let cache = production_materializer()?;

    match command {
        Command::CreateScene(args) => video(&index, &cache, &mut args.input)?,
        Command::PlaceSurface(args) => {
            video(&index, &cache, &mut args.input)?;
            file(&index, &cache, &mut args.image)?;
        }
        Command::Analyze(args) => scene_intent(&index, &cache, &mut args.input, &mut args.scene)?,
        Command::ExportTrajectory(args) => directory(&index, &cache, &mut args.analysis)?,
        Command::Segment(args) => scene_intent(&index, &cache, &mut args.input, &mut args.scene)?,
        Command::Render(args) => {
            let render = args.as_mut();
            scene_intent(&index, &cache, &mut render.input, &mut render.scene)?;
            if let Some(analysis) = render.analysis.as_mut() {
                directory(&index, &cache, analysis)?;
            }
            file(&index, &cache, &mut render.font)?;
            optional(&index, &cache, &mut render.style_file)?;
            optional(&index, &cache, &mut render.text_file)?;
            textures(&index, &cache)?;
        }
        Command::Verify(args) => {
            directory(&index, &cache, &mut args.analysis)?;
            file(&index, &cache, &mut args.rendered)?;
            optional(&index, &cache, &mut args.original)?;
        }
        Command::Review(args) => {
            directory(&index, &cache, &mut args.analysis)?;
            optional(&index, &cache, &mut args.scene)?;
            optional(&index, &cache, &mut args.verification)?;
            textures(&index, &cache)?;
        }
        Command::List(_) | Command::Homologate(_) | Command::HomologationCoverage(_) => {}
        Command::CheckAnalysisCache(_args) => {
            // The audit walks every bundled asset's source video and analysis
            // pack below `--assets-dir`.
            index.extract_prefix(&cache, "assets/")?;
        }
    }
    Ok(())
}
