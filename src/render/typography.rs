//! Typography shaping, fitting, styling, and rasterization.
//!
//! Text is fitted against the analyzed writable mask rather than only its bounding box.
//! Shaping/layout stays separate from paint effects so future animation and material
//! stages can evolve without entangling line breaking with compositing.

use std::path::Path;

use anyhow::{Context, Result, bail};
use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
    fontdb,
};
use image::{RgbaImage, imageops::FilterType};

use crate::{
    application::{FitMode, TextAlign, VerticalAlign},
    color::Rgba,
    model::TypographyMetrics,
    surface::Surface,
};

use super::effects::Style;

pub struct TextRender {
    pub layer: Surface,
    /// Final-resolution fill coverage before glow/shadow/material animation.
    pub glyph_mask: Vec<u8>,
    pub metrics: TypographyMetrics,
}

/// Inputs required to shape and fit one title in canonical plaque coordinates.
///
/// Keeping this separate from CLI types makes typography reusable by future preview,
/// animation, and batch-render front ends without coupling them to clap.
pub struct RenderRequest<'a> {
    pub width: u32,
    pub height: u32,
    pub mask: &'a [u8],
    pub text: &'a str,
    pub font_path: &'a Path,
    pub fit_mode: FitMode,
    pub requested_font_size: Option<f32>,
    pub supersampling: u32,
    pub target_fill: f32,
    pub max_lines: usize,
    pub padding_ratio: f32,
    pub line_height_ratio: f32,
    pub text_align: TextAlign,
    pub vertical_align: VerticalAlign,
    pub style: &'a Style,
}

struct TypographyContext<'a> {
    family: &'a str,
    requested_face: fontdb::ID,
    line_height_ratio: f32,
    raster_width: u32,
    raster_height: u32,
    max_lines: usize,
    align: &'a Align,
    width: u32,
    height: u32,
    mask: &'a [u8],
    bounds: (u32, u32, u32, u32),
    pad_x: u32,
    pad_y: u32,
    supersampling: u32,
    vertical_align: VerticalAlign,
    style: &'a Style,
}

pub fn render(request: RenderRequest<'_>) -> Result<TextRender> {
    let RenderRequest {
        width,
        height,
        mask,
        text,
        font_path,
        fit_mode,
        requested_font_size,
        supersampling,
        target_fill,
        max_lines,
        padding_ratio,
        line_height_ratio,
        text_align,
        vertical_align,
        style,
    } = request;
    if text.trim().is_empty() {
        bail!("title text is empty; provide --text or a non-empty --text-file");
    }
    if !font_path.is_file() {
        bail!("font file does not exist: {}", font_path.display());
    }
    if max_lines == 0 {
        bail!("--max-lines must be at least 1");
    }
    if !(0.05..=0.98).contains(&target_fill) {
        bail!("--target-fill must be between 0.05 and 0.98");
    }
    if !(0.0..=0.40).contains(&padding_ratio) {
        bail!("--padding must be between 0 and 0.40");
    }
    if !(0.75..=2.0).contains(&line_height_ratio) {
        bail!("--line-height must be between 0.75 and 2.0");
    }
    if matches!(fit_mode, FitMode::Fixed) && requested_font_size.is_none() {
        bail!("--fit fixed requires --font-size");
    }

    let supersampling = supersampling.clamp(1, 4);
    let bounds = mask_bounds(width, height, mask).context("content mask is empty")?;
    let pad_x = ((bounds.2 - bounds.0 + 1) as f32 * padding_ratio).round() as u32;
    let pad_y = ((bounds.3 - bounds.1 + 1) as f32 * padding_ratio).round() as u32;
    let safe_width = (bounds.2 - bounds.0 + 1).saturating_sub(2 * pad_x).max(1);
    let safe_height = (bounds.3 - bounds.1 + 1).saturating_sub(2 * pad_y).max(1);

    let mut font_system =
        FontSystem::new_with_fonts([fontdb::Source::File(font_path.to_path_buf())]);
    let (family, requested_face) = discover_family(&font_system, font_path)?;
    let raster_width = safe_width * supersampling;
    let raster_height = safe_height * supersampling;
    let align = match text_align {
        TextAlign::Left => Align::Left,
        TextAlign::Center => Align::Center,
        TextAlign::Right => Align::Right,
    };
    let context = TypographyContext {
        family: &family,
        requested_face,
        line_height_ratio,
        raster_width,
        raster_height,
        max_lines,
        align: &align,
        width,
        height,
        mask,
        bounds,
        pad_x,
        pad_y,
        supersampling,
        vertical_align,
        style,
    };
    let preflight = measure_candidate(
        &mut font_system,
        text,
        24.0 * supersampling as f32,
        max_lines.max(text.lines().count()),
        &context,
        Wrap::WordOrGlyph,
    )?;
    if preflight.missing_glyphs > 0 || preflight.fallback_glyphs > 0 {
        bail!(
            "font {} cannot render the requested title deterministically: {} missing glyphs, {} fallback glyphs; choose a font containing every character",
            font_path.display(),
            preflight.missing_glyphs,
            preflight.fallback_glyphs
        );
    }

    let upper = requested_font_size
        .map(|size| size * supersampling as f32)
        .unwrap_or(raster_height as f32)
        .min(raster_height as f32 * 1.5)
        .max(6.0 * supersampling as f32);

    let (maximum_size, selected_size, composed, resolved_text) = match fit_mode {
        FitMode::Fixed => {
            let size = requested_font_size.expect("validated") * supersampling as f32;
            let candidate = rasterize_candidate(
                &mut font_system,
                text,
                size,
                max_lines,
                &context,
                Wrap::WordOrGlyph,
            )?;
            if !candidate.fits(max_lines) {
                bail!(
                    "requested font size {:.2}px does not fit the plaque with --max-lines {}; \
                     reduce --font-size, reduce --padding, or increase --max-lines",
                    requested_font_size.expect("validated"),
                    max_lines
                );
            }
            let fitted = probe_masked_candidate(&context, size, candidate)?;
            if fitted.clipped_pixels > 0 {
                bail!(
                    "font size {:.2}px fits the bounding rectangle but its final glyph/effect layer crosses the plaque mask by {} pixels in {}; increase --padding or reduce --font-size",
                    requested_font_size.expect("validated"),
                    fitted.clipped_pixels,
                    format_bounds(fitted.clipped_bounds)
                );
            }
            let result = compose_layer(&context, size, fitted)?;
            (size, size, result, text.to_string())
        }
        FitMode::Artistic => find_artistic_layout(&mut font_system, text, upper, &context)?,
        FitMode::Maximize | FitMode::Balanced => {
            let (rectangle_maximum, _) =
                find_maximum_candidate(&mut font_system, text, upper, &context, Wrap::WordOrGlyph)?;
            let (maximum_size, maximum_fitted) = find_maximum_masked_candidate(
                &mut font_system,
                text,
                rectangle_maximum,
                &context,
                Wrap::WordOrGlyph,
            )?;
            if matches!(fit_mode, FitMode::Balanced)
                && maximum_fitted.fill_ratio > target_fill as f64
            {
                let scale = (target_fill as f64 / maximum_fitted.fill_ratio)
                    .sqrt()
                    .clamp(0.65, 1.0) as f32;
                let size = maximum_size * scale;
                let balanced = evaluate_masked_candidate(
                    &mut font_system,
                    text,
                    size,
                    &context,
                    Wrap::WordOrGlyph,
                )?;
                match balanced.filter(|result| result.clipped_pixels == 0) {
                    Some(result) => (
                        maximum_size,
                        size,
                        compose_layer(&context, size, result)?,
                        text.to_string(),
                    ),
                    None => (
                        maximum_size,
                        maximum_size,
                        compose_layer(&context, maximum_size, maximum_fitted)?,
                        text.to_string(),
                    ),
                }
            } else {
                (
                    maximum_size,
                    maximum_size,
                    compose_layer(&context, maximum_size, maximum_fitted)?,
                    text.to_string(),
                )
            }
        }
    };

    let explicit_newlines = text.chars().filter(|&character| character == '\n').count();

    Ok(TextRender {
        layer: composed.layer,
        glyph_mask: composed.glyph_mask,
        metrics: TypographyMetrics {
            fit_mode: format!("{fit_mode:?}").to_lowercase(),
            font_size: selected_size / supersampling as f32,
            maximum_safe_font_size: maximum_size / supersampling as f32,
            lines: composed.lines,
            fill_ratio: composed.fill_ratio,
            minimum_padding_ratio: composed.minimum_padding_ratio,
            clipped_pixels: composed.clipped_pixels,
            missing_glyphs: composed.missing_glyphs,
            fallback_glyphs: composed.fallback_glyphs,
            explicit_newlines,
            resolved_text,
        },
    })
}

fn find_artistic_layout(
    font_system: &mut FontSystem,
    text: &str,
    upper: f32,
    context: &TypographyContext<'_>,
) -> Result<(f32, f32, Composed, String)> {
    let candidates = artistic_line_break_candidates(text, context.max_lines);
    let mut proposals = Vec::<(String, f32, Vec<f32>)>::new();

    for candidate_text in candidates {
        let Ok((rectangle_maximum, measured)) =
            find_maximum_candidate(font_system, &candidate_text, upper, context, Wrap::None)
        else {
            continue;
        };
        proposals.push((candidate_text, rectangle_maximum, measured.line_widths));
    }

    // Exact irregular-mask probing rasterizes glyphs. Rank the cheap font-aware
    // measurements first and keep a deliberately small Pareto-like finalist set.
    // This includes real advances for spaces, kerning, punctuation, and script
    // shaping because cosmic-text supplied every `line_widths` value above.
    proposals.sort_by(
        |(left_text, left_size, left_widths), (right_text, right_size, right_widths)| {
            artistic_layout_score(*right_size, right_widths, right_text)
                .total_cmp(&artistic_layout_score(*left_size, left_widths, left_text))
                .then_with(|| right_size.total_cmp(left_size))
        },
    );
    let largest_proposal = proposals
        .iter()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .cloned();
    // Keep several font-shaped finalists. The exact masked probe is now cheap,
    // and a single rectangle-ranked proposal can be badly suboptimal for an
    // ellipse, rounded plaque, or irregular writable silhouette.
    proposals.truncate(6);
    if let Some(largest) = largest_proposal
        && proposals.iter().all(|proposal| proposal.0 != largest.0)
    {
        proposals.push(largest);
    }

    let mut finalists = Vec::<(String, f32, FittedCandidate)>::new();
    for (candidate_text, rectangle_maximum, _) in proposals {
        let Ok((maximum_size, fitted)) = find_maximum_masked_candidate(
            font_system,
            &candidate_text,
            rectangle_maximum,
            context,
            Wrap::None,
        ) else {
            continue;
        };

        finalists.push((candidate_text, maximum_size, fitted));
    }

    let largest = finalists
        .iter()
        .map(|(_, size, _)| *size)
        .max_by(f32::total_cmp)
        .context(
        "no artistic word-boundary layout fits the plaque; use --fit maximize, increase --max-lines, use a narrower font, or shorten the title",
    )?;

    // Size is the primary quality constraint. Aesthetics choose only among layouts
    // within three percent of the largest safe type, so a tidy but timid layout
    // cannot leave a large part of the plaque unused.
    let minimum_finalist = largest * 0.97;
    let (selected_text, maximum_size, selected) = finalists
        .into_iter()
        .filter(|(_, size, _)| *size >= minimum_finalist)
        .max_by(
            |(left_text, left_size, left), (right_text, right_size, right)| {
                artistic_layout_score(*left_size, &left.candidate.line_widths, left_text).total_cmp(
                    &artistic_layout_score(*right_size, &right.candidate.line_widths, right_text),
                )
            },
        )
        .context("artistic layout finalist set is empty")?;
    let maximum_composed = compose_layer(context, maximum_size, selected)?;

    // Artistic mode chooses the best explicit line composition and then uses its
    // maximum safe size. `--target-fill` remains a Balanced-mode control; shrinking
    // an artistic composition after choosing it made titles needlessly timid.
    Ok((maximum_size, maximum_size, maximum_composed, selected_text))
}

fn artistic_layout_score(font_size: f32, line_widths: &[f32], text: &str) -> f64 {
    if line_widths.is_empty() {
        return f64::NEG_INFINITY;
    }
    let widths: Vec<f64> = line_widths.iter().map(|&width| width as f64).collect();
    let mean = widths.iter().sum::<f64>() / widths.len() as f64;
    if mean <= f64::EPSILON {
        return f64::NEG_INFINITY;
    }
    let variance = widths
        .iter()
        .map(|width| (width - mean).powi(2))
        .sum::<f64>()
        / widths.len() as f64;
    let imbalance = variance.sqrt() / mean;
    let last_ratio = widths.last().copied().unwrap_or(mean) / mean;
    let orphan_penalty = (0.58 - last_ratio).max(0.0);
    let max_width = widths.iter().copied().fold(0.0_f64, f64::max).max(1.0);
    let adjacent_raggedness = widths
        .windows(2)
        .map(|pair| (pair[0] - pair[1]).abs() / max_width)
        .sum::<f64>()
        / widths.len().saturating_sub(1).max(1) as f64;

    let syntax_penalty = line_break_syntax_penalty(text);

    font_size as f64
        * (1.0
            - 0.30 * imbalance
            - 0.24 * orphan_penalty
            - 0.08 * adjacent_raggedness
            - syntax_penalty)
}

fn line_break_syntax_penalty(text: &str) -> f64 {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return 1.0;
    }

    let mut penalty: f64 = 0.0;
    for line in &lines {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens
            .iter()
            .all(|token| token.chars().all(is_break_punctuation))
        {
            penalty += 0.70;
            continue;
        }
        if tokens.first().is_some_and(is_dangling_leading_punctuation) {
            penalty += 0.10;
        }
        if tokens.last().is_some_and(is_dangling_trailing_punctuation) {
            penalty += 0.14;
        }
    }

    if let Some(last) = lines.last() {
        let words = last
            .split_whitespace()
            .filter(|token| token.chars().any(char::is_alphanumeric))
            .count();
        if words == 1 && lines.len() > 1 {
            penalty += 0.12;
        }
    }
    penalty.min(0.90)
}

fn is_break_punctuation(character: char) -> bool {
    character.is_ascii_punctuation()
        || matches!(
            character,
            '–' | '—' | '…' | '·' | '•' | '“' | '”' | '‘' | '’' | '«' | '»'
        )
}

fn is_dangling_leading_punctuation(token: &&str) -> bool {
    matches!(*token, "-" | "–" | "—" | "," | "." | ";" | ":" | "!" | "?")
}

fn is_dangling_trailing_punctuation(token: &&str) -> bool {
    matches!(*token, "-" | "–" | "—" | "(" | "[" | "{")
}

fn artistic_line_break_candidates(text: &str, max_lines: usize) -> Vec<String> {
    if text.contains('\n') {
        return vec![text.to_string()];
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 1 {
        return vec![text.trim().to_string()];
    }

    // Generate a bounded set of balanced partitions instead of enumerating every
    // possible placement of line breaks (which grows combinatorially with word count).
    // Font-aware scoring happens later, so these are only cheap proposals.
    let line_limit = max_lines.min(words.len()).max(1);
    let mut ranked = Vec::<(f64, String)>::new();
    for lines in 1..=line_limit {
        for bias in [-1_isize, 0, 1] {
            if let Some(candidate) = balanced_partition(&words, lines, bias) {
                let parts: Vec<String> = candidate.lines().map(str::to_owned).collect();
                let score =
                    cheap_line_balance_score(&parts) + line_break_syntax_penalty(&candidate);
                ranked.push((score, candidate));
            }
        }
    }

    ranked.sort_by(|left, right| {
        left.0
            .total_cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
    });
    ranked.dedup_by(|left, right| left.1 == right.1);
    ranked.truncate(32);
    ranked.into_iter().map(|(_, text)| text).collect()
}

fn balanced_partition(words: &[&str], lines: usize, bias: isize) -> Option<String> {
    if lines == 0 || lines > words.len() {
        return None;
    }
    if lines == 1 {
        return Some(words.join(" "));
    }

    // Prefix lengths approximate visual width cheaply. The expensive font-aware
    // measurement is deliberately deferred until after the proposal set is bounded.
    let mut prefix = Vec::with_capacity(words.len() + 1);
    prefix.push(0usize);
    for (index, word) in words.iter().enumerate() {
        let separator = usize::from(index > 0);
        prefix.push(prefix[index] + separator + word.chars().count());
    }
    let total = *prefix.last()? as f64;

    let mut previous = 0usize;
    let mut breaks = Vec::with_capacity(lines - 1);
    for split in 1..lines {
        let min_end = previous + 1;
        let max_end = words.len() - (lines - split);
        let target = total * split as f64 / lines as f64;

        let nearest = (min_end..=max_end)
            .min_by(|&left, &right| {
                let left_distance = (prefix[left] as f64 - target).abs();
                let right_distance = (prefix[right] as f64 - target).abs();
                left_distance
                    .total_cmp(&right_distance)
                    .then_with(|| left.cmp(&right))
            })
            .expect("non-empty line-break search range");

        let shifted = (nearest as isize + bias).clamp(min_end as isize, max_end as isize) as usize;
        let end = shifted.max(previous + 1);
        breaks.push(end);
        previous = end;
    }

    let mut begin = 0usize;
    let mut parts = Vec::with_capacity(lines);
    for end in breaks.into_iter().chain(std::iter::once(words.len())) {
        if end <= begin {
            return None;
        }
        parts.push(words[begin..end].join(" "));
        begin = end;
    }
    Some(parts.join("\n"))
}

fn cheap_line_balance_score(lines: &[String]) -> f64 {
    let lengths: Vec<f64> = lines
        .iter()
        .map(|line| line.chars().count().max(1) as f64)
        .collect();
    let mean = lengths.iter().sum::<f64>() / lengths.len() as f64;
    let variance = lengths
        .iter()
        .map(|length| (length - mean).powi(2))
        .sum::<f64>()
        / lengths.len() as f64;
    let last_ratio = lengths.last().copied().unwrap_or(mean) / mean.max(1.0);
    variance.sqrt() / mean.max(1.0) + (0.55 - last_ratio).max(0.0) * 2.0
}

fn find_maximum_candidate(
    font_system: &mut FontSystem,
    text: &str,
    upper: f32,
    context: &TypographyContext<'_>,
    wrap: Wrap,
) -> Result<(f32, Candidate)> {
    let mut low = 5.0_f32;
    let mut high = upper;
    let mut best: Option<(f32, Candidate)> = None;
    for _ in 0..32 {
        let size = (low + high) * 0.5;
        let candidate =
            measure_candidate(font_system, text, size, context.max_lines, context, wrap)?;
        if candidate.fits(context.max_lines) {
            best = Some((size, candidate));
            low = size + 0.05;
        } else {
            high = size - 0.05;
        }
    }
    best.context(
        "text cannot fit the plaque even at the minimum font size; increase --max-lines, reduce --padding, use a narrower font, or shorten the text",
    )
}

fn find_maximum_masked_candidate(
    font_system: &mut FontSystem,
    text: &str,
    rectangle_maximum: f32,
    context: &TypographyContext<'_>,
    wrap: Wrap,
) -> Result<(f32, FittedCandidate)> {
    let minimum_size = 5.0_f32;
    let maximum = evaluate_masked_candidate(font_system, text, rectangle_maximum, context, wrap)?;
    if let Some(result) = maximum
        && result.clipped_pixels == 0
    {
        return Ok((rectangle_maximum, result));
    }

    let minimum = evaluate_masked_candidate(font_system, text, minimum_size, context, wrap)?;
    let Some(mut best) = minimum.filter(|candidate| candidate.clipped_pixels == 0) else {
        bail!(
            "text cannot fit the irregular plaque mask without clipping even at the minimum size; increase --padding, increase --max-lines, use a narrower font, or shorten the text"
        );
    };
    let mut low = minimum_size;
    let mut high = rectangle_maximum;
    for _ in 0..12 {
        if high - low <= 0.05 {
            break;
        }
        let candidate_size = (low + high) * 0.5;
        match evaluate_masked_candidate(font_system, text, candidate_size, context, wrap)? {
            Some(result) if result.clipped_pixels == 0 => {
                low = candidate_size;
                best = result;
            }
            _ => high = candidate_size,
        }
    }
    Ok((low, best))
}

fn evaluate_masked_candidate(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    context: &TypographyContext<'_>,
    wrap: Wrap,
) -> Result<Option<FittedCandidate>> {
    let candidate = rasterize_candidate(font_system, text, size, context.max_lines, context, wrap)?;
    if !candidate.fits(context.max_lines) {
        return Ok(None);
    }
    probe_masked_candidate(context, size, candidate).map(Some)
}

fn format_bounds(bounds: Option<(u32, u32, u32, u32)>) -> String {
    match bounds {
        Some((x0, y0, x1, y1)) => format!("x={x0}..{x1}, y={y0}..{y1}"),
        None => "an unknown region".to_string(),
    }
}

struct Composed {
    layer: Surface,
    glyph_mask: Vec<u8>,
    lines: usize,
    fill_ratio: f64,
    minimum_padding_ratio: f64,
    clipped_pixels: u64,
    missing_glyphs: usize,
    fallback_glyphs: usize,
}

struct FittedCandidate {
    candidate: Candidate,
    fill_ratio: f64,
    minimum_padding_ratio: f64,
    clipped_pixels: u64,
    clipped_bounds: Option<(u32, u32, u32, u32)>,
}

fn position_candidate_alpha(
    context: &TypographyContext<'_>,
    candidate: Candidate,
) -> Result<(Vec<u8>, Candidate)> {
    let high_width = context.width * context.supersampling;
    let high_height = context.height * context.supersampling;
    let mut high_resolution = vec![0_u8; high_width as usize * high_height as usize];
    let x = (context.bounds.0 + context.pad_x) * context.supersampling;
    let y = (context.bounds.1 + context.pad_y) * context.supersampling;
    let block_bounds = alpha_mask_bounds(&candidate.alpha, context.raster_width)
        .context("font produced no visible glyphs")?;
    let block_height = block_bounds.3 - block_bounds.1 + 1;
    let vertical_offset = match context.vertical_align {
        VerticalAlign::Top => -(block_bounds.1 as i32),
        VerticalAlign::Center => {
            ((context.raster_height - block_height) / 2) as i32 - block_bounds.1 as i32
        }
        VerticalAlign::Bottom => {
            (context.raster_height - block_height) as i32 - block_bounds.1 as i32
        }
    };
    blit_alpha(
        &mut high_resolution,
        high_width,
        high_height,
        &candidate.alpha,
        context.raster_width,
        context.raster_height,
        x as i32,
        y as i32 + vertical_offset,
    );
    Ok((high_resolution, candidate))
}

fn apply_layout_transform(context: &TypographyContext<'_>, high_alpha: Vec<u8>) -> Result<Vec<u8>> {
    let high = Surface::from_alpha_mask(
        context.width * context.supersampling,
        context.height * context.supersampling,
        &high_alpha,
        Rgba::new(255, 255, 255, 255),
    )?;
    Ok(context.style.layout_transform(&high)?.alpha_mask())
}

fn probe_masked_candidate(
    context: &TypographyContext<'_>,
    font_size: f32,
    candidate: Candidate,
) -> Result<FittedCandidate> {
    let (high_alpha, candidate) = position_candidate_alpha(context, candidate)?;
    let high_alpha = apply_layout_transform(context, high_alpha)?;

    // Fit safety is based on glyphs and hard geometry such as stroke/extrusion. Glow
    // and blurred shadows are intentionally soft: their tails may be clipped by the
    // writable mask rather than forcing the title to shrink.
    let glyph_alpha = downsample_alpha(
        &high_alpha,
        context.width * context.supersampling,
        context.height * context.supersampling,
        context.width,
        context.height,
    )?;
    let alpha_before = context.style.fit_envelope_alpha(
        &glyph_alpha,
        context.width,
        context.height,
        font_size / context.supersampling as f32,
    );
    let alpha_after: Vec<u8> = alpha_before
        .iter()
        .zip(context.mask)
        .map(|(&alpha, &mask)| ((alpha as u16 * mask as u16 + 127) / 255) as u8)
        .collect();
    let (clipped_pixels, clipped_bounds) =
        clipping_summary(&alpha_before, &alpha_after, context.width);
    let final_bounds =
        alpha_mask_bounds(&alpha_after, context.width).context("final text layer is empty")?;
    let block_area =
        (final_bounds.2 - final_bounds.0 + 1) as f64 * (final_bounds.3 - final_bounds.1 + 1) as f64;
    let safe_width = (context.bounds.2 - context.bounds.0 + 1)
        .saturating_sub(2 * context.pad_x)
        .max(1);
    let safe_height = (context.bounds.3 - context.bounds.1 + 1)
        .saturating_sub(2 * context.pad_y)
        .max(1);
    let safe_area = (safe_width as f64 * safe_height as f64).max(1.0);
    let minimum_padding_ratio = [
        final_bounds.0.saturating_sub(context.bounds.0),
        final_bounds.1.saturating_sub(context.bounds.1),
        context.bounds.2.saturating_sub(final_bounds.2),
        context.bounds.3.saturating_sub(final_bounds.3),
    ]
    .into_iter()
    .min()
    .unwrap_or(0) as f64
        / safe_width.min(safe_height).max(1) as f64;

    Ok(FittedCandidate {
        candidate,
        fill_ratio: (block_area / safe_area).clamp(0.0, 1.0),
        minimum_padding_ratio,
        clipped_pixels,
        clipped_bounds,
    })
}

fn compose_layer(
    context: &TypographyContext<'_>,
    font_size: f32,
    fitted: FittedCandidate,
) -> Result<Composed> {
    let (high_alpha, candidate) = position_candidate_alpha(context, fitted.candidate)?;
    let high_alpha = apply_layout_transform(context, high_alpha)?;
    let high_resolution = Surface::from_alpha_mask(
        context.width * context.supersampling,
        context.height * context.supersampling,
        &high_alpha,
        Rgba::new(235, 255, 255, 255),
    )?;
    // Full paint is performed once, after geometry selection. Rejected probes never
    // evaluate RGB material, blur, bevel, or the linear-light compositor.
    let combined = context
        .style
        .compose(&high_resolution, font_size, context.supersampling)?;
    let mut layer = downsample(&combined, context.width, context.height)?;
    layer.apply_alpha_mask(context.mask)?;
    let mut glyph_mask = downsample_alpha(
        &high_alpha,
        context.width * context.supersampling,
        context.height * context.supersampling,
        context.width,
        context.height,
    )?;
    for (alpha, &mask) in glyph_mask.iter_mut().zip(context.mask) {
        *alpha = ((*alpha as u16 * mask as u16 + 127) / 255) as u8;
    }
    Ok(Composed {
        layer,
        glyph_mask,
        lines: candidate.lines,
        fill_ratio: fitted.fill_ratio,
        minimum_padding_ratio: fitted.minimum_padding_ratio,
        clipped_pixels: fitted.clipped_pixels,
        missing_glyphs: candidate.missing_glyphs,
        fallback_glyphs: candidate.fallback_glyphs,
    })
}

struct Candidate {
    alpha: Vec<u8>,
    bounds: Option<(u32, u32, u32, u32)>,
    lines: usize,
    line_widths: Vec<f32>,
    missing_glyphs: usize,
    fallback_glyphs: usize,
}

impl Candidate {
    fn fits(&self, max_lines: usize) -> bool {
        self.bounds.is_some()
            && self.lines > 0
            && self.lines <= max_lines
            && self.missing_glyphs == 0
            && self.fallback_glyphs == 0
    }
}

fn rasterize_candidate(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    max_lines: usize,
    context: &TypographyContext<'_>,
    wrap: Wrap,
) -> Result<Candidate> {
    shape_candidate(font_system, text, size, max_lines, context, wrap, true)
}

fn measure_candidate(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    max_lines: usize,
    context: &TypographyContext<'_>,
    wrap: Wrap,
) -> Result<Candidate> {
    shape_candidate(font_system, text, size, max_lines, context, wrap, false)
}

#[allow(clippy::too_many_arguments)]
fn shape_candidate(
    font_system: &mut FontSystem,
    text: &str,
    size: f32,
    max_lines: usize,
    context: &TypographyContext<'_>,
    wrap: Wrap,
    rasterize: bool,
) -> Result<Candidate> {
    let line_height = size * context.line_height_ratio;
    let mut buffer = Buffer::new(font_system, Metrics::new(size, line_height));
    buffer.set_size(
        Some(context.raster_width as f32),
        Some(context.raster_height as f32),
    );
    buffer.set_wrap(wrap);
    buffer.set_text(
        text,
        &Attrs::new()
            .family(Family::Name(context.family))
            .weight(Weight(600)),
        Shaping::Advanced,
        Some(*context.align),
    );
    buffer.shape_until_scroll(font_system, true);

    let mut lines = 0usize;
    let mut max_width = 0.0_f32;
    let mut measured_height = 0.0_f32;
    let mut line_widths = Vec::new();
    let mut missing_glyphs = 0usize;
    let mut fallback_glyphs = 0usize;
    for run in buffer.layout_runs() {
        lines += 1;
        max_width = max_width.max(run.line_w);
        line_widths.push(run.line_w);
        measured_height = measured_height.max(run.line_top + run.line_height);
        for glyph in run.glyphs {
            if glyph.glyph_id == 0 {
                missing_glyphs += 1;
            }
            if glyph.font_id != context.requested_face {
                fallback_glyphs += 1;
            }
        }
    }

    if lines == 0
        || lines > max_lines
        || max_width > context.raster_width as f32 + 0.5
        || measured_height > context.raster_height as f32 + 0.5
    {
        return Ok(Candidate {
            alpha: Vec::new(),
            bounds: None,
            lines,
            line_widths: Vec::new(),
            missing_glyphs,
            fallback_glyphs,
        });
    }

    if !rasterize {
        return Ok(Candidate {
            alpha: Vec::new(),
            // Measurement already proved that the shaped block is visible and in
            // bounds. Exact ink bounds are needed only by masked finalists.
            bounds: Some((0, 0, 0, 0)),
            lines,
            line_widths,
            missing_glyphs,
            fallback_glyphs,
        });
    }

    let mut alpha = vec![0_u8; context.raster_width as usize * context.raster_height as usize];
    let mut cache = SwashCache::new();
    buffer.draw(
        font_system,
        &mut cache,
        Color::rgba(235, 255, 255, 255),
        |x, y, width, height, color| {
            let (_, _, _, source_alpha) = color.as_rgba_tuple();
            for py in y..y.saturating_add(height as i32) {
                for px in x..x.saturating_add(width as i32) {
                    if px < 0
                        || py < 0
                        || px >= context.raster_width as i32
                        || py >= context.raster_height as i32
                    {
                        continue;
                    }
                    let value =
                        &mut alpha[py as usize * context.raster_width as usize + px as usize];
                    let remaining = (255 - *value as u16) * (255 - source_alpha as u16);
                    *value = (255 - (remaining + 127) / 255) as u8;
                }
            }
        },
    );
    let bounds = alpha_mask_bounds(&alpha, context.raster_width);
    Ok(Candidate {
        alpha,
        bounds,
        lines,
        line_widths,
        missing_glyphs,
        fallback_glyphs,
    })
}

fn clipping_summary(
    alpha_before: &[u8],
    alpha_after: &[u8],
    width: u32,
) -> (u64, Option<(u32, u32, u32, u32)>) {
    let mut clipped_pixels = 0_u64;
    let mut clipped_bounds: Option<(u32, u32, u32, u32)> = None;
    for (index, (&before, &after)) in alpha_before.iter().zip(alpha_after).enumerate() {
        if before > 4 && after.saturating_add(4) < before {
            clipped_pixels += 1;
            let x = index as u32 % width;
            let y = index as u32 / width;
            clipped_bounds = Some(match clipped_bounds {
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                None => (x, y, x, y),
            });
        }
    }
    (clipped_pixels, clipped_bounds)
}

fn discover_family(font_system: &FontSystem, path: &Path) -> Result<(String, fontdb::ID)> {
    let face = font_system
        .db()
        .faces()
        .next()
        .with_context(|| format!("{} contained no usable font face", path.display()))?;
    let family = face
        .families
        .first()
        .map(|(name, _)| name.clone())
        .context("font has no family name")?;
    Ok((family, face.id))
}

fn mask_bounds(width: u32, height: u32, mask: &[u8]) -> Option<(u32, u32, u32, u32)> {
    if mask.len() != width as usize * height as usize {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    let mut any = false;
    for y in 0..height {
        for x in 0..width {
            if mask[(y * width + x) as usize] > 32 {
                any = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    any.then_some((x0, y0, x1, y1))
}

fn alpha_mask_bounds(mask: &[u8], width: u32) -> Option<(u32, u32, u32, u32)> {
    if width == 0 || mask.is_empty() || !mask.len().is_multiple_of(width as usize) {
        return None;
    }
    mask_bounds(width, (mask.len() / width as usize) as u32, mask)
}

#[allow(clippy::too_many_arguments)]
fn blit_alpha(
    destination: &mut [u8],
    destination_width: u32,
    destination_height: u32,
    alpha: &[u8],
    width: u32,
    height: u32,
    dx: i32,
    dy: i32,
) {
    debug_assert_eq!(
        destination.len(),
        destination_width as usize * destination_height as usize
    );
    debug_assert_eq!(alpha.len(), width as usize * height as usize);
    for y in 0..height {
        let destination_y = dy + y as i32;
        if destination_y < 0 || destination_y >= destination_height as i32 {
            continue;
        }
        for x in 0..width {
            let destination_x = dx + x as i32;
            if destination_x < 0 || destination_x >= destination_width as i32 {
                continue;
            }
            let coverage = alpha[(y * width + x) as usize];
            if coverage > 0 {
                destination[destination_y as usize * destination_width as usize
                    + destination_x as usize] = coverage;
            }
        }
    }
}

fn downsample_alpha(
    alpha: &[u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let image = image::GrayImage::from_raw(source_width, source_height, alpha.to_vec())
        .context("invalid supersampled alpha surface")?;
    Ok(image::imageops::resize(&image, width, height, FilterType::Lanczos3).into_raw())
}

fn downsample(surface: &Surface, width: u32, height: u32) -> Result<Surface> {
    let image = RgbaImage::from_raw(surface.width(), surface.height(), surface.pixels().to_vec())
        .context("invalid supersampled text surface")?;
    let resized = image::imageops::resize(&image, width, height, FilterType::Lanczos3);
    Surface::from_rgba(width, height, resized.into_raw())
}

#[cfg(test)]
mod tests {
    use super::{
        artistic_layout_score, artistic_line_break_candidates, balanced_partition, clipping_summary,
    };

    #[test]
    fn artistic_candidates_preserve_explicit_line_breaks() {
        let text = "first line\nsecond line";
        assert_eq!(
            artistic_line_break_candidates(text, 5),
            vec![text.to_string()]
        );
    }

    #[test]
    fn artistic_candidates_are_bounded_and_include_multiple_layouts() {
        let text = "Nós que aqui estamos, por vós ansiosamente esperamos";
        let candidates = artistic_line_break_candidates(text, 5);
        assert!(!candidates.is_empty());
        assert!(candidates.len() <= 32);
        assert!(candidates.iter().any(|candidate| candidate.contains('\n')));
        assert!(
            candidates
                .iter()
                .all(|candidate| candidate.lines().count() <= 5)
        );
    }

    #[test]
    fn balanced_partition_never_emits_empty_lines() {
        let words = ["one", "two", "three", "four", "five"];
        for bias in [-1, 0, 1] {
            let candidate = balanced_partition(&words, 4, bias).expect("partition");
            assert_eq!(candidate.lines().count(), 4);
            assert!(candidate.lines().all(|line| !line.trim().is_empty()));
        }
    }

    #[test]
    fn artistic_score_prefers_balanced_lines_at_equal_font_size() {
        let balanced = artistic_layout_score(
            100.0,
            &[300.0, 290.0, 305.0],
            "alpha beta\ngamma delta\nepsilon zeta",
        );
        let ragged = artistic_layout_score(
            100.0,
            &[420.0, 250.0, 80.0],
            "alpha beta gamma\ndelta epsilon\nzeta",
        );
        assert!(balanced > ragged);
    }

    #[test]
    fn artistic_score_penalizes_stranded_punctuation() {
        let clean = artistic_layout_score(
            100.0,
            &[300.0, 300.0],
            "por vós - ansiosamente -\nesperamos juntos",
        );
        let stranded = artistic_layout_score(100.0, &[300.0, 300.0], "por vós\n-");
        assert!(clean > stranded);
    }

    #[test]
    fn clipping_summary_reports_only_visible_alpha_loss() {
        let before = [0, 4, 5, 40, 200, 255, 0, 0];
        let after = [0, 0, 1, 36, 150, 255, 0, 0];
        let (pixels, bounds) = clipping_summary(&before, &after, 4);
        assert_eq!(pixels, 1);
        assert_eq!(bounds, Some((0, 1, 0, 1)));
    }

    #[test]
    fn clipping_summary_returns_bounds_for_irregular_mask_loss() {
        let before = [0, 200, 0, 0, 0, 180, 0, 0, 0, 0, 220, 0];
        let after = [0, 0, 0, 0, 0, 30, 0, 0, 0, 0, 0, 0];
        let (pixels, bounds) = clipping_summary(&before, &after, 4);
        assert_eq!(pixels, 3);
        assert_eq!(bounds, Some((1, 0, 2, 2)));
    }

    // ---- Typography fitting component tests ----
    //
    // These exercise the full typography::render() pipeline on synthetic inputs
    // (a deterministic font + a simple writable mask), verifying layout behavior,
    // input validation, and mask containment without video or FFmpeg.

    use super::super::test_font;
    use super::{RenderRequest, render};
    use crate::application::{FitMode, TextAlign, VerticalAlign};
    use crate::render::effects::{DirectStyleOptions, Style};

    fn default_test_style() -> Style {
        Style::direct(DirectStyleOptions {
            text_color: "#FFFFFFFF",
            stroke_color: "#000000FF",
            glow_color: "#00000000",
            glow_radius: 0,
            stroke_width_ratio: 0.0,
            shadow_offset_x_ratio: 0.0,
            shadow_offset_y_ratio: 0.0,
            shadow_blur_radius: 0,
            shadow_color: "#00000000",
        })
        .unwrap()
    }

    #[test]
    fn artistic_mode_produces_multi_line_layout() {
        let width = 200;
        let height = 120;
        let mask = vec![255u8; width as usize * height as usize];
        let style = default_test_style();
        let font = test_font();

        let result = render(RenderRequest {
            width,
            height,
            mask: &mask,
            text: "Nós que aqui estamos por vós esperamos",
            font_path: &font,
            fit_mode: FitMode::Artistic,
            requested_font_size: None,
            supersampling: 1,
            target_fill: 0.80,
            max_lines: 4,
            padding_ratio: 0.03,
            line_height_ratio: 1.08,
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            style: &style,
        })
        .unwrap();

        assert!(
            result.metrics.lines > 1,
            "artistic mode should break into multiple lines, got {}",
            result.metrics.lines
        );
        assert!(
            result.layer.alpha_bounds().is_some(),
            "rendered text mask should have visible alpha"
        );
    }

    #[test]
    fn maximize_mode_uses_full_writable_region() {
        let width = 200;
        let height = 120;
        let mask = vec![255u8; width as usize * height as usize];
        let style = default_test_style();
        let font = test_font();

        let result = render(RenderRequest {
            width,
            height,
            mask: &mask,
            text: "PLAQUE FORGE",
            font_path: &font,
            fit_mode: FitMode::Maximize,
            requested_font_size: None,
            supersampling: 1,
            target_fill: 0.94,
            max_lines: 3,
            padding_ratio: 0.03,
            line_height_ratio: 1.08,
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            style: &style,
        })
        .unwrap();

        assert!(
            result.metrics.fill_ratio > 0.30,
            "maximize mode should use a significant portion of the region, got {:.2}",
            result.metrics.fill_ratio
        );
    }

    #[test]
    fn fixed_size_respects_requested_font_size() {
        let width = 200;
        let height = 120;
        let mask = vec![255u8; width as usize * height as usize];
        let style = default_test_style();
        let font = test_font();
        let requested = 18.0_f32;

        let result = render(RenderRequest {
            width,
            height,
            mask: &mask,
            text: "AB",
            font_path: &font,
            fit_mode: FitMode::Fixed,
            requested_font_size: Some(requested),
            supersampling: 1,
            target_fill: 0.80,
            max_lines: 3,
            padding_ratio: 0.03,
            line_height_ratio: 1.08,
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            style: &style,
        })
        .unwrap();

        assert!(
            (result.metrics.font_size - requested).abs() < 0.5,
            "fixed mode should use requested size {requested:.1}px, got {:.1}px",
            result.metrics.font_size
        );
    }

    #[test]
    fn vertical_alignment_shifts_mask_position() {
        let width = 200;
        let height = 200;
        let mask = vec![255u8; width as usize * height as usize];
        let style = default_test_style();
        let font = test_font();

        let centroid = |va: VerticalAlign| -> f64 {
            let result = render(RenderRequest {
                width,
                height,
                mask: &mask,
                text: "TEST",
                font_path: &font,
                fit_mode: FitMode::Maximize,
                requested_font_size: None,
                supersampling: 1,
                target_fill: 0.80,
                max_lines: 1,
                padding_ratio: 0.03,
                line_height_ratio: 1.08,
                text_align: TextAlign::Center,
                vertical_align: va,
                style: &style,
            })
            .unwrap();
            let alpha = result.layer.alpha_mask();
            let mut weighted_y = 0_u64;
            let mut total_alpha = 0_u64;
            for y in 0..height {
                for x in 0..width {
                    let a = alpha[y as usize * width as usize + x as usize] as u64;
                    weighted_y += a * y as u64;
                    total_alpha += a;
                }
            }
            weighted_y as f64 / total_alpha.max(1) as f64
        };

        let top_centroid = centroid(VerticalAlign::Top);
        let center_centroid = centroid(VerticalAlign::Center);
        let bottom_centroid = centroid(VerticalAlign::Bottom);

        assert!(
            top_centroid < center_centroid,
            "top centroid ({top_centroid:.1}) should be above center ({center_centroid:.1})"
        );
        assert!(
            center_centroid < bottom_centroid,
            "center centroid ({center_centroid:.1}) should be above bottom ({bottom_centroid:.1})"
        );
    }

    #[test]
    fn empty_text_is_rejected() {
        let width = 100;
        let height = 60;
        let mask = vec![255u8; width as usize * height as usize];
        let style = default_test_style();
        let font = test_font();

        let result = render(RenderRequest {
            width,
            height,
            mask: &mask,
            text: "   ",
            font_path: &font,
            fit_mode: FitMode::Artistic,
            requested_font_size: None,
            supersampling: 1,
            target_fill: 0.80,
            max_lines: 3,
            padding_ratio: 0.03,
            line_height_ratio: 1.08,
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            style: &style,
        });

        assert!(result.is_err(), "whitespace-only text should be rejected");
    }

    #[test]
    fn text_mask_is_contained_within_writable_region() {
        let width = 120;
        let height = 80;
        // Create a mask with a 10px transparent border
        let inset = 10_u32;
        let mut mask = vec![0u8; width as usize * height as usize];
        for y in inset..(height - inset) {
            for x in inset..(width - inset) {
                mask[y as usize * width as usize + x as usize] = 255;
            }
        }
        let style = default_test_style();
        let font = test_font();

        let result = render(RenderRequest {
            width,
            height,
            mask: &mask,
            text: "HI",
            font_path: &font,
            fit_mode: FitMode::Maximize,
            requested_font_size: None,
            supersampling: 1,
            target_fill: 0.80,
            max_lines: 1,
            padding_ratio: 0.05,
            line_height_ratio: 1.08,
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            style: &style,
        })
        .unwrap();

        // The rendered text mask's alpha should not extend into the transparent border,
        // with 2px tolerance for anti-aliasing at boundaries.
        let tolerance = 2;
        if let Some((min_x, min_y, max_x, max_y)) = result.layer.alpha_bounds() {
            assert!(
                min_x + tolerance >= inset
                    && min_y + tolerance >= inset
                    && max_x < width - inset + tolerance
                    && max_y < height - inset + tolerance,
                "text mask extends outside writable region: \
                 alpha bounds ({min_x},{min_y})-({max_x},{max_y}), \
                 writable starts at ({inset},{inset})"
            );
        }
    }
}
