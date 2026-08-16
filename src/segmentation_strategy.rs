//! Device-independent planning for optional ML segmentation.
//!
//! Rust owns *which* semantic model/refiner/precision policy should run.  Python owns
//! execution of the already-sealed plan.  This keeps scene intent, quality policy,
//! cache identity, and fallback behavior outside the Python implementation details.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::scene::{LayerMatteMode, LayerRole, LayerSubject, SegmentationPrompt};

/// User-visible quality/performance policy.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SegmentationProfile {
    /// Fast iteration.  Uses one general semantic tracker and may enable compilation.
    Preview,
    /// Normal local development.  Keeps the robust semantic ensemble but uses BF16.
    #[default]
    Balanced,
    /// Reproducibility-first acceptance path.  FP32 and no compile-induced variance.
    Canonical,
}

impl SegmentationProfile {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "preview" => Ok(Self::Preview),
            "balanced" => Ok(Self::Balanced),
            "canonical" => Ok(Self::Canonical),
            other => bail!("unsupported segmentation profile {other:?}"),
        }
    }
}

/// Numeric policy is independent from the execution device.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SegmentationPrecision {
    Fp32,
    Bf16,
}

impl SegmentationPrecision {
    pub fn parse(value: &str) -> Result<Option<Self>> {
        match value {
            "auto" => Ok(None),
            "fp32" => Ok(Some(Self::Fp32)),
            "bf16" => Ok(Some(Self::Bf16)),
            other => bail!("unsupported segmentation precision {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SemanticBackend {
    Sam2,
    Cutie,
    Sam2Cutie,
    #[serde(rename = "matanyone2")]
    MatAnyone2,
    /// Optional research backend.  The official Meta runtime currently requires CUDA.
    #[serde(rename = "sam3.1")]
    Sam31,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MatteRefiner {
    None,
    #[serde(rename = "vitmatte")]
    VitMatte,
    /// The semantic backend itself produces optical alpha (MatAnyone2).
    Native,
}

/// Sealed worker execution plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SegmentationPlan {
    pub profile: SegmentationProfile,
    pub precision: SegmentationPrecision,
    pub semantic_backend: SemanticBackend,
    pub semantic_model: String,
    pub matte_refiner: MatteRefiner,
    pub compile: bool,
    /// Human-readable causal explanation included in provenance/diagnostics.
    pub reason: Vec<String>,
}

/// Inputs which may legitimately affect model selection.
pub struct PlanningInput<'a> {
    pub profile: SegmentationProfile,
    pub precision_override: Option<SegmentationPrecision>,
    /// `auto` lets Rust choose. Legacy backend strings remain accepted as explicit policy.
    pub backend_override: &'a str,
    /// `auto` uses the planner's pinned model.
    pub model_override: &'a str,
    pub role: LayerRole,
    pub matte_mode: LayerMatteMode,
    pub subject: LayerSubject,
    pub prompts: &'a [SegmentationPrompt],
}

pub const PREVIEW_SAM2_MODEL: &str = "facebook/sam2.1-hiera-small";
pub const DEFAULT_SAM2_MODEL: &str = "facebook/sam2.1-hiera-large";
pub const DEFAULT_SAM31_MODEL: &str = "facebook/sam3.1";
pub const DEFAULT_MATANYONE2_MODEL: &str = "PeiqingYang/MatAnyone2";

impl SegmentationPlan {
    /// Stable protocol label understood by the Python executor.
    pub fn backend_label(&self) -> &'static str {
        match (self.semantic_backend, self.matte_refiner) {
            (SemanticBackend::Sam2, MatteRefiner::VitMatte) => "sam2-vitmatte",
            (SemanticBackend::Sam2, _) => "sam2",
            (SemanticBackend::Cutie, MatteRefiner::VitMatte) => "cutie-vitmatte",
            (SemanticBackend::Cutie, _) => "cutie",
            (SemanticBackend::Sam2Cutie, MatteRefiner::VitMatte) => "sam2-cutie-vitmatte",
            (SemanticBackend::Sam2Cutie, _) => "sam2-cutie",
            (SemanticBackend::MatAnyone2, _) => "matanyone2",
            (SemanticBackend::Sam31, MatteRefiner::VitMatte) => "sam3.1-vitmatte",
            (SemanticBackend::Sam31, _) => "sam3.1",
        }
    }
}

pub fn plan(input: PlanningInput<'_>) -> Result<SegmentationPlan> {
    let precision = input.precision_override.unwrap_or(match input.profile {
        SegmentationProfile::Preview | SegmentationProfile::Balanced => SegmentationPrecision::Bf16,
        SegmentationProfile::Canonical => SegmentationPrecision::Fp32,
    });

    if input.backend_override != "auto" {
        return explicit_plan(input, precision);
    }

    let optical =
        input.role == LayerRole::Foreground && input.matte_mode == LayerMatteMode::Optical;
    let human_matting_candidate =
        optical && input.subject == LayerSubject::Human && has_frame_zero_area_seed(input.prompts);

    let mut reason = vec![format!("profile={:?}", input.profile).to_lowercase()];
    let (semantic_backend, semantic_model, matte_refiner, compile) = if human_matting_candidate
        && input.profile == SegmentationProfile::Canonical
    {
        reason.push(
            "explicit human subject with frame-0 area seed selects specialist video matting".into(),
        );
        (
            SemanticBackend::MatAnyone2,
            DEFAULT_MATANYONE2_MODEL.to_string(),
            MatteRefiner::Native,
            false,
        )
    } else {
        let semantic_backend = match input.profile {
            SegmentationProfile::Preview => SemanticBackend::Sam2,
            SegmentationProfile::Balanced | SegmentationProfile::Canonical => {
                SemanticBackend::Sam2Cutie
            }
        };
        if input.subject == LayerSubject::Human && !human_matting_candidate {
            reason.push(
                "human specialist not selected because optical matte and frame-0 area seed are required"
                    .into(),
            );
        }
        let matte_refiner = if optical {
            MatteRefiner::VitMatte
        } else {
            MatteRefiner::None
        };
        if matte_refiner == MatteRefiner::None {
            reason.push(
                "categorical/opaque membership does not require optical alpha refinement".into(),
            );
        }
        (
            semantic_backend,
            sam2_model_for_profile(input.profile).to_string(),
            matte_refiner,
            input.profile == SegmentationProfile::Preview,
        )
    };

    Ok(SegmentationPlan {
        profile: input.profile,
        precision,
        semantic_backend,
        semantic_model: override_model(input.model_override, semantic_model),
        matte_refiner,
        compile,
        reason,
    })
}

fn explicit_plan(
    input: PlanningInput<'_>,
    precision: SegmentationPrecision,
) -> Result<SegmentationPlan> {
    let optical =
        input.role == LayerRole::Foreground && input.matte_mode == LayerMatteMode::Optical;
    let (semantic_backend, default_model, matte_refiner) = match input.backend_override {
        "sam2" => (
            SemanticBackend::Sam2,
            sam2_model_for_profile(input.profile),
            MatteRefiner::None,
        ),
        "sam2-vitmatte" => (
            SemanticBackend::Sam2,
            sam2_model_for_profile(input.profile),
            MatteRefiner::VitMatte,
        ),
        "cutie" => (
            SemanticBackend::Cutie,
            sam2_model_for_profile(input.profile),
            MatteRefiner::None,
        ),
        "cutie-vitmatte" => (
            SemanticBackend::Cutie,
            sam2_model_for_profile(input.profile),
            MatteRefiner::VitMatte,
        ),
        "sam2-cutie" => (
            SemanticBackend::Sam2Cutie,
            sam2_model_for_profile(input.profile),
            MatteRefiner::None,
        ),
        "sam2-cutie-vitmatte" => (
            SemanticBackend::Sam2Cutie,
            sam2_model_for_profile(input.profile),
            MatteRefiner::VitMatte,
        ),
        "matanyone2" => {
            if input.subject != LayerSubject::Human {
                bail!(
                    "matanyone2 is a human-video-matting specialist; declare subject = \"human\" on the scene layer before selecting it"
                );
            }
            if !has_frame_zero_area_seed(input.prompts) {
                bail!("matanyone2 requires a frame-0 box/polygon/quad seed");
            }
            (
                SemanticBackend::MatAnyone2,
                DEFAULT_MATANYONE2_MODEL,
                MatteRefiner::Native,
            )
        }
        "sam3.1" | "sam31" | "sam3.1-vitmatte" => {
            if precision != SegmentationPrecision::Bf16 {
                bail!(
                    "SAM 3.1 is experimental and currently integrated only with its documented CUDA/BF16 execution contract; use --segmentation-precision bf16"
                );
            }
            if !sam31_has_supported_prompt(input.prompts) {
                bail!(
                    "sam3.1 bridge currently requires exactly one prompt containing a text concept or positive point; add concept = \"...\" to an authored prompt or choose another backend"
                );
            }
            let matte = if input.backend_override.ends_with("-vitmatte") || optical {
                MatteRefiner::VitMatte
            } else {
                MatteRefiner::None
            };
            (SemanticBackend::Sam31, DEFAULT_SAM31_MODEL, matte)
        }
        other => bail!("unsupported segmentation backend {other:?}"),
    };
    if matches!(matte_refiner, MatteRefiner::VitMatte) && !optical {
        // Explicit requests remain legal for experimentation, but record that the
        // caller deliberately asked for optical refinement of categorical support.
    }
    Ok(SegmentationPlan {
        profile: input.profile,
        precision,
        semantic_backend,
        semantic_model: override_model(input.model_override, default_model.to_string()),
        matte_refiner,
        compile: input.profile == SegmentationProfile::Preview
            && matches!(
                semantic_backend,
                SemanticBackend::Sam2 | SemanticBackend::Sam2Cutie
            ),
        reason: vec![format!(
            "explicit backend override: {}",
            input.backend_override
        )],
    })
}

fn sam2_model_for_profile(profile: SegmentationProfile) -> &'static str {
    match profile {
        SegmentationProfile::Preview => PREVIEW_SAM2_MODEL,
        SegmentationProfile::Balanced | SegmentationProfile::Canonical => DEFAULT_SAM2_MODEL,
    }
}

fn override_model(requested: &str, default_model: String) -> String {
    if requested == "auto" {
        default_model
    } else {
        requested.to_string()
    }
}

fn has_frame_zero_area_seed(prompts: &[SegmentationPrompt]) -> bool {
    prompts.iter().any(|prompt| {
        prompt.frame == 0
            && (prompt.box_bounds.is_some() || prompt.quad.is_some() || !prompt.polygon.is_empty())
    })
}

fn sam31_has_supported_prompt(prompts: &[SegmentationPrompt]) -> bool {
    let [prompt] = prompts else {
        return false;
    };
    prompt
        .concept
        .as_deref()
        .is_some_and(|concept| !concept.trim().is_empty())
        || !prompt.positive_points.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::SpatialCoordinates;

    fn prompt(frame: usize) -> SegmentationPrompt {
        SegmentationPrompt {
            frame,
            coordinates: SpatialCoordinates::SourcePixels,
            object: Some("subject".into()),
            concept: None,
            box_bounds: Some([0.0, 0.0, 10.0, 10.0]),
            positive_points: Vec::new(),
            negative_points: Vec::new(),
            polygon: Vec::new(),
            quad: None,
        }
    }

    #[test]
    fn opaque_foreground_skips_optical_matting() {
        let plan = plan(PlanningInput {
            profile: SegmentationProfile::Balanced,
            precision_override: None,
            backend_override: "auto",
            model_override: "auto",
            role: LayerRole::Foreground,
            matte_mode: LayerMatteMode::Opaque,
            subject: LayerSubject::Unspecified,
            prompts: &[prompt(12)],
        })
        .unwrap();
        assert_eq!(plan.semantic_backend, SemanticBackend::Sam2Cutie);
        assert_eq!(plan.matte_refiner, MatteRefiner::None);
        assert_eq!(plan.precision, SegmentationPrecision::Bf16);
    }

    #[test]
    fn preview_uses_pinned_small_model_and_bf16() {
        let plan = plan(PlanningInput {
            profile: SegmentationProfile::Preview,
            precision_override: None,
            backend_override: "auto",
            model_override: "auto",
            role: LayerRole::WritingSurface,
            matte_mode: LayerMatteMode::Optical,
            subject: LayerSubject::Unspecified,
            prompts: &[prompt(12)],
        })
        .unwrap();
        assert_eq!(plan.semantic_backend, SemanticBackend::Sam2);
        assert_eq!(plan.semantic_model, PREVIEW_SAM2_MODEL);
        assert_eq!(plan.precision, SegmentationPrecision::Bf16);
        assert!(plan.compile);
    }

    #[test]
    fn canonical_precision_is_device_independent_fp32() {
        let plan = plan(PlanningInput {
            profile: SegmentationProfile::Canonical,
            precision_override: None,
            backend_override: "auto",
            model_override: "auto",
            role: LayerRole::WritingSurface,
            matte_mode: LayerMatteMode::Optical,
            subject: LayerSubject::Unspecified,
            prompts: &[prompt(12)],
        })
        .unwrap();
        assert_eq!(plan.precision, SegmentationPrecision::Fp32);
        assert!(!plan.compile);
    }

    #[test]
    fn human_specialist_requires_explicit_semantics_and_seed() {
        let prompts = [prompt(0)];
        let plan = plan(PlanningInput {
            profile: SegmentationProfile::Canonical,
            precision_override: None,
            backend_override: "auto",
            model_override: "auto",
            role: LayerRole::Foreground,
            matte_mode: LayerMatteMode::Optical,
            subject: LayerSubject::Human,
            prompts: &prompts,
        })
        .unwrap();
        assert_eq!(plan.semantic_backend, SemanticBackend::MatAnyone2);
        assert_eq!(plan.matte_refiner, MatteRefiner::Native);
    }

    #[test]
    fn sam31_is_opt_in_and_requires_a_supported_prompt() {
        let mut prompt = prompt(0);
        prompt.concept = Some("person".into());
        let prompts = [prompt];
        let plan = plan(PlanningInput {
            profile: SegmentationProfile::Canonical,
            precision_override: Some(SegmentationPrecision::Bf16),
            backend_override: "sam3.1",
            model_override: "auto",
            role: LayerRole::Foreground,
            matte_mode: LayerMatteMode::Optical,
            subject: LayerSubject::Unspecified,
            prompts: &prompts,
        })
        .unwrap();
        assert_eq!(plan.semantic_backend, SemanticBackend::Sam31);
        assert_eq!(plan.matte_refiner, MatteRefiner::VitMatte);
        assert_eq!(plan.precision, SegmentationPrecision::Bf16);
    }

    #[test]
    fn generic_content_never_accidentally_selects_human_specialist() {
        let prompts = [prompt(0)];
        let plan = plan(PlanningInput {
            profile: SegmentationProfile::Canonical,
            precision_override: None,
            backend_override: "auto",
            model_override: "auto",
            role: LayerRole::Foreground,
            matte_mode: LayerMatteMode::Optical,
            subject: LayerSubject::Unspecified,
            prompts: &prompts,
        })
        .unwrap();
        assert_ne!(plan.semantic_backend, SemanticBackend::MatAnyone2);
    }
}
