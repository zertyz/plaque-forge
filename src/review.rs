//! Human-oriented front door for analysis and verification diagnostics.
//!
//! Machine JSON remains available for automation; this command assembles the same
//! evidence into a compact HTML page so a reviewer can decide where visual refinement
//! is needed without opening a directory of unrelated artifacts.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::{analysis::Analysis, cli::ReviewArgs};

pub fn run(args: ReviewArgs) -> Result<()> {
    let analysis = Analysis::open(&args.analysis)
        .with_context(|| format!("failed to open analysis cache {}", args.analysis.display()))?;
    let diagnostics = analysis.root.join("diagnostics");
    fs::create_dir_all(&diagnostics)?;
    let output = args
        .output
        .unwrap_or_else(|| diagnostics.join("review.html"));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

    let summary = read_json(&analysis.root.join("analysis-summary.json"))?;
    let occlusion = read_json_optional(&diagnostics.join("occlusion-summary.json"))?;
    let verification = match args.verification.as_deref() {
        Some(path) => Some(read_json(path)?),
        None => None,
    };
    let render_manifest = match args.render_manifest.as_deref() {
        Some(path) => Some(read_json(path)?),
        None => None,
    };

    let html = build_report(
        &analysis.root,
        &diagnostics,
        &summary,
        occlusion.as_ref(),
        verification.as_ref(),
        render_manifest.as_ref(),
    );
    fs::write(&output, html)
        .with_context(|| format!("failed to write review report {}", output.display()))?;
    println!("{}", output.display());
    Ok(())
}

fn read_json(path: &Path) -> Result<Value> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("invalid JSON in {}", path.display()))
}

fn read_json_optional(path: &Path) -> Result<Option<Value>> {
    if path.is_file() {
        read_json(path).map(Some)
    } else {
        Ok(None)
    }
}

fn build_report(
    analysis_root: &Path,
    diagnostics: &Path,
    summary: &Value,
    occlusion: Option<&Value>,
    verification: Option<&Value>,
    render_manifest: Option<&Value>,
) -> String {
    let mut body = String::new();
    body.push_str("<h1>Plaque Forge review</h1>");
    body.push_str(&format!(
        "<p class=path>Analysis: <code>{}</code></p>",
        escape_html(&analysis_root.display().to_string())
    ));
    body.push_str("<p>This page is for visual triage. Green metrics usually need no attention; amber values deserve inspection; red values should be refined before typography tuning.</p>");

    body.push_str("<h2>Analysis health</h2><div class=metrics>");
    metric(&mut body, "Overall", number(summary, "overall"), 0.90, 0.75);
    metric(
        &mut body,
        "Plaque detection",
        number(summary, "candidate_confidence"),
        0.90,
        0.75,
    );
    metric(
        &mut body,
        "Motion tracking",
        number(summary, "motion_confidence"),
        0.90,
        0.75,
    );
    metric(
        &mut body,
        "Structural reference",
        number(summary, "structural_confidence"),
        0.90,
        0.75,
    );
    metric(
        &mut body,
        "Occlusion",
        number(summary, "occlusion_confidence"),
        0.85,
        0.70,
    );
    body.push_str("</div>");

    if let Some(occlusion) = occlusion {
        body.push_str("<h2>Foreground / occlusion</h2>");
        body.push_str("<p>These are scene-complexity facts, not quality grades. Heavy foreground coverage can be perfectly correct; use the occlusion confidence and rendered-video verification above to judge whether it was handled well.</p><div class=metrics>");
        metric_neutral(
            &mut body,
            "Mean content occlusion",
            number(occlusion, "mean_content_occlusion"),
        );
        metric_neutral(
            &mut body,
            "Maximum coverage",
            number(occlusion, "max_coverage"),
        );
        metric_neutral(
            &mut body,
            "Minimum plaque visibility",
            number(occlusion, "minimum_plaque_visibility"),
        );
        body.push_str("</div>");
    }

    if let Some(manifest) = render_manifest {
        append_typography(&mut body, manifest);
    }

    if let Some(verification) = verification {
        body.push_str("<h2>Rendered-video verification</h2><div class=metrics>");
        for (label, key) in [
            ("Overall", "overall"),
            ("Tracking lock", "tracking_lock"),
            ("Scene integrity", "scene_integrity"),
            ("Typography fit", "typography_fit"),
            ("Typography validity", "typography_validity"),
            ("Temporal stability", "temporal_stability"),
            ("Occlusion restore", "occlusion_restore"),
            ("Loop seam", "loop_seam"),
        ] {
            let green = verification_threshold(verification, key).unwrap_or(0.95);
            let amber = (green - 0.08).max(0.0);
            metric(&mut body, label, number(verification, key), green, amber);
        }
        body.push_str("</div>");
        append_string_array(
            &mut body,
            "Verification failures",
            verification.get("failures"),
        );
        append_string_array(
            &mut body,
            "Suggested remedies",
            verification.get("remedies"),
        );
    }

    body.push_str("<h2>Where to look</h2><div class=gallery>");
    diagnostic_image(
        &mut body,
        diagnostics.join("candidate.png"),
        "Detected plaque",
        "Confirm the selected plaque and its boundary before tuning anything downstream.",
    );
    diagnostic_image(
        &mut body,
        diagnostics.join("tracking-contact-sheet.jpg"),
        "Tracking contact sheet",
        "Look for drift, scale jumps, or perspective errors across the clip.",
    );
    diagnostic_image(
        &mut body,
        diagnostics.join("canonical-reference.png"),
        "Canonical plaque",
        "Inspect the recovered writing surface for texture damage, residual old text, or unstable reconstruction.",
    );
    diagnostic_image(
        &mut body,
        diagnostics.join("temporal-mad.png"),
        "Temporal change map",
        "Bright regions identify areas that vary over time and may need occlusion or reconstruction attention.",
    );
    body.push_str("</div>");

    append_string_array(&mut body, "Analysis remedies", summary.get("remedies"));

    body.push_str("<h2>Decision order</h2><ol><li>Fix plaque selection or tracking first.</li><li>Fix foreground/occlusion restoration next.</li><li>Fix canonical plaque reconstruction next.</li><li>Only then tune line layout, font, materials, and text effects.</li></ol>");

    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Plaque Forge review</title><style>{}</style></head><body>{}</body></html>",
        CSS, body
    )
}

fn append_typography(body: &mut String, manifest: &Value) {
    let Some(typography) = manifest.get("typography") else {
        return;
    };
    body.push_str("<h2>Typography / text style</h2><div class=metrics>");
    metric_neutral(body, "Font size", number(typography, "font_size"));
    metric_neutral(
        body,
        "Maximum safe size",
        number(typography, "maximum_safe_font_size"),
    );
    metric_neutral(body, "Fill ratio", number(typography, "fill_ratio"));
    metric_neutral(
        body,
        "Lines",
        typography
            .get("lines")
            .and_then(Value::as_u64)
            .map(|value| value as f64),
    );
    body.push_str("</div>");

    if let Some(mode) = typography.get("fit_mode").and_then(Value::as_str) {
        body.push_str(&format!(
            "<p><strong>Fit:</strong> <code>{}</code></p>",
            escape_html(mode)
        ));
    }
    if let Some(font) = manifest
        .get("font_file")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        body.push_str(&format!(
            "<p><strong>Font:</strong> <code>{}</code></p>",
            escape_html(font)
        ));
    }
    if let Some(style) = manifest
        .get("text_style")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
    {
        body.push_str(&format!(
            "<p><strong>Resolved style:</strong> <code>{}</code></p>",
            escape_html(style)
        ));
    }

    let original = manifest
        .get("title_text")
        .and_then(Value::as_str)
        .unwrap_or("");
    let resolved = typography
        .get("resolved_text")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(original);
    if !resolved.is_empty() {
        body.push_str("<p><strong>Resolved line layout:</strong></p>");
        body.push_str(&format!("<pre>{}</pre>", escape_html(resolved)));
    }
}

fn diagnostic_image(body: &mut String, path: PathBuf, title: &str, guidance: &str) {
    if !path.is_file() {
        return;
    }
    let url = file_url(&path);
    body.push_str(&format!(
        "<figure><figcaption><strong>{}</strong><br>{}</figcaption><a href=\"{}\"><img src=\"{}\" loading=lazy></a></figure>",
        escape_html(title),
        escape_html(guidance),
        escape_html(&url),
        escape_html(&url),
    ));
}

fn file_url(path: &Path) -> String {
    let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let raw = absolute.to_string_lossy();
    let mut encoded = String::with_capacity(raw.len() + 8);
    for byte in raw.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    format!("file://{encoded}")
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn metric(body: &mut String, label: &str, value: Option<f64>, green: f64, amber: f64) {
    let Some(value) = value else {
        return;
    };
    let class = if value >= green {
        "good"
    } else if value >= amber {
        "warn"
    } else {
        "bad"
    };
    body.push_str(&format!(
        "<div class=\"metric {}\"><span>{}</span><strong>{:.3}</strong></div>",
        class,
        escape_html(label),
        value,
    ));
}

fn metric_neutral(body: &mut String, label: &str, value: Option<f64>) {
    let Some(value) = value else {
        return;
    };
    body.push_str(&format!(
        "<div class=\"metric neutral\"><span>{}</span><strong>{:.3}</strong></div>",
        escape_html(label),
        value,
    ));
}

fn verification_threshold(verification: &Value, key: &str) -> Option<f64> {
    verification
        .get("thresholds")
        .and_then(|thresholds| thresholds.get(key))
        .and_then(Value::as_f64)
}

fn append_string_array(body: &mut String, title: &str, value: Option<&Value>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    if items.is_empty() {
        return;
    }
    body.push_str(&format!("<h2>{}</h2><ul>", escape_html(title)));
    for item in items.iter().filter_map(Value::as_str) {
        body.push_str(&format!("<li>{}</li>", escape_html(item)));
    }
    body.push_str("</ul>");
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const CSS: &str = r#"
:root { color-scheme: dark; font-family: system-ui, sans-serif; background:#11151a; color:#e9eef3; }
body { max-width: 1180px; margin: 0 auto; padding: 32px; line-height: 1.45; }
h1,h2 { letter-spacing: .01em; } h2 { margin-top: 34px; }
.path { color:#aeb9c4; } code { color:#d8e3ec; }
.metrics { display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:10px; }
.metric { border:1px solid #3a4651; border-left-width:6px; border-radius:8px; padding:12px; background:#171d23; display:flex; justify-content:space-between; gap:16px; }
.metric.good { border-left-color:#54c987; } .metric.warn { border-left-color:#d7ae4a; } .metric.bad { border-left-color:#df6363; } .metric.neutral { border-left-color:#71808e; }
.gallery { display:grid; grid-template-columns:repeat(auto-fit,minmax(420px,1fr)); gap:18px; }
figure { margin:0; border:1px solid #33404b; border-radius:10px; overflow:hidden; background:#171d23; }
figcaption { padding:12px 14px; color:#c6d0d9; } img { display:block; width:100%; height:auto; background:#090b0d; }
pre { padding:14px; border:1px solid #33404b; border-radius:8px; background:#0d1115; white-space:pre-wrap; font-size:1rem; }
li { margin:.45em 0; }
"#;
