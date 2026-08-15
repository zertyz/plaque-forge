use std::{fs, path::Path};

use plaque_forge::{
    homologation::HomologationContract,
    scene::{Scene, resolve_relative},
};
use sha2::{Digest, Sha256};

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn homologated_assets_pin_scene_geometry_and_source_identity() {
    let root = repository_root().join("assets/homologation");
    assert!(root.is_dir(), "assets/homologation is missing");

    let mut contracts = 0;
    for entry in fs::read_dir(&root).unwrap() {
        let entry = entry.unwrap();
        let contract_path = entry.path().join("contract.toml");
        if !contract_path.is_file() {
            continue;
        }
        contracts += 1;
        let contract = HomologationContract::load(&contract_path).unwrap_or_else(|error| {
            panic!(
                "invalid homologation contract {}: {error:#}",
                contract_path.display()
            )
        });

        let source = resolve_relative(&contract_path, &contract.source);
        let source_bytes = fs::read(&source)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", source.display()));
        assert_eq!(
            Sha256::digest(&source_bytes)
    .iter()
    .map(|b| format!("{:02x}", b))
    .collect::<String>(),
            contract.source_sha256,
            "homologated source identity changed for {}",
            contract.asset
        );

        let scene_path = resolve_relative(&contract_path, &contract.scene);
        let scene = Scene::load(&scene_path)
            .unwrap_or_else(|error| panic!("failed to load {}: {error:#}", scene_path.display()));
        let surface = scene
            .surfaces
            .iter()
            .find(|surface| surface.id == contract.surface)
            .unwrap_or_else(|| {
                panic!(
                    "homologated surface {:?} is missing from {}",
                    contract.surface,
                    scene_path.display()
                )
            });
        assert_eq!(
            surface.bounds,
            Some(contract.geometry.tracking_bounds),
            "tracking geometry changed for homologated asset {}",
            contract.asset
        );
        assert_eq!(
            surface.writable_region.as_ref().map(|region| region.bounds()),
            Some(contract.geometry.writable_bounds),
            "writable geometry changed for homologated asset {}",
            contract.asset
        );

        let analysis = resolve_relative(&contract_path, &contract.analysis);
        assert!(
            analysis.join("manifest.toml").is_file(),
            "homologated analysis cache is missing for {}",
            contract.asset
        );
    }

    assert!(contracts > 0, "no homologation contracts were found");
}

#[test]
fn capability_matrix_makes_unprotected_behavior_explicit() {
    let path = repository_root().join("assets/homologation/capabilities.toml");
    let matrix = plaque_forge::homologation::matrix::CapabilityMatrix::load(&path)
        .unwrap_or_else(|error| panic!("invalid capability matrix {}: {error:#}", path.display()));
    let report = matrix.report();
    assert!(
        report.capabilities >= 8,
        "expected a representative capability matrix"
    );
    assert!(
        report.homologated >= 1,
        "at least one capability must be human-homologated"
    );
    assert!(
        report.ci_protected >= 1,
        "at least one homologated capability must run in CI"
    );
    assert!(
        !report.complete,
        "new capability classes must not be silently treated as homologated before human acceptance"
    );
}

#[test]
fn every_homologation_contract_is_represented_by_the_capability_matrix() {
    let root = repository_root().join("assets/homologation");
    let matrix_path = root.join("capabilities.toml");
    let matrix = plaque_forge::homologation::matrix::CapabilityMatrix::load(&matrix_path).unwrap();
    let represented = matrix
        .capabilities
        .iter()
        .filter_map(|entry| entry.contract.as_ref())
        .map(|path| matrix_path.parent().unwrap().join(path))
        .collect::<std::collections::BTreeSet<_>>();

    for entry in fs::read_dir(&root).unwrap() {
        let entry = entry.unwrap();
        let contract = entry.path().join("contract.toml");
        if contract.is_file() {
            assert!(
                represented.contains(&contract),
                "homologation contract is absent from capability matrix: {}",
                contract.display()
            );
        }
    }
}
