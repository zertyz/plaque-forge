//! Human-oriented front door for analysis and verification diagnostics.
//!
//! Machine JSON remains canonical for automation. This module turns the same evidence
//! into a short triage report that says what deserves human attention first. It also
//! accepts compact retained failure directories, because failed quality gates are exactly
//! when a useful review page matters most.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{
    cli::ReviewArgs,
    render::{RenderDecisionTrace, RenderManifest},
    scene::Scene,
};

#[derive(Debug, Clone)]
struct FocusItem {
    title: &'static str,
    evidence: String,
    action: &'static str,
}

struct ReportInputs<'a> {
    report_path: &'a Path,
    analysis_root: &'a Path,
    diagnostics: &'a Path,
    summary: &'a Value,
    occlusion: Option<&'a Value>,
    candidates: Option<&'a Value>,
    verification: Option<&'a Value>,
    render_manifest: Option<&'a Value>,
    decision_trace: Option<&'a RenderDecisionTrace>,
    scene: Option<&'a (PathBuf, Scene)>,
    focus: &'a [FocusItem],
}

pub fn run(args: ReviewArgs) -> Result<()> {
    let analysis_root = args.analysis;
    if !analysis_root.is_dir() {
        bail!(
            "analysis directory does not exist: {}",
            analysis_root.display()
        );
    }
    let diagnostics = analysis_root.join("diagnostics");
    fs::create_dir_all(&diagnostics)?;
    let output = args
        .output
        .unwrap_or_else(|| diagnostics.join("review.html"));
    let text_output = output.with_extension("txt");

    let summary = read_json_optional(&analysis_root.join("analysis-summary.json"))?
        .unwrap_or_else(|| Value::Object(Default::default()));
    let occlusion = read_json_optional(&diagnostics.join("occlusion-summary.json"))?;
    let candidates = read_json_optional(&diagnostics.join("candidate-ranking.json"))?;
    let verification = match args.verification.as_deref() {
        Some(path) => Some(read_json(path)?),
        None => None,
    };
    let render_manifest = match args.render_manifest.as_deref() {
        Some(path) => Some(read_json(path)?),
        None => None,
    };
    let decision_trace = match args.render_manifest.as_deref() {
        Some(path) => {
            let bytes = fs::read(path)
                .with_context(|| format!("failed to read render manifest {}", path.display()))?;
            let manifest: RenderManifest = serde_json::from_slice(&bytes)
                .with_context(|| format!("invalid render manifest {}", path.display()))?;
            Some(crate::render::load_decision_trace(path, &manifest)?)
        }
        None => None,
    };
    validate_verification_provenance(
        args.verification.as_deref(),
        verification.as_ref(),
        args.render_manifest.as_deref(),
        render_manifest.as_ref(),
    )?;
    let scene = match args.scene.as_deref() {
        Some(path) if path.is_file() => Some((path.to_path_buf(), Scene::load(path)?)),
        _ => None,
    };

    let focus = focus_items(&summary, verification.as_ref());
    let html = build_report(ReportInputs {
        report_path: &output,
        analysis_root: &analysis_root,
        diagnostics: &diagnostics,
        summary: &summary,
        occlusion: occlusion.as_ref(),
        candidates: candidates.as_ref(),
        verification: verification.as_ref(),
        render_manifest: render_manifest.as_ref(),
        decision_trace: decision_trace.as_ref(),
        scene: scene.as_ref(),
        focus: &focus,
    });
    let text = build_text_summary(
        &text_output,
        &analysis_root,
        &summary,
        scene.as_ref(),
        &focus,
    );
    let staged = crate::staged_output::create(&output)?;
    let output_name = output
        .file_name()
        .context("review output has no file name")?;
    let text_name = text_output
        .file_name()
        .context("review text output has no file name")?;
    let staged_html = staged.path().join(output_name);
    let staged_text = staged.path().join(text_name);
    fs::write(&staged_html, html)
        .with_context(|| format!("failed to stage review report {}", output.display()))?;
    fs::write(&staged_text, text)
        .with_context(|| format!("failed to stage review summary {}", text_output.display()))?;
    staged.commit_files(
        &[
            (staged_text, text_output.clone()),
            (staged_html, output.clone()),
        ],
        true,
    )?;

    println!("review: {}", output.display());
    println!("summary: {}", text_output.display());
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

fn validate_verification_provenance(
    verification_path: Option<&Path>,
    verification: Option<&Value>,
    render_manifest_path: Option<&Path>,
    render_manifest: Option<&Value>,
) -> Result<()> {
    let Some(verification) = verification else {
        return Ok(());
    };
    let verification_path = verification_path.context("verification path is unavailable")?;
    let render_manifest_path = render_manifest_path.with_context(|| {
        format!(
            "verification {} cannot be reviewed without its render manifest",
            verification_path.display()
        )
    })?;
    let render_manifest = render_manifest.context("render manifest is unavailable")?;
    let manifest_bytes = fs::read(render_manifest_path).with_context(|| {
        format!(
            "failed to read render manifest {}",
            render_manifest_path.display()
        )
    })?;
    let manifest_sha256 = crate::digest::bytes_sha256(&manifest_bytes);

    require_matching_identity(
        verification,
        "render_manifest_sha256",
        &manifest_sha256,
        verification_path,
    )?;
    for field in [
        "source_sha256",
        "analysis_manifest_sha256",
        "rendered_sha256",
    ] {
        let expected = render_manifest
            .get(field)
            .and_then(Value::as_str)
            .with_context(|| {
                format!(
                    "render manifest {} is missing {field}",
                    render_manifest_path.display()
                )
            })?;
        require_matching_identity(verification, field, expected, verification_path)?;
    }
    Ok(())
}

fn require_matching_identity(
    verification: &Value,
    field: &str,
    expected: &str,
    verification_path: &Path,
) -> Result<()> {
    let actual = verification
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| {
            format!(
                "verification {} is missing {field}",
                verification_path.display()
            )
        })?;
    if actual != expected {
        bail!(
            "verification {} is stale: {field} does not match the supplied render manifest",
            verification_path.display()
        );
    }
    Ok(())
}

fn focus_items(summary: &Value, verification: Option<&Value>) -> Vec<FocusItem> {
    let mut items = Vec::new();
    let candidate = number(summary, "candidate_confidence");
    let motion = number(summary, "motion_confidence");
    let structural = number(summary, "structural_confidence");
    let occlusion = number(summary, "occlusion_confidence");

    if number(summary, "overall").is_none() {
        items.push(FocusItem {
            title: "Analysis stopped before the final quality summary",
            evidence: "only early-stage diagnostics were produced".to_string(),
            action: "Start with the selected-surface/candidate evidence below. If selection is correct, rerun after addressing the explicit analyzer error rather than inventing downstream scenes.",
        });
    }

    if candidate.is_some_and(|value| value < 0.75) {
        items.push(FocusItem {
            title: "Writing-surface selection",
            evidence: format!("candidate confidence {:.3}", candidate.unwrap_or_default()),
            action: "Confirm the intended surface in candidate.png. If it is wrong, edit only reference_frame/bounds or writable_region in scene.toml.",
        });
    }
    if motion.is_some_and(|value| value < 0.75) {
        items.push(FocusItem {
            title: "Motion tracking",
            evidence: format!("tracking confidence {:.3}", motion.unwrap_or_default()),
            action: "Inspect tracking-contact-sheet.png. Add sparse [[surfaces.anchors]] measurements only at frames where the tracked surface is visibly wrong.",
        });
    }
    if structural.is_some_and(|value| value < 0.65) {
        items.push(FocusItem {
            title: "Writable surface / canonical reconstruction",
            evidence: format!("structural confidence {:.3}", structural.unwrap_or_default()),
            action: "Inspect canonical-reference.png and temporal-mad.png. If the intended surface is smooth or irregular, declare its writable_region rather than inventing dense motion data.",
        });
    }
    if occlusion.is_some_and(|value| value < 0.70) {
        items.push(FocusItem {
            title: "Foreground crossing",
            evidence: format!("occlusion confidence {:.3}", occlusion.unwrap_or_default()),
            action: "Check whether objects that cross the plaque are restored cleanly. Add one sparse foreground segmentation prompt only when the automatic mask misses the object.",
        });
    }

    if let Some(verification) = verification {
        let tracking = number(verification, "tracking_lock");
        let title_plane = number(verification, "rendered_title_plane_lock");
        let typography = number(verification, "typography_fit");
        let temporal = number(verification, "temporal_stability");
        if tracking.is_some_and(|value| value < 0.95)
            && !items.iter().any(|item| item.title == "Motion tracking")
        {
            let supported = verification
                .get("source_flow_uses_writing_surface_support")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let p95 = number(verification, "source_flow_p95_error_pixels");
            let p99 = number(verification, "source_flow_p99_error_pixels");
            let pairs = verification
                .get("source_flow_observed_pairs")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            items.push(FocusItem {
                title: "Rendered tracking lock",
                evidence: format!(
                    "verification tracking score {:.3}; independent {} p95 {}, p99 {} across {pairs} observations",
                    tracking.unwrap_or_default(),
                    if supported { "surface-supported material flow" } else { "source flow" },
                    display_pixels(p95),
                    display_pixels(p99),
                ),
                action: "Inspect the independent source-flow worst frame and tracking-contact-sheet.png before changing typography.",
            });
        }
        if title_plane.is_some_and(|value| value < 0.96) {
            items.push(FocusItem {
                title: "Rendered title-to-plaque lock",
                evidence: format!(
                    "source-subtracted title-plane score {:.3}",
                    title_plane.unwrap_or_default()
                ),
                action: "Inspect the reported worst frame: the actual rendered title does not remain fixed in expected plaque coordinates.",
            });
        }
        if typography.is_some_and(|value| value < 0.95) {
            items.push(FocusItem {
                title: "Typography fit",
                evidence: format!("typography verification {:.3}", typography.unwrap_or_default()),
                action: "Only after scene geometry is correct, inspect resolved line breaks, font size, padding, and style in the typography section below.",
            });
        }
        if temporal.is_some_and(|value| value < 0.95) {
            items.push(FocusItem {
                title: "Temporal stability",
                evidence: format!("temporal verification {:.3}", temporal.unwrap_or_default()),
                action: "Inspect the worst verification frames for flicker, mask popping, or effect animation discontinuities.",
            });
        }
    }

    if items.is_empty() {
        let verification_passed = verification
            .and_then(|report| report.get("passed"))
            .and_then(Value::as_bool)
            == Some(true);
        items.push(FocusItem {
            title: if verification_passed {
                "Automated checks pass; visual review remains required"
            } else {
                "No obvious automated scene-analysis blocker"
            },
            evidence: if verification_passed {
                "independent source evidence and rendered-video checks meet their thresholds; these checks cannot certify artistic judgment"
            } else {
                "the available analysis/verification metrics are above the triage thresholds"
            }
            .to_string(),
            action: "Inspect motion-relative contact sheets and the rendered video for plaque drift, depth inversions, matte lag, and timid or awkward type before accepting the asset.",
        });
    }
    items
}

fn build_report(inputs: ReportInputs<'_>) -> String {
    let ReportInputs {
        report_path,
        analysis_root,
        diagnostics,
        summary,
        occlusion,
        candidates,
        verification,
        render_manifest,
        decision_trace,
        scene,
        focus,
    } = inputs;
    let mut body = String::new();
    body.push_str("<h1>Plaque Forge review</h1>");
    body.push_str(&format!(
        "<p class=path>Analysis: <code>{}</code></p>",
        escape_html(&portable_display(report_path, analysis_root))
    ));
    if !analysis_root.join("manifest.toml").is_file() {
        body.push_str("<p class=partial><strong>Incomplete analysis.</strong> The work tree was cleaned after the quality gate stopped this run; only compact evidence is retained for human triage.</p>");
    }

    body.push_str("<h2>Focus first</h2><div class=focus>");
    for (index, item) in focus.iter().enumerate() {
        body.push_str(&format!(
            "<article><strong>{}. {}</strong><p>{}</p><p class=action>{}</p></article>",
            index + 1,
            escape_html(item.title),
            escape_html(&item.evidence),
            escape_html(item.action),
        ));
    }
    body.push_str("</div>");

    body.push_str("<h2>Analysis health</h2><div class=metrics>");
    metric(&mut body, "Overall", number(summary, "overall"), 0.90, 0.75);
    metric(
        &mut body,
        "Surface detection",
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
        "Canonical surface",
        number(summary, "structural_confidence"),
        0.90,
        0.65,
    );
    metric(
        &mut body,
        "Occlusion",
        number(summary, "occlusion_confidence"),
        0.85,
        0.70,
    );
    body.push_str("</div>");

    append_actionable_commands(&mut body, report_path, analysis_root, scene);
    append_ml_status(&mut body, summary);
    append_scene_owned(&mut body, report_path, scene);
    append_candidate_ranking(&mut body, candidates);
    append_coordinate_helper(
        &mut body,
        report_path,
        diagnostics.join("candidate.png"),
        selected_candidate_frame(candidates).unwrap_or(0),
    );

    if let Some(occlusion) = occlusion {
        body.push_str("<h2>Foreground / occlusion</h2>");
        body.push_str("<p>Coverage is scene complexity, not a failure by itself. The important question is whether crossings are restored cleanly.</p><div class=metrics>");
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
    if let Some(trace) = decision_trace {
        append_decision_trace(&mut body, trace);
    }

    if let Some(verification) = verification {
        body.push_str("<h2>Rendered-video verification</h2><div class=metrics>");
        for (label, key) in [
            ("Overall", "overall"),
            ("Tracking lock", "tracking_lock"),
            ("Rendered title-plane lock", "rendered_title_plane_lock"),
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
        metric_neutral(
            &mut body,
            "Raw trajectory curvature stability",
            number(verification, "trajectory_curvature_stability"),
        );
        body.push_str("</div>");
        let supported = verification
            .get("source_flow_uses_writing_surface_support")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let supported_frames = verification
            .get("source_flow_writing_surface_supported_frames")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let fallback_frames = verification
            .get("source_flow_writing_surface_fallback_frames")
            .and_then(Value::as_u64)
            .unwrap_or_default();
        body.push_str(if supported {
            "<h3>Independent source-motion evidence</h3><p>This is measured from lossless decoded source material at consecutive and longer frame baselines. A writing-surface matte constrains membership only on frames whose area and overlap agree with the independently predicted rigid plane; rejected/dropout frames fall back to foreground-aware plaque-region evidence. The matte is never mistaken for four-corner geometry. This neither trusts analyzer residuals nor registers the rendered title.</p><div class=metrics>"
        } else {
            "<h3>Independent source-motion evidence</h3><p>This is measured from lossless decoded source pixels at consecutive and longer frame baselines. It neither trusts the analyzer's residuals nor registers the rendered title.</p><div class=metrics>"
        });
        metric_neutral(
            &mut body,
            "Observed frame pairs",
            verification
                .get("source_flow_observed_pairs")
                .and_then(Value::as_u64)
                .map(|value| value as f64),
        );
        if supported_frames + fallback_frames > 0 {
            metric_neutral(
                &mut body,
                "Support-qualified frames",
                Some(supported_frames as f64),
            );
            metric_neutral(
                &mut body,
                "Support-fallback frames",
                Some(fallback_frames as f64),
            );
        }
        for (label, key) in [
            ("Median error (px)", "source_flow_median_error_pixels"),
            ("p95 error (px)", "source_flow_p95_error_pixels"),
            ("p99 error (px)", "source_flow_p99_error_pixels"),
            (
                "Median inlier fraction",
                "source_flow_median_inlier_fraction",
            ),
            (
                "Median spatial coverage",
                "source_flow_median_spatial_coverage",
            ),
        ] {
            metric_neutral(&mut body, label, number(verification, key));
        }
        metric_neutral(
            &mut body,
            "Worst frame",
            verification
                .get("source_flow_worst_frame")
                .and_then(Value::as_u64)
                .map(|value| value as f64),
        );
        body.push_str("</div>");
        body.push_str("<h4>Residual by temporal baseline</h4><p>Lag 1 catches a one-frame slip; lags 6 and 12 expose a title that drifts slowly or remains screen-fixed while the plaque moves.</p><table><thead><tr><th>Lag</th><th>Observed pairs</th><th>p95 error</th><th>p99 error</th></tr></thead><tbody>");
        for lag in [1, 6, 12] {
            let pairs = verification
                .get(format!("source_flow_lag_{lag}_observed_pairs"))
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let p95 = number(
                verification,
                &format!("source_flow_lag_{lag}_p95_error_pixels"),
            );
            let p99 = number(
                verification,
                &format!("source_flow_lag_{lag}_p99_error_pixels"),
            );
            body.push_str(&format!(
                "<tr><td>{lag} frame{}</td><td>{pairs}</td><td>{}</td><td>{}</td></tr>",
                if lag == 1 { "" } else { "s" },
                escape_html(&display_pixels(p95)),
                escape_html(&display_pixels(p99)),
            ));
        }
        body.push_str("</tbody></table>");
        if let Some(basis) = verification
            .get("tracking_lock_basis")
            .and_then(Value::as_str)
        {
            body.push_str(&format!(
                "<p><strong>Tracking decision basis:</strong> <code>{}</code></p>",
                escape_html(basis)
            ));
        }
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

    body.push_str("<h2>Visual evidence</h2><div class=gallery>");
    diagnostic_image(
        &mut body,
        report_path,
        diagnostics.join("candidate.png"),
        "Selected writing surface",
        "If this is the wrong object, stop here and correct selection/bounds. Do not tune tracking or typography yet.",
    );
    diagnostic_image(
        &mut body,
        report_path,
        diagnostics.join("tracking-contact-sheet.png"),
        "Tracking across time",
        "Look for drift, scale jumps, or perspective errors. Correct only the frames that are actually wrong.",
    );
    diagnostic_image(
        &mut body,
        report_path,
        diagnostics.join("canonical-reference.png"),
        "Canonical writing surface",
        "Look for damaged texture, residual old title content, or an incorrect writable silhouette.",
    );
    diagnostic_image(
        &mut body,
        report_path,
        diagnostics.join("temporal-mad.png"),
        "Temporal change map",
        "Bright regions identify unstable areas. Use this to decide whether foreground/occlusion or reconstruction needs attention.",
    );
    body.push_str("</div>");

    append_string_array(&mut body, "Analyzer notes", summary.get("remedies"));
    body.push_str("<h2>Rule of thumb</h2><p>Correct <strong>intent</strong>, not generated state: surface selection first, then sparse motion anchors, then sparse foreground prompts. Dense tracks and masks belong to generated artifacts/caches.</p>");

    format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Plaque Forge review</title><style>{}</style></head><body>{}</body></html>",
        CSS, body
    )
}

fn analysis_asset_stem(analysis_root: &Path) -> Option<String> {
    if analysis_root
        .parent()?
        .parent()?
        .file_name()
        .and_then(|name| name.to_str())
        == Some("failures")
    {
        return analysis_root
            .parent()?
            .file_name()?
            .to_str()
            .map(str::to_owned);
    }
    let name = analysis_root.file_name()?.to_str()?;
    Some(name.split(".partial-").next().unwrap_or(name).to_string())
}

fn append_actionable_commands(
    body: &mut String,
    report_path: &Path,
    analysis_root: &Path,
    scene: Option<&(PathBuf, Scene)>,
) {
    let Some(asset) = analysis_asset_stem(analysis_root) else {
        return;
    };
    body.push_str("<h2>What to do next</h2><p>This report is the human front door. Make the smallest correction suggested above, then rerun the high-level workflow:</p><pre>");
    body.push_str(&escape_html(&format!(
        "./scripts/analyze_assets.sh {asset}\n"
    )));
    body.push_str(&escape_html(&format!(
        "./scripts/review_assets.sh {asset}\n"
    )));
    body.push_str("</pre>");
    if let Some((path, _)) = scene {
        body.push_str(&format!(
            "<p>Human intent file: <code>{}</code>. Dense generated tracks/masks should not be hand-edited.</p>",
            escape_html(&portable_display(report_path, path))
        ));
    } else {
        body.push_str(&format!(
            "<p>No scene exists. Only create one when the automatic result is visibly wrong: <code>target/release/plaque-forge create-scene --input assets/{}.mp4</code>.</p>",
            escape_html(&asset)
        ));
    }
}

fn append_ml_status(body: &mut String, summary: &Value) {
    let automatic = summary
        .get("automatic_ml_foreground")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_occluder = summary
        .get("has_occluder")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    body.push_str("<h2>ML / Python participation</h2>");
    if automatic {
        body.push_str("<p><strong>Python ML was used automatically</strong> to refine foreground masks after Rust detected a crossing. Inspect <code>ml-foreground/</code> for generated provenance and <code>./scripts/ml_status.sh</code> for runtime history.</p>");
    } else if has_occluder {
        body.push_str("<p>Foreground was detected but this cache does not record automatic ML scene. If the crossing looks poor, rerun analysis with the ML runtime installed and enabled, then inspect <code>./scripts/ml_status.sh</code>.</p>");
    } else {
        body.push_str("<p>No automatic ML foreground pass was needed for this analysis. Authored prompted layers, when present, are reported through scene provenance.</p>");
    }
}

fn append_scene_owned(body: &mut String, report_path: &Path, scene: Option<&(PathBuf, Scene)>) {
    let Some((path, scene)) = scene else {
        return;
    };
    let selected = scene.select_surface(None).ok();
    body.push_str("<h2>Current human intent</h2>");
    body.push_str(&format!(
        "<p><code>{}</code> &middot; schema {}.</p>",
        escape_html(&portable_display(report_path, path)),
        scene.format,
    ));
    if let Some(plaque) = selected {
        let shape = plaque
            .writable_region
            .as_ref()
            .map(|region| region.kind().to_string())
            .unwrap_or_else(|| "rectangular bounds".to_string());
        body.push_str(&format!(
            "<p>Plaque <code>{}</code>; {}; {} sparse motion anchor(s); {} plaque prompt(s).</p>",
            escape_html(&plaque.id),
            escape_html(&shape),
            plaque.anchors.len(),
            plaque.prompts.len(),
        ));
        let prompted_layers = scene
            .layers
            .iter()
            .filter(|layer| layer.surface == plaque.id && !layer.prompts.is_empty())
            .count();
        if prompted_layers > 0 {
            body.push_str(&format!(
                "<p>{prompted_layers} foreground/layer scene(s) contain sparse prompts.</p>"
            ));
        }
    }
}

fn append_candidate_ranking(body: &mut String, candidates: Option<&Value>) {
    let Some(rows) = candidates.and_then(Value::as_array) else {
        return;
    };
    if rows.is_empty() {
        return;
    }
    body.push_str("<h2>Automatic surface alternatives</h2><p>These are for comparison only. Prefer the largest plausible writing surface, not merely the highest local score.</p><table><thead><tr><th></th><th>Confidence</th><th>Frame</th><th>Bounds</th><th>Area</th></tr></thead><tbody>");
    for row in rows.iter().take(6) {
        let selected = row
            .get("selected")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let confidence = row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
        let frame = row.get("frame").and_then(Value::as_u64).unwrap_or(0);
        let rect = row
            .get("rect")
            .and_then(Value::as_array)
            .map(|values| values.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
            .unwrap_or_default();
        let bounds = if rect.len() == 4 {
            format!(
                "{:.0}, {:.0}, {:.0}, {:.0}",
                rect[0], rect[1], rect[2], rect[3]
            )
        } else {
            "?".to_string()
        };
        let area = if rect.len() == 4 {
            rect[2] * rect[3]
        } else {
            0.0
        };
        body.push_str(&format!(
            "<tr{}><td>{}</td><td>{:.3}</td><td>{}</td><td><code>{}</code></td><td>{:.0}</td></tr>",
            if selected { " class=selected" } else { "" },
            if selected { "selected" } else { "" },
            confidence,
            frame,
            escape_html(&bounds),
            area,
        ));
    }
    body.push_str("</tbody></table>");
}

fn selected_candidate_frame(candidates: Option<&Value>) -> Option<u64> {
    candidates
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter()
                .find(|row| {
                    row.get("selected")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                })
                .or_else(|| rows.first())
        })
        .and_then(|row| row.get("frame"))
        .and_then(Value::as_u64)
}

fn append_coordinate_helper(body: &mut String, report_path: &Path, path: PathBuf, frame: u64) {
    if !path.is_file() {
        return;
    }
    let url = relative_url(report_path, &path);
    body.push_str(&format!(
        r#"<h2>Visual coordinate helper</h2>
<p>Click the selected-surface image instead of calculating coordinates. The helper reports both source pixels and normalized coordinates. Four clicks also produce a sparse motion-anchor snippet in TL/TR/BR/BL click order. Reset and click again if needed.</p>
<div class=coord-helper>
<img id=coord-image src="{}" data-frame="{}" alt="candidate coordinate helper">
<div><button type=button id=coord-reset>Reset points</button><pre id=coord-output>Click a point on the image.</pre></div>
</div>
<script>
(() => {{
  const img = document.getElementById('coord-image');
  const out = document.getElementById('coord-output');
  const reset = document.getElementById('coord-reset');
  if (!img || !out || !reset) return;
  let points = [];
  const show = (last) => {{
    let text = '';
    if (last) {{
      text += `pixel = [${{last.px.toFixed(1)}}, ${{last.py.toFixed(1)}}]\n`;
      text += `normalized = [${{last.nx.toFixed(5)}}, ${{last.ny.toFixed(5)}}]\n\n`;
      text += `# Segmentation point\ncoordinates = "normalized"\npositive_points = [[${{last.nx.toFixed(5)}}, ${{last.ny.toFixed(5)}}]]\n`;
    }}
    if (points.length === 4) {{
      const q = points.map(p => `[${{p.nx.toFixed(5)}}, ${{p.ny.toFixed(5)}}]`).join(', ');
      text += `\n# Sparse physical-plane anchor\n[[surfaces.anchors]]\nframe = ${{img.dataset.frame}}\ncoordinates = "normalized"\nquad = [${{q}}]\nlocked = true\n`;
    }} else if (points.length > 0) {{
      text += `\n${{points.length}}/4 motion-quad points selected.`;
    }}
    out.textContent = text || 'Click a point on the image.';
  }};
  img.addEventListener('click', event => {{
    const rect = img.getBoundingClientRect();
    const nx = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    const ny = Math.min(1, Math.max(0, (event.clientY - rect.top) / rect.height));
    const point = {{ nx, ny, px: nx * Math.max(0, img.naturalWidth - 1), py: ny * Math.max(0, img.naturalHeight - 1) }};
    if (points.length >= 4) points = [];
    points.push(point);
    show(point);
  }});
  reset.addEventListener('click', () => {{ points = []; show(null); }});
}})();
</script>"#,
        escape_html(&url),
        frame,
    ));
}

fn build_text_summary(
    report_path: &Path,
    analysis_root: &Path,
    summary: &Value,
    scene: Option<&(PathBuf, Scene)>,
    focus: &[FocusItem],
) -> String {
    let mut output = format!(
        "Plaque Forge review\nAnalysis: {}\nOverall: {}\n\nFocus first:\n",
        portable_display(report_path, analysis_root),
        number(summary, "overall")
            .map(|value| format!("{value:.3}"))
            .unwrap_or_else(|| "n/a".to_string()),
    );
    for (index, item) in focus.iter().enumerate() {
        output.push_str(&format!(
            "{}. {}\n   Evidence: {}\n   Action: {}\n",
            index + 1,
            item.title,
            item.evidence,
            item.action,
        ));
    }
    if let Some((path, scene)) = scene {
        output.push_str(&format!(
            "\nScene: {} (schema {})\n",
            portable_display(report_path, path),
            scene.format
        ));
        if let Ok(plaque) = scene.select_surface(None) {
            let layer_prompts = scene
                .layers
                .iter()
                .filter(|layer| layer.surface == plaque.id)
                .map(|layer| layer.prompts.len())
                .sum::<usize>();
            output.push_str(&format!(
                "Selected surface: {}; sparse trajectory anchors: {}; segmentation prompts: {}\n",
                plaque.id,
                plaque.anchors.len(),
                plaque.prompts.len() + layer_prompts,
            ));
        }
    }
    let automatic_ml = summary
        .get("automatic_ml_foreground")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let has_occluder = summary
        .get("has_occluder")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let prompted_ml =
        scene.is_some_and(|(_, scene)| scene.layers.iter().any(|layer| !layer.prompts.is_empty()));
    output.push_str(&format!(
        "\nPython ML: {}\n",
        if automatic_ml {
            "used automatically for foreground scene"
        } else if prompted_ml {
            "used for authored lossless segmentation layers"
        } else if has_occluder {
            "not recorded for this cache although a foreground crossing was detected"
        } else {
            "not needed for automatic foreground scene"
        }
    ));
    if let Some(asset) = analysis_asset_stem(analysis_root) {
        output.push_str(&format!(
            "\nNext commands:\n  ./scripts/analyze_assets.sh {asset}\n  ./scripts/review_assets.sh {asset}\n"
        ));
    }
    output
}

fn append_decision_trace(body: &mut String, trace: &RenderDecisionTrace) {
    body.push_str("<h2>Render decision trace</h2>");
    body.push_str(
        "<p>This section explains the causal rendering choices rather than only their resulting scores.</p><dl>",
    );
    let surface = trace.surface.id.as_deref().unwrap_or("<automatic>");
    body.push_str(&format!(
        "<dt>Selected surface</dt><dd><code>{}</code> ({})</dd>",
        escape_html(surface),
        escape_html(&trace.surface.selection_reason)
    ));
    body.push_str(&format!(
        "<dt>Reference frame</dt><dd>{}</dd>",
        trace.surface.reference_frame
    ));
    body.push_str(&format!(
        "<dt>Tracking model</dt><dd><code>{}</code></dd>",
        escape_html(&trace.tracking.trajectory_model)
    ));
    body.push_str(&format!(
        "<dt>Canonical title plane</dt><dd>{} × {}</dd>",
        trace.surface.canonical_width, trace.surface.canonical_height
    ));
    body.push_str(&format!(
        "<dt>Typography</dt><dd>{:.2}px, {} lines, {:.1}% fill</dd>",
        trace.typography.font_size,
        trace.typography.lines,
        trace.typography.fill_ratio * 100.0
    ));
    if !trace
        .tracking
        .foreground_layers_excluded_from_tracking
        .is_empty()
    {
        body.push_str(&format!(
            "<dt>Foreground excluded from tracking</dt><dd><code>{}</code></dd>",
            escape_html(
                &trace
                    .tracking
                    .foreground_layers_excluded_from_tracking
                    .join(", ")
            )
        ));
    }
    body.push_str("</dl>");
    if !trace.compositing_layers.is_empty() {
        body.push_str("<h3>Compositing layers</h3><ul>");
        for layer in &trace.compositing_layers {
            body.push_str(&format!(
                "<li><code>{}</code>: {:?}; layout={}, tracking={}, matte={:?}</li>",
                escape_html(&layer.id),
                layer.role,
                layer.affects_layout,
                layer.affects_tracking,
                layer.matte.mode
            ));
        }
        body.push_str("</ul>");
    }
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

fn diagnostic_image(
    body: &mut String,
    report_path: &Path,
    path: PathBuf,
    title: &str,
    guidance: &str,
) {
    if !path.is_file() {
        return;
    }
    let url = relative_url(report_path, &path);
    body.push_str(&format!(
        "<figure><figcaption><strong>{}</strong><br>{}</figcaption><a href=\"{}\"><img src=\"{}\" loading=lazy></a></figure>",
        escape_html(title),
        escape_html(guidance),
        escape_html(&url),
        escape_html(&url),
    ));
}

fn relative_url(owner: &Path, path: &Path) -> String {
    let relative = crate::portable_path::relative_reference(owner, path)
        .map(|path| path.to_string())
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "diagnostic".to_string())
        });
    let mut encoded = String::with_capacity(relative.len());
    for byte in relative.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(*byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

fn portable_display(owner: &Path, path: &Path) -> String {
    if path.is_relative() {
        return path.to_string_lossy().into_owned();
    }
    if let Ok(project) = std::env::current_dir()
        && let Ok(relative) = path.strip_prefix(&project)
    {
        return relative.to_string_lossy().into_owned();
    }
    if path.is_absolute() {
        return path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
    }
    crate::portable_path::relative_reference(owner, path)
        .map(|path| path.to_string())
        .unwrap_or_else(|_| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| ".".to_string())
        })
}

fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_u64().map(|value| value as f64))
            .or_else(|| value.as_i64().map(|value| value as f64))
    })
}

fn display_pixels(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.2}px"))
        .unwrap_or_else(|| "unmeasurable".to_string())
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
body { max-width:1180px; margin:0 auto; padding:32px; line-height:1.45; }
h1,h2 { letter-spacing:.01em; } h2 { margin-top:34px; }
.path { color:#aeb9c4; } code { color:#d8e3ec; }
.partial { border-left:5px solid #df6363; padding:10px 14px; background:#24191b; }
.focus { display:grid; gap:10px; }
.focus article { border:1px solid #3a4651; border-radius:9px; padding:14px 16px; background:#171d23; }
.focus article p { margin:.35em 0 0; }
.focus .action { color:#cfe3d7; }
.metrics { display:grid; grid-template-columns:repeat(auto-fit,minmax(180px,1fr)); gap:10px; }
.metric { border:1px solid #3a4651; border-left-width:6px; border-radius:8px; padding:12px; background:#171d23; display:flex; justify-content:space-between; gap:16px; }
.metric.good { border-left-color:#54c987; } .metric.warn { border-left-color:#d7ae4a; } .metric.bad { border-left-color:#df6363; } .metric.neutral { border-left-color:#71808e; }
.gallery { display:grid; grid-template-columns:repeat(auto-fit,minmax(420px,1fr)); gap:18px; }
figure { margin:0; border:1px solid #33404b; border-radius:10px; overflow:hidden; background:#171d23; }
figcaption { padding:12px 14px; color:#c6d0d9; } img { display:block; width:100%; height:auto; background:#090b0d; }
pre { padding:14px; border:1px solid #33404b; border-radius:8px; background:#0d1115; white-space:pre-wrap; font-size:1rem; }
table { width:100%; border-collapse:collapse; background:#171d23; } th,td { text-align:left; padding:8px 10px; border-bottom:1px solid #33404b; } tr.selected { background:#1b2b23; }
.coord-helper { display:grid; grid-template-columns:minmax(320px,2fr) minmax(280px,1fr); gap:16px; align-items:start; }
.coord-helper img { width:100%; cursor:crosshair; border:1px solid #40505d; border-radius:8px; }
button { padding:8px 12px; background:#25313a; color:#e9eef3; border:1px solid #566774; border-radius:6px; cursor:pointer; }
li { margin:.45em 0; }
"#;

#[cfg(test)]
mod tests {
    use super::{analysis_asset_stem, focus_items, validate_verification_provenance};
    use serde_json::json;
    use std::{fs, path::Path};

    #[test]
    fn retained_failure_uses_the_asset_directory_not_the_run_id() {
        assert_eq!(
            analysis_asset_stem(Path::new(
                "/tmp/plaque-forge/failures/16_9_scene/1720000000-42"
            ))
            .as_deref(),
            Some("16_9_scene")
        );
        assert_eq!(
            analysis_asset_stem(Path::new("assets/analysis/16_9_scene.partial-42")).as_deref(),
            Some("16_9_scene")
        );
    }

    #[test]
    fn passing_render_verification_does_not_hide_analysis_findings() {
        let summary = json!({
            "overall": 0.72,
            "candidate_confidence": 0.60,
            "motion_confidence": 0.50,
            "structural_confidence": 0.40,
            "occlusion_confidence": 0.30
        });
        let verification = json!({"passed": true, "overall": 0.99});

        let focus = focus_items(&summary, Some(&verification));

        assert_eq!(focus.len(), 4);
        assert!(
            focus
                .iter()
                .any(|item| item.title == "Writing-surface selection")
        );
        assert!(focus.iter().any(|item| item.title == "Motion tracking"));
        assert!(
            focus
                .iter()
                .any(|item| item.title == "Writable surface / canonical reconstruction")
        );
        assert!(focus.iter().any(|item| item.title == "Foreground crossing"));
    }

    #[test]
    fn review_rejects_verification_for_another_render_manifest() {
        let root = std::env::temp_dir().join(format!(
            "plaque-forge-review-provenance-test-{}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(&root).unwrap();
        let manifest_path = root.join("render-manifest.json");
        let manifest = json!({
            "source_sha256": "source",
            "analysis_manifest_sha256": "analysis",
            "rendered_sha256": "render"
        });
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        fs::write(&manifest_path, &manifest_bytes).unwrap();
        let verification_path = root.join("verification.json");
        let verification = json!({
            "source_sha256": "source",
            "analysis_manifest_sha256": "analysis",
            "rendered_sha256": "different-render",
            "render_manifest_sha256": crate::digest::bytes_sha256(&manifest_bytes)
        });

        let error = validate_verification_provenance(
            Some(&verification_path),
            Some(&verification),
            Some(&manifest_path),
            Some(&manifest),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("is stale"), "unexpected error: {error}");
        fs::remove_dir_all(root).unwrap();
    }
}
