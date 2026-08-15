use std::{ffi::OsString, path::{Path, PathBuf}};

use anyhow::Result;

use plaque_forge::application::{
    AnalyzeRequest, ApplicationServices, FitMode, HomologateRequest, HomologationCoverageRequest,
    RenderRequest, TitleSource, VerifyRequest,
};
use plaque_forge::infrastructure::{CommandExecutor, CommandOutput, CommandStatus};

struct DeterministicCommands;

impl CommandExecutor for DeterministicCommands {
    fn output(&self, _program: &Path, _args: &[OsString]) -> Result<CommandOutput> {
        Ok(CommandOutput {
            success: true,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn status(&self, _program: &Path, _args: &[OsString]) -> Result<CommandStatus> {
        Ok(CommandStatus {
            success: true,
            code: Some(0),
        })
    }
}

#[test]
fn programmatic_requests_have_valid_interface_independent_defaults() {
    let analyze = AnalyzeRequest::text_free("assets/example.mp4");
    assert!(analyze.source_is_text_free);
    assert_eq!(analyze.minimum_analysis_confidence, 0.70);

    let render = RenderRequest::new(
        "assets/example.mp4",
        "assets/analysis/example",
        "output/example.mkv",
        TitleSource::Text("Example title".to_string()),
        "font.ttf",
    );
    assert!(matches!(render.fit, FitMode::Artistic));
    assert_eq!(render.output, PathBuf::from("output/example.mkv"));

    let verify = VerifyRequest::new("assets/analysis/example", "output/example.mkv");
    assert_eq!(verify.minimum_score, 0.95);

    let homologate = HomologateRequest::new(
        "assets/homologation/example/contract.toml",
        "output/example.mkv",
    );
    assert!(homologate.diagnostics.is_none());

    let coverage = HomologationCoverageRequest::new("assets/homologation/capabilities.toml");
    assert!(!coverage.require_complete);

    let commands = DeterministicCommands;
    let _services = ApplicationServices::new(&commands);
}

#[test]
fn title_source_cannot_represent_text_and_file_simultaneously() {
    let text = TitleSource::Text("one source".to_string());
    assert!(matches!(text, TitleSource::Text(_)));
    let file = TitleSource::File("title.txt".into());
    assert!(matches!(file, TitleSource::File(_)));
}
