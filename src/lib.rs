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
//! - **Analysis (`analyze`)**: Feature extraction, homography tracking, writable region
//!   discovery, and photometric/edge structural lock.
//! - **Scene & Refinement (`scene`)**: Manifest declarations (`scene.toml`), human intent
//!   overrides, sparse motion anchors, and layer assignments.
//! - **Segmentation (`segmentation`)**: ML-assisted and structural foreground occluder
//!   segmentation with prompt support.
//! - **Rendering (`render`)**: Line-breaking typography fitting (`typography`), effect
//!   shaders (`effects`), projective warping, and linear-light layer compositing.
//! - **Verification (`verify`)**: Automated quality scorecards measuring tracking lock,
//!   temporal stability, occlusion restoration, and trajectory curvature.
//! - **Homologation (`homologation`)**: Executable acceptance contracts protecting
//!   previously accepted geometry, typography, provenance, and foreground ordering.
//! - **Safety & Staging (`staged_output`)**: Lease-held atomic file staging preventing
//!   partial or corrupted destination artifacts.

use anyhow::Result;
use clap::Parser;

mod analysis;
mod analyze;
mod build_info;
mod cli;
mod color;
mod digest;
mod geometry;
pub mod homologation;
mod image_io;
mod layers;
pub mod model;
mod portable_path;
mod progress;
mod render;
mod review;
pub mod scene;
mod scene_commands;
mod segmentation;
mod staged_output;
mod surface;
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
    let cli = Cli::parse();
    match cli.command {
        Command::CreateScene(args) => scene_commands::create(args),
        Command::PlaceSurface(args) => scene_commands::place_surface(args),
        Command::Analyze(args) => analyze::run(args),
        Command::ExportTrajectory(args) => scene_commands::export_trajectory(args),
        Command::Segment(args) => segmentation::run(args),
        Command::Verify(args) => verify::run(args),
        Command::Homologate(args) => homologation::run(args),
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
            render::run(args.as_compose_args(analysis, output))
        }
    }
}
