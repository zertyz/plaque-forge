//! Capability-oriented homologation coverage.
//!
//! The matrix answers a different question from an individual contract: whether every
//! behavior class that matters to the renderer has a deliberately chosen regression
//! sentinel, and whether that sentinel has actually been human-homologated yet.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use super::HomologationContract;

pub const CAPABILITY_MATRIX_FORMAT: &str = "plaque-forge.homologation-capabilities/1";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMatrix {
    pub format: String,
    pub capabilities: Vec<CapabilityEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityEntry {
    pub id: String,
    pub description: String,
    pub representative_asset: String,
    pub scene: PathBuf,
    pub contract: Option<PathBuf>,
    #[serde(default)]
    pub ci: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityCoverage {
    pub id: String,
    pub description: String,
    pub representative_asset: String,
    pub scene: String,
    pub homologated: bool,
    pub ci: bool,
    pub contract: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CapabilityCoverageReport {
    pub schema_version: u32,
    pub capabilities: usize,
    pub homologated: usize,
    pub ci_protected: usize,
    pub complete: bool,
    pub coverage: Vec<CapabilityCoverage>,
}

impl CapabilityMatrix {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read capability matrix {}", path.display()))?;
        let matrix: Self = toml::from_str(&text)
            .with_context(|| format!("failed to parse capability matrix {}", path.display()))?;
        matrix.validate(path)?;
        Ok(matrix)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        ensure!(
            self.format == CAPABILITY_MATRIX_FORMAT,
            "unsupported capability matrix format {:?}; expected {CAPABILITY_MATRIX_FORMAT:?}",
            self.format
        );
        ensure!(
            !self.capabilities.is_empty(),
            "capability matrix contains no capabilities"
        );
        let mut ids = BTreeSet::new();
        for capability in &self.capabilities {
            ensure!(
                !capability.id.trim().is_empty(),
                "capability id must not be empty"
            );
            ensure!(
                ids.insert(&capability.id),
                "capability {:?} is declared more than once",
                capability.id
            );
            ensure!(
                !capability.description.trim().is_empty(),
                "capability {:?} has no description",
                capability.id
            );
            ensure!(
                !capability.representative_asset.trim().is_empty(),
                "capability {:?} has no representative asset",
                capability.id
            );
            let scene = resolve(path, &capability.scene);
            ensure!(
                scene.is_file(),
                "capability {:?} scene is missing: {}",
                capability.id,
                scene.display()
            );
            let scene_document = crate::scene::Scene::load(&scene)?;
            let source = crate::scene::resolve_relative(&scene, &scene_document.source);
            ensure!(
                source.file_stem().and_then(|stem| stem.to_str())
                    == Some(capability.representative_asset.as_str()),
                "capability {:?} representative {:?} does not match scene source {}",
                capability.id,
                capability.representative_asset,
                source.display()
            );
            if let Some(contract) = &capability.contract {
                let contract_path = resolve(path, contract);
                let contract = HomologationContract::load(&contract_path)?;
                ensure!(
                    contract.asset == capability.representative_asset,
                    "capability {:?} representative {:?} differs from contract asset {:?}",
                    capability.id,
                    capability.representative_asset,
                    contract.asset
                );
            } else {
                ensure!(
                    !capability.ci,
                    "capability {:?} cannot be a CI sentinel without a homologation contract",
                    capability.id
                );
            }
        }
        Ok(())
    }

    pub fn report(&self) -> CapabilityCoverageReport {
        let coverage = self
            .capabilities
            .iter()
            .map(|capability| CapabilityCoverage {
                id: capability.id.clone(),
                description: capability.description.clone(),
                representative_asset: capability.representative_asset.clone(),
                scene: capability.scene.to_string_lossy().into_owned(),
                homologated: capability.contract.is_some(),
                ci: capability.ci,
                contract: capability
                    .contract
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
            })
            .collect::<Vec<_>>();
        let homologated = coverage.iter().filter(|entry| entry.homologated).count();
        let ci_protected = coverage.iter().filter(|entry| entry.ci).count();
        CapabilityCoverageReport {
            schema_version: 1,
            capabilities: coverage.len(),
            homologated,
            ci_protected,
            complete: homologated == coverage.len(),
            coverage,
        }
    }
}

fn resolve(owner: &Path, referenced: &Path) -> PathBuf {
    owner
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(referenced)
}
