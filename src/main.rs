use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

mod analysis;
mod analyze;
mod build_info;
mod cli;
mod color;
mod geometry;
mod image_io;
mod layers;
mod model;
mod progress;
mod refinement;
mod refinement_commands;
mod render;
mod segmentation;
mod surface;
mod verify;
mod video;
mod workspace;

use cli::{Cli, Command};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            for cause in error.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Refine(args) => refinement_commands::refine(args),
        Command::Analyze(args) => analyze::run(args),
        Command::ExportMotion(args) => refinement_commands::export_motion(args),
        Command::Segment(args) => segmentation::run(args),
        Command::Verify(args) => verify::run(args),
        Command::Render(args) => {
            let pack = args
                .analysis
                .clone()
                .map(Ok)
                .unwrap_or_else(|| workspace::analysis_path(&args.input))?;
            let output = args
                .output
                .clone()
                .map(Ok)
                .unwrap_or_else(|| workspace::output_path(&args.input))?;
            let current_refinement = refinement::current_refinement_provenance(
                &args.input,
                args.refinement.as_deref(),
                args.plaque.as_deref(),
            )?;
            let reusable_for_input = if analysis::is_analysis(&pack) && args.input.is_file() {
                let cached = analysis::Analysis::open(&pack)?;
                cached.manifest.source.sha256 == video::sha256(&args.input)?
                    && match (&cached.manifest.refinements, &current_refinement) {
                        (None, None) => true,
                        (Some(cached), Some(current)) => cached.content_matches(current),
                        _ => false,
                    }
            } else {
                false
            };
            let must_analyze = args.reanalyze || !reusable_for_input;
            if must_analyze {
                let mut analyze_args = args.as_analyze_args(pack.clone());
                analyze_args.force = args.reanalyze || pack.exists();
                analyze::run(analyze_args)?;
            }
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent)?;
            }
            render::run(args.as_compose_args(pack.clone(), output.clone()))?;
            if !args.skip_verify {
                verify::run(args.as_verify_args(pack, output))?;
            }
            Ok(())
        }
    }
}
