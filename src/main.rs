use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;

mod analyze;
mod build_info;
mod cli;
mod color;
mod geometry;
mod image_io;
mod model;
mod progress;
mod render;
mod surface;
mod titlepack;
mod verify;
mod video;

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
        Command::Analyze(args) => analyze::run(args),
        Command::Render(args) => render::run(args),
        Command::Verify(args) => verify::run(args),
        Command::Replace(args) => {
            let pack = args
                .analysis
                .clone()
                .unwrap_or_else(|| args.output.with_extension("titlepack"));
            let reusable_for_input = if titlepack::is_titlepack(&pack) && args.input.is_file() {
                let cached = titlepack::TitlePack::open(&pack)?;
                cached.manifest.source.sha256 == video::sha256(&args.input)?
            } else {
                false
            };
            let must_analyze = args.reanalyze || !reusable_for_input;
            if must_analyze {
                let mut analyze_args = args.as_analyze_args(pack.clone());
                // Empty, stale and interrupted directories are not valid caches.
                analyze_args.force = args.reanalyze || pack.exists();
                analyze::run(analyze_args)?;
            }
            render::run(args.as_render_args(pack.clone()))?;
            if !args.skip_verify {
                verify::run(args.as_verify_args(pack))?;
            }
            Ok(())
        }
    }
}
