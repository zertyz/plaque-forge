//! Device-independent planning for optional ML segmentation.
//!
//! Rust owns *which* semantic model/refiner/precision policy should run. Python owns
//! execution of an already-sealed candidate plan. For `auto`, Rust may provide a
//! conservative escalation chain and accepts a cheaper candidate only after independent
//! mask evidence passes the versioned policy in `assets/segmentation/policy.toml`.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::scene::{LayerMatteMode, LayerRole, LayerSubject, SegmentationPrompt};

const POLICY_DOCUMENT: &str = include_str!("../assets/segmentation/policy.toml");

/// User-visible quality/performance policy.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SegmentationProfile {
    /// Fast iteration. Uses SAM2 Small first and may escalate to the large model.
    Preview,
    /// Normal local development. Tries SAM2 Large before paying for Cutie.
    #[default]
    Balanced,
    /// Reproducibility-first acceptance path. Keeps the robust ensemble and FP32.
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Balanced => "balanced",
            Self::Canonical => "canonical",
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

    pub fn label(self) -> &'static str {
        match self {
            Self::Fp32 => "fp32",
            Self::Bf16 => "bf16",
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
    /// Optional research backend. The official Meta runtime currently requires CUDA.
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

/// Sealed worker execution plan. Exactly one of these is sent to Python at a time.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptancePolicy {
    pub min_prompt_alpha_u16: u16,
    pub max_negative_alpha_u16: u16,
    pub min_nonempty_permille: u16,
    pub max_foreground_coverage_permille: u16,
    pub max_surface_coverage_permille: u16,
}

/// Rust-owned strategy. Candidate order is significant: execute the cheapest first and
/// escalate only when independent evidence rejects it. Explicit backend overrides always
/// contain one candidate.
#[derive(Debug, Clone)]
pub struct SegmentationStrategy {
    pub policy_id: String,
    pub acceptance: AcceptancePolicy,
    pub candidates: Vec<SegmentationPlan>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyDocument {
    format: String,
    policy_id: String,
    profiles: PolicyProfiles,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyProfiles {
    preview: PolicyThresholds,
    balanced: PolicyThresholds,
    canonical: PolicyThresholds,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyThresholds {
    min_prompt_alpha_u16: u16,
    max_negative_alpha_u16: u16,
    min_nonempty_permille: u16,
    max_foreground_coverage_permille: u16,
    max_surface_coverage_permille: u16,
}

impl From<PolicyThresholds> for AcceptancePolicy {
    fn from(value: PolicyThresholds) -> Self {
        Self {
            min_prompt_alpha_u16: value.min_prompt_alpha_u16,
            max_negative_alpha_u16: value.max_negative_alpha_u16,
            min_nonempty_permille: value.min_nonempty_permille,
            max_foreground_coverage_permille: value.max_foreground_coverage_permille,
            max_surface_coverage_permille: value.max_surface_coverage_permille,
        }
    }
}

/// Inputs which may legitimately affect model selection.
pub struct PlanningInput<'a> {
    pub profile: SegmentationProfile,
    pub precision_override: Option<SegmentationPrecision>,
    /// `auto` lets Rust choose. Legacy backend strings remain accepted as explicit policy.
    pub backend_override: &'a str,
    /// `auto` uses the model selected by the Rust strategy planner.
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

/// Backwards-compatible single-plan helper used by explicit/low-level callers. Auto mode
/// returns the first candidate; high-level analysis should call [`strategy`] and apply its
/// escalation contract.
pub fn plan(input: PlanningInput<'_>) -> Result<SegmentationPlan> {
    strategy(input)?
        .candidates
        .into_iter()
        .next()
        .context("segmentation strategy unexpectedly has no candidate")
}

pub fn strategy(input: PlanningInput<'_>) -> Result<SegmentationStrategy> {
    let policy = load_policy()?;
    let precision = input.precision_override.unwrap_or(match input.profile {
        SegmentationProfile::Preview | SegmentationProfile::Balanced => SegmentationPrecision::Bf16,
        SegmentationProfile::Canonical => SegmentationPrecision::Fp32,
    });
    let acceptance = thresholds_for(&policy, input.profile).into();

    if input.backend_override != "auto" {
        return Ok(SegmentationStrategy {
            policy_id: policy.policy_id,
            acceptance,
            candidates: vec![explicit_plan(input, precision)?],
        });
    }

    let optical =
        input.role == LayerRole::Foreground && input.matte_mode == LayerMatteMode::Optical;
    let human_matting_candidate = optical
        && input.subject == LayerSubject::Human
        && input.model_override == "auto"
        && has_frame_zero_area_seed(input.prompts);
    let matte_refiner = if optical {
        MatteRefiner::VitMatte
    } else {
        MatteRefiner::None
    };
    let mut common_reason = vec![
        format!("profile={}", input.profile.label()),
        format!("adaptive-policy={}", policy.policy_id),
    ];
    if matte_refiner == MatteRefiner::None {
        common_reason
            .push("categorical/opaque membership does not require optical alpha refinement".into());
    }

    let mut candidates = Vec::new();
    if human_matting_candidate && input.profile == SegmentationProfile::Canonical {
        let mut reason = common_reason.clone();
        reason.push(
            "explicit human subject with frame-0 area seed selects specialist video matting first"
                .into(),
        );
        candidates.push(SegmentationPlan {
            profile: input.profile,
            precision,
            semantic_backend: SemanticBackend::MatAnyone2,
            semantic_model: override_model(
                input.model_override,
                DEFAULT_MATANYONE2_MODEL.to_string(),
            ),
            matte_refiner: MatteRefiner::Native,
            compile: false,
            reason,
        });
        let mut reason = common_reason.clone();
        reason.push(
            "general SAM2+Cutie+ViTMatte fallback if specialist output fails independent evidence"
                .into(),
        );
        candidates.push(SegmentationPlan {
            profile: input.profile,
            precision,
            semantic_backend: SemanticBackend::Sam2Cutie,
            semantic_model: override_model(input.model_override, DEFAULT_SAM2_MODEL.to_string()),
            matte_refiner: MatteRefiner::VitMatte,
            compile: false,
            reason,
        });
    } else {
        if input.subject == LayerSubject::Human && !human_matting_candidate {
            common_reason.push(
                "human specialist not selected because optical matte, automatic model choice, and a frame-0 area seed are required".into(),
            );
        }
        match input.profile {
            SegmentationProfile::Preview => {
                let mut primary_reason = common_reason.clone();
                primary_reason.push("preview primary uses SAM2 Small".into());
                candidates.push(SegmentationPlan {
                    profile: input.profile,
                    precision,
                    semantic_backend: SemanticBackend::Sam2,
                    semantic_model: override_model(
                        input.model_override,
                        PREVIEW_SAM2_MODEL.to_string(),
                    ),
                    matte_refiner,
                    compile: true,
                    reason: primary_reason,
                });
                if input.model_override == "auto" {
                    let mut fallback_reason = common_reason.clone();
                    fallback_reason.push(
                        "preview escalation uses SAM2 Large after independent evidence failure"
                            .into(),
                    );
                    candidates.push(SegmentationPlan {
                        profile: input.profile,
                        precision,
                        semantic_backend: SemanticBackend::Sam2,
                        semantic_model: DEFAULT_SAM2_MODEL.to_string(),
                        matte_refiner,
                        compile: true,
                        reason: fallback_reason,
                    });
                }
            }
            SegmentationProfile::Balanced => {
                let mut primary_reason = common_reason.clone();
                primary_reason.push(
                    "balanced primary avoids Cutie unless independent evidence requests escalation"
                        .into(),
                );
                candidates.push(SegmentationPlan {
                    profile: input.profile,
                    precision,
                    semantic_backend: SemanticBackend::Sam2,
                    semantic_model: override_model(
                        input.model_override,
                        DEFAULT_SAM2_MODEL.to_string(),
                    ),
                    matte_refiner,
                    compile: false,
                    reason: primary_reason,
                });
                let mut fallback_reason = common_reason.clone();
                fallback_reason.push("balanced escalation adds Cutie temporal tracking".into());
                candidates.push(SegmentationPlan {
                    profile: input.profile,
                    precision,
                    semantic_backend: SemanticBackend::Sam2Cutie,
                    semantic_model: override_model(
                        input.model_override,
                        DEFAULT_SAM2_MODEL.to_string(),
                    ),
                    matte_refiner,
                    compile: false,
                    reason: fallback_reason,
                });
            }
            SegmentationProfile::Canonical => {
                let mut reason = common_reason;
                reason.push("canonical keeps robust SAM2+Cutie ensemble until bake-offs justify cheaper acceptance".into());
                candidates.push(SegmentationPlan {
                    profile: input.profile,
                    precision,
                    semantic_backend: SemanticBackend::Sam2Cutie,
                    semantic_model: override_model(
                        input.model_override,
                        DEFAULT_SAM2_MODEL.to_string(),
                    ),
                    matte_refiner,
                    compile: false,
                    reason,
                });
            }
        }
    }

    Ok(SegmentationStrategy {
        policy_id: policy.policy_id,
        acceptance,
        candidates,
    })
}

fn load_policy() -> Result<PolicyDocument> {
    let policy: PolicyDocument =
        toml::from_str(POLICY_DOCUMENT).context("invalid embedded segmentation policy")?;
    if policy.format != "plaque-forge.segmentation-policy/1" || policy.policy_id.trim().is_empty() {
        bail!("unsupported or unnamed embedded segmentation policy");
    }
    Ok(policy)
}

fn thresholds_for(policy: &PolicyDocument, profile: SegmentationProfile) -> PolicyThresholds {
    match profile {
        SegmentationProfile::Preview => policy.profiles.preview,
        SegmentationProfile::Balanced => policy.profiles.balanced,
        SegmentationProfile::Canonical => policy.profiles.canonical,
    }
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
    fn policy_document_is_versioned_and_parseable() {
        let policy = load_policy().unwrap();
        assert_eq!(policy.format, "plaque-forge.segmentation-policy/1");
        assert!(!policy.policy_id.is_empty());
    }

    #[test]
    fn balanced_auto_defers_cutie_until_escalation() {
        let strategy = strategy(PlanningInput {
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
        assert_eq!(strategy.candidates.len(), 2);
        assert_eq!(
            strategy.candidates[0].semantic_backend,
            SemanticBackend::Sam2
        );
        assert_eq!(
            strategy.candidates[1].semantic_backend,
            SemanticBackend::Sam2Cutie
        );
        assert_eq!(strategy.candidates[0].matte_refiner, MatteRefiner::None);
    }

    #[test]
    fn preview_can_escalate_small_to_large_without_changing_precision() {
        let strategy = strategy(PlanningInput {
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
        assert_eq!(strategy.candidates[0].semantic_model, PREVIEW_SAM2_MODEL);
        assert_eq!(strategy.candidates[1].semantic_model, DEFAULT_SAM2_MODEL);
        assert!(
            strategy
                .candidates
                .iter()
                .all(|candidate| candidate.precision == SegmentationPrecision::Bf16)
        );
    }

    #[test]
    fn canonical_precision_is_device_independent_fp32() {
        let strategy = strategy(PlanningInput {
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
        assert_eq!(strategy.candidates.len(), 1);
        assert_eq!(
            strategy.candidates[0].precision,
            SegmentationPrecision::Fp32
        );
        assert_eq!(
            strategy.candidates[0].semantic_backend,
            SemanticBackend::Sam2Cutie
        );
        assert!(!strategy.candidates[0].compile);
    }

    #[test]
    fn human_specialist_has_general_fallback() {
        let prompts = [prompt(0)];
        let strategy = strategy(PlanningInput {
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
        assert_eq!(
            strategy.candidates[0].semantic_backend,
            SemanticBackend::MatAnyone2
        );
        assert_eq!(
            strategy.candidates[1].semantic_backend,
            SemanticBackend::Sam2Cutie
        );
        assert_eq!(strategy.candidates[1].matte_refiner, MatteRefiner::VitMatte);
    }

    #[test]
    fn explicit_backend_disables_adaptive_substitution() {
        let strategy = strategy(PlanningInput {
            profile: SegmentationProfile::Balanced,
            precision_override: None,
            backend_override: "sam2-cutie",
            model_override: "auto",
            role: LayerRole::Foreground,
            matte_mode: LayerMatteMode::Opaque,
            subject: LayerSubject::Unspecified,
            prompts: &[prompt(0)],
        })
        .unwrap();
        assert_eq!(strategy.candidates.len(), 1);
        assert_eq!(
            strategy.candidates[0].semantic_backend,
            SemanticBackend::Sam2Cutie
        );
    }

    #[test]
    fn sam31_is_opt_in_and_requires_supported_prompt() {
        let mut prompt = prompt(0);
        prompt.concept = Some("person".into());
        let plan = plan(PlanningInput {
            profile: SegmentationProfile::Canonical,
            precision_override: Some(SegmentationPrecision::Bf16),
            backend_override: "sam3.1",
            model_override: "auto",
            role: LayerRole::Foreground,
            matte_mode: LayerMatteMode::Optical,
            subject: LayerSubject::Unspecified,
            prompts: &[prompt],
        })
        .unwrap();
        assert_eq!(plan.semantic_backend, SemanticBackend::Sam31);
        assert_eq!(plan.matte_refiner, MatteRefiner::VitMatte);
    }

    #[test]
    fn generic_content_never_accidentally_selects_human_specialist() {
        let strategy = strategy(PlanningInput {
            profile: SegmentationProfile::Canonical,
            precision_override: None,
            backend_override: "auto",
            model_override: "auto",
            role: LayerRole::Foreground,
            matte_mode: LayerMatteMode::Optical,
            subject: LayerSubject::Unspecified,
            prompts: &[prompt(0)],
        })
        .unwrap();
        assert!(
            strategy
                .candidates
                .iter()
                .all(|plan| plan.semantic_backend != SemanticBackend::MatAnyone2)
        );
    }
}
