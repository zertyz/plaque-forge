//! # Plaque Forge
//!
//! Plaque Forge analyzes video scenes, discovers planar writing surfaces,
//! tracks their motion across frames, detects foreground occlusions, and renders
//! styled typography overlays with subpixel spatial stability.
//!
//! ## Core Architecture
//!
//! The system is organized into modular pipelines:
//!
//! - **Application API (`application`)**: Interface-independent requests for analyze, render,
//!   verify, homologate, and homologation-coverage workflows. The CLI is an adapter over it.
//! - **Infrastructure (`infrastructure`)**: Small replaceable contracts for genuinely external
//!   process boundaries; avoid interface-for-everything abstraction.
//!
//! - **Analysis (`analyze`)**: Feature extraction, homography tracking, writable region
//!   discovery, and photometric/edge structural lock.
//! - **Scene & Refinement (`scene`)**: Manifest declarations (`scene.toml`), human intent
//!   overrides, sparse motion anchors, and layer assignments.
//! - **Segmentation (`segmentation`)**: ML-assisted and structural foreground occluder
//!   segmentation with prompt support.
//! - **Rendering (`render`)**: Line-breaking typography fitting (`typography`), effect
//!   shaders (`effects`), projective warping, and linear-light layer compositing.
//! - **Verification (`verify`)**: Automated quality scorecards measuring independent
//!   source-motion lock, rendered title stability, occlusion restoration, and trajectory
//!   dynamics.
//! - **Homologation (`homologation`)**: Executable acceptance contracts protecting
//!   previously accepted geometry, typography, provenance, and foreground ordering.
//! - **Media (`media`)**: One catalog contract listing videos, styles, plaques,
//!   textures, and fonts either from repository directories or, under the
//!   `bundle-media` feature, from data embedded inside the binary.
//! - **Showcase (`showcase`)**: Interactive `egui`/`eframe` preview (`plaque-forge-showcase`)
//!   looping videos, compositing live typography, and exposing every style
//!   parameter through a full-widget editor; also builds to `wasm` for web preview.
//! - **Safety & Staging (`staged_output`)**: Lease-held atomic file staging preventing
//!   partial or corrupted destination artifacts.

use anyhow::Result;
use clap::Parser;

pub mod analysis;
pub mod analyze;
pub mod application;
mod build_info;
mod cli;
pub mod color;
mod digest;
pub mod geometry;
pub mod homologation;
mod image_io;
pub mod infrastructure;
mod io;
mod layers;
#[cfg(feature = "bundle-media")]
mod materialize;
pub mod media;
pub mod model;
mod portable_path;
mod progress;
pub mod render;
mod review;
pub mod scene;
mod scene_commands;
mod segmentation;
pub mod segmentation_strategy;
pub mod showcase;
mod staged_output;
mod stats;
pub mod surface;
mod verify;
mod video;
pub mod workspace;
pub mod writable_region;

use cli::{Cli, Command};

/// Parse the command line and execute one Plaque Forge workflow.
///
/// Keeping dispatch in the library gives the binary and integration tests one
/// module graph instead of compiling a second copy of shared modules.
pub fn run() -> Result<()> {
    #[cfg(feature = "bundle-media")]
    let mut cli = Cli::parse();
    #[cfg(not(feature = "bundle-media"))]
    let cli = Cli::parse();
    #[cfg(feature = "bundle-media")]
    crate::materialize::materialize_command(&mut cli.command)?;
    match cli.command {
        Command::CreateScene(args) => scene_commands::create(args),
        Command::PlaceSurface(args) => scene_commands::place_surface(args),
        Command::Analyze(args) => application::analyze(args.into()),
        Command::ExportTrajectory(args) => scene_commands::export_trajectory(args),
        Command::Segment(args) => segmentation::run(args),
        Command::Verify(args) => application::verify(args.into()),
        Command::Homologate(args) => application::homologate(args.into()),
        Command::CheckAnalysisCache(args) => analysis::run_check_analysis_cache(&args.assets_dir),
        Command::HomologationCoverage(args) => {
            let report = application::homologation_coverage(args.into())?;
            print_stdout(&serde_json::to_string_pretty(&report)?)
        }
        Command::List(args) => {
            let json = args.json;
            let inventory = application::list(args.into(), production_media_catalog()?.as_ref())?;
            if json {
                print_stdout(&serde_json::to_string_pretty(&inventory)?)
            } else {
                cli::print_media_inventory(&inventory)
            }
        }
        Command::Review(args) => review::run(args),
        Command::Render(args) => {
            let analysis = args
                .analysis
                .clone()
                .map(Ok)
                .unwrap_or_else(|| workspace::analysis_path(&args.input))?;
            let output = args
                .output
                .clone()
                .map(Ok)
                .unwrap_or_else(|| workspace::output_path(&args.input))?;
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            application::render(args.into_request(analysis, output))
        }
    }
}

/// Media source for this build: embedded data under `bundle-media`,
/// repository directories otherwise.
fn production_media_catalog() -> Result<Box<dyn media::MediaCatalog>> {
    #[cfg(feature = "bundle-media")]
    {
        Ok(Box::new(media::bundled::BundledMedia::production()))
    }
    #[cfg(not(feature = "bundle-media"))]
    {
        Ok(Box::new(media::FilesystemCatalog::production()?))
    }
}

/// Print a finished report, tolerating a closed downstream pipe (`| head`).
fn print_stdout(text: &str) -> Result<()> {
    let mut out = std::io::stdout().lock();
    crate::io::write_stdout_line(&mut out, text)?;
    crate::io::flush_tolerating_broken_pipe(&mut out)
}
