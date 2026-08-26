pub mod diagnostics;
pub mod fonts;
pub mod preview;
pub mod screenshots;
pub mod state;
pub mod styles;
pub mod video;

#[cfg(feature = "showcase")]
pub mod app;

#[cfg(feature = "showcase")]
pub use app::ShowcaseApp;

#[cfg(feature = "showcase")]
use anyhow::Result;

#[cfg(feature = "showcase")]
pub fn run() -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Plaque Forge — Showcase",
        options,
        Box::new(|cc| Ok(Box::new(ShowcaseApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;
    Ok(())
}

#[cfg(not(feature = "showcase"))]
pub fn run() -> anyhow::Result<()> {
    anyhow::bail!("showcase feature not enabled; rebuild with --features showcase")
}
