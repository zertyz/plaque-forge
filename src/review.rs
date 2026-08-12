//! Human-oriented front door for analysis and verification diagnostics.
//!
//! Machine JSON remains canonical for automation. This module turns the same evidence
//! into a short triage report that says what deserves human attention first. It also
//! accepts partial analysis directories, because failed quality gates are exactly when
//! a useful review page matters most.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::{cli::ReviewArgs, refinement::Refinement};

#[derive(Debug, Clone)]
struct FocusItem {
    title: &'static str,
    evidence: String,
    action: &'static str,
}

struct ReportInputs<'a> {
    analysis_root: &'a Path,
    diagnostics: &'a Path,
    summary: &'a Value,
    occlusion: Option<&'a Value>,
    candidates: Option<&'a Value>,
    verification: Option<&'a Value>,
    render_manifest: Option<&'a Value>,
    refinement: Option<&'a (PathBuf, Refinement)>,
    focus: &'a [FocusItem],
}

pub fn run(args: ReviewArgs) -> Result<()> {
    let analysis_root = args.analysis;
    if !analysis_root.is_dir() {
        bail!("analysis directory does not exist: {}", analysis_root.display());
    }
    let diagnostics = analysis_root.join("diagnostics");
    fs::create_dir_all(&diagnostics)?;
    let output = args
        .output
        .unwrap_or_else(|| diagnostics.join("review.html"));
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }

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
    let refinement = match args.refinement.as_deref() {
        Some(path) if path.is_file() => Some((path.to_path_buf(), Refinement::load(path)?)),
        _ => None,
    };

    let focus = focus_items(&summary, verification.as_ref());
    let html = build_report(ReportInputs {
        analysis_root: &analysis_root,
        diagnostics: &diagnostics,
        summary: &summary,
        occlusion: occlusion.as_ref(),
        candidates: candidates.as_ref(),
        verification: verification.as_ref(),
        render_manifest: render_manifest.as_ref(),
        refinement: refinement.as_ref(),
        focus: &focus,
    });
    fs::write(&output, html)
        .with_context(|| format!("failed to write review report {}", output.display()))?;

    let text_output = output.with_extension("txt");
    fs::write(
        &text_output,
        build_text_summary(&analysis_root, &summary, refinement.as_ref(), &focus),
    )
    .with_context(|| format!("failed to write review summary {}", text_output.display()))?;

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
            action: "Start with the selected-surface/candidate evidence below. If selection is correct, rerun after addressing the explicit analyzer error rather than inventing downstream refinements.",
        });
    }

    if candidate.is_some_and(|value| value < 0.75) {
        items.push(FocusItem {
            title: "Writing-surface selection",
            evidence: format!("candidate confidence {:.3}", candidate.unwrap_or_default()),
            action: "Confirm the intended surface in candidate.png. If it is wrong, edit only reference_frame/bounds or writable_region in refinement.toml.",
        });
    }
    if motion.is_some_and(|value| value < 0.75) {
        items.push(FocusItem {
            title: "Motion tracking",
            evidence: format!("tracking confidence {:.3}", motion.unwrap_or_default()),
            action: "Inspect tracking-contact-sheet.jpg. Add sparse [[plaques.motion]] anchors only at frames where the tracked surface is visibly wrong.",
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
        let typography = number(verification, "typography_fit");
        let temporal = number(verification, "temporal_stability");
        if tracking.is_some_and(|value| value < 0.95)
            && !items.iter().any(|item| item.title == "Motion tracking")
        {
            items.push(FocusItem {
                title: "Rendered tracking lock",
                evidence: format!("verification tracking score {:.3}", tracking.unwrap_or_default()),
                action: "Compare the rendered video against tracking-contact-sheet.jpg before changing typography.",
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
        items.push(FocusItem {
            title: "No obvious scene-analysis blocker",
            evidence: "the available analysis/verification metrics are above the triage thresholds".to_string(),
            action: "Proceed to visual typography/effect tuning. Human refinement is optional unless you can see a defect that metrics missed.",
        });
    }
    items
}

fn build_report(inputs: ReportInputs<'_>) -> String {
    let ReportInputs {
        analysis_root,
        diagnostics,
        summary,
        occlusion,
        candidates,
        verification,
        render_manifest,
        refinement,
        focus,
    } = inputs;
    let mut body = String::new();
    body.push_str("<h1>Plaque Forge review</h1>");
    body.push_str(&format!(
        "<p class=path>Analysis: <code>{}</code></p>",
        escape_html(&analysis_root.display().to_string())
    ));
    if analysis_root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(".partial-"))
    {
        body.push_str("<p class=partial><strong>Partial analysis.</strong> The quality gate stopped this run, but the evidence below is intentionally retained for human triage.</p>");
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

    append_refinement_owned(&mut body, refinement);
    append_candidate_ranking(&mut body, candidates);
    append_coordinate_helper(
        &mut body,
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

    body.push_str("<h2>Visual evidence</h2><div class=gallery>");
    diagnostic_image(
        &mut body,
        diagnostics.join("candidate.png"),
        "Selected writing surface",
        "If this is the wrong object, stop here and correct selection/bounds. Do not tune tracking or typography yet.",
    );
    diagnostic_image(
        &mut body,
        diagnostics.join("tracking-contact-sheet.jpg"),
        "Tracking across time",
        "Look for drift, scale jumps, or perspective errors. Correct only the frames that are actually wrong.",
    );
    diagnostic_image(
        &mut body,
        diagnostics.join("canonical-reference.png"),
        "Canonical writing surface",
        "Look for damaged texture, residual old title content, or an incorrect writable silhouette.",
    );
    diagnostic_image(
        &mut body,
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

fn append_refinement_owned(body: &mut String, refinement: Option<&(PathBuf, Refinement)>) {
    let Some((path, refinement)) = refinement else {
        return;
    };
    let selected = refinement.select_plaque(None).ok();
    body.push_str("<h2>Current human intent</h2>");
    body.push_str(&format!(
        "<p><code>{}</code> &middot; schema {}.</p>",
        escape_html(&path.display().to_string()),
        refinement.schema_version,
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
            plaque.motion.len(),
            plaque.prompts.len(),
        ));
        let prompted_layers = refinement
            .layers
            .iter()
            .filter(|layer| layer.plaque == plaque.id && !layer.prompts.is_empty())
            .count();
        if prompted_layers > 0 {
            body.push_str(&format!(
                "<p>{prompted_layers} foreground/layer refinement(s) contain sparse prompts.</p>"
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
        let selected = row.get("selected").and_then(Value::as_bool).unwrap_or(false);
        let confidence = row.get("confidence").and_then(Value::as_f64).unwrap_or(0.0);
        let frame = row.get("frame").and_then(Value::as_u64).unwrap_or(0);
        let rect = row
            .get("rect")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_f64)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let bounds = if rect.len() == 4 {
            format!("{:.0}, {:.0}, {:.0}, {:.0}", rect[0], rect[1], rect[2], rect[3])
        } else {
            "?".to_string()
        };
        let area = if rect.len() == 4 { rect[2] * rect[3] } else { 0.0 };
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
                .find(|row| row.get("selected").and_then(Value::as_bool).unwrap_or(false))
                .or_else(|| rows.first())
        })
        .and_then(|row| row.get("frame"))
        .and_then(Value::as_u64)
}

fn append_coordinate_helper(body: &mut String, path: PathBuf, frame: u64) {
    if !path.is_file() {
        return;
    }
    let url = file_url(&path);
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
      text += `\n# Sparse motion correction\n[[plaques.motion]]\nframe = ${{img.dataset.frame}}\ncoordinates = "normalized"\nquad = [${{q}}]\nlocked = true\n`;
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
    analysis_root: &Path,
    summary: &Value,
    refinement: Option<&(PathBuf, Refinement)>,
    focus: &[FocusItem],
) -> String {
    let mut output = format!(
        "Plaque Forge review\nAnalysis: {}\nOverall: {}\n\nFocus first:\n",
        analysis_root.display(),
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
    if let Some((path, refinement)) = refinement {
        output.push_str(&format!(
            "\nRefinement: {} (schema {})\n",
            path.display(),
            refinement.schema_version
        ));
        if let Ok(plaque) = refinement.select_plaque(None) {
            output.push_str(&format!(
                "Selected plaque: {}; sparse motion anchors: {}; plaque prompts: {}\n",
                plaque.id,
                plaque.motion.len(),
                plaque.prompts.len(),
            ));
        }
    }
    output
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
