use anyhow::Result;
use clap::Parser;

mod analysis;
mod analyze;
mod build_info;
mod cli;
mod color;
mod digest;
mod geometry;
mod image_io;
mod layers;
pub mod model;
mod portable_path;
mod progress;
pub mod refinement;
mod refinement_commands;
mod render;
mod review;
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
        Command::Refine(args) => refinement_commands::refine(args),
        Command::PlacePlaque(args) => refinement_commands::place_plaque(args),
        Command::Analyze(args) => analyze::run(args),
        Command::ExportMotion(args) => refinement_commands::export_motion(args),
        Command::Segment(args) => segmentation::run(args),
        Command::Verify(args) => verify::run(args),
        Command::Review(args) => review::run(args),
        Command::MigrateAnalysis(args) => analysis::migrate_tree(&args.root, args.apply),
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
