use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("plaque-forge-showcase – interactive typographic preview");
        println!();
        println!("USAGE: plaque-forge-showcase [--help] [--screenshot <dir>]");
        println!();
        println!("Keys:");
        println!("  PgUp/PgDown  next/prev video (loops)");
        println!("  Enter        edit title text");
        println!(
            "  /            font picker (↑/↓ preview, type to search, Enter confirm, Esc revert)"
        );
        println!("  ↑/↓          cycle styles (discarding draft)");
        println!("  i            cycle inspect overlays (yellow plaque, green foreground, etc.)");
        println!("  Shift+I      multi-overlay checklist");
        println!("  d            demo random curated fonts + styles (Esc exits)");
        println!("  s / Save     save draft to styles/<name>_custom.toml");
        println!();
        println!("When analysis is missing, video is greyscale with:");
        println!(
            "  \"No analysis data for this video\\nConsult the documentation on how to generate it\""
        );
        return ExitCode::SUCCESS;
    }
    if let Some(pos) = args.iter().position(|a| a == "--screenshot") {
        let dir = args
            .get(pos + 1)
            .map(|s| s.as_str())
            .unwrap_or("/tmp/plaque-forge-showcase-screenshots");
        let dir = std::path::PathBuf::from(dir);
        match plaque_forge::showcase::screenshots::simulate_navigation_and_capture(&dir) {
            Ok(()) => {
                println!("screenshots saved to {}", dir.display());
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                eprintln!("screenshot error: {e:#}");
                return ExitCode::FAILURE;
            }
        }
    }
    match plaque_forge::showcase::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
