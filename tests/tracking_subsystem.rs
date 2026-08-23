mod support;

use plaque_forge::{
    geometry::{Point, Quad, homography},
    model::{Mat3, PointF},
};
use support::synthetic::{
    BackgroundPattern, SyntheticMotion, SyntheticOccluder, SyntheticSequenceBuilder,
};

#[test]
fn homography_recovers_exact_translation() {
    let source = Quad::from_rect(100.0, 100.0, 400.0, 200.0);
    let (dx, dy) = (35.5, -18.25);
    let target = Quad::new(
        Point::new(source.tl.x + dx, source.tl.y + dy),
        Point::new(source.tr.x + dx, source.tr.y + dy),
        Point::new(source.br.x + dx, source.br.y + dy),
        Point::new(source.bl.x + dx, source.bl.y + dy),
    );

    let h = homography(source, target).expect("homography failed");
    for (src_pt, expected_tgt) in [
        (source.tl, target.tl),
        (source.tr, target.tr),
        (source.br, target.br),
        (source.bl, target.bl),
        (Point::new(300.0, 200.0), Point::new(335.5, 181.75)),
    ] {
        let mapped = h.transform(src_pt).expect("valid point transform");
        assert!(
            (mapped.x - expected_tgt.x).abs() < 1e-6,
            "x mapped ({}) != expected ({})",
            mapped.x,
            expected_tgt.x
        );
        assert!(
            (mapped.y - expected_tgt.y).abs() < 1e-6,
            "y mapped ({}) != expected ({})",
            mapped.y,
            expected_tgt.y
        );
    }
}

#[test]
fn homography_recovers_rotation_and_scale() {
    let source = Quad::from_rect(0.0, 0.0, 200.0, 100.0);
    let angle: f64 = 0.5; // ~28.6 degrees
    let scale: f64 = 1.5;
    let (sin, cos) = angle.sin_cos();

    let transform = |p: Point| -> Point {
        Point::new(
            scale * (p.x * cos - p.y * sin) + 50.0,
            scale * (p.x * sin + p.y * cos) + 80.0,
        )
    };

    let target = Quad::new(
        transform(source.tl),
        transform(source.tr),
        transform(source.br),
        transform(source.bl),
    );

    let h = homography(source, target).expect("homography failed");
    for p in [source.tl, source.tr, source.br, source.bl] {
        let expected = transform(p);
        let mapped = h.transform(p).expect("valid rotation transform");
        assert!((mapped.x - expected.x).abs() < 1e-5);
        assert!((mapped.y - expected.y).abs() < 1e-5);
    }
}

#[test]
fn quad_orientation_and_validation_rejects_degenerate_geometry() {
    let valid = Quad::from_rect(10.0, 10.0, 100.0, 50.0);
    assert!(valid.validate("valid rect").is_ok());
    assert!(valid.orientation() > 0.0);

    // Inverted/crossed quad (bowtie)
    let bowtie = Quad::new(
        Point::new(0.0, 0.0),
        Point::new(100.0, 100.0),
        Point::new(0.0, 100.0),
        Point::new(100.0, 0.0),
    );
    assert!(bowtie.validate("bowtie").is_err());

    // Degenerate zero-area collinear line
    let flat_line = Quad::new(
        Point::new(0.0, 0.0),
        Point::new(50.0, 0.0),
        Point::new(100.0, 0.0),
        Point::new(25.0, 0.0),
    );
    assert!(flat_line.validate("flat_line").is_err());
}

#[test]
fn matrix_inverse_and_point_mapping_round_trips() {
    let mat = Mat3 {
        values: [[1.2, -0.3, 45.0], [0.4, 1.1, -12.0], [0.0005, -0.0002, 1.0]],
    };
    let inv = mat.inverse().expect("matrix invertible");
    let pt = PointF { x: 150.0, y: 75.0 };
    let mapped = mat.transform(pt);
    let roundtrip = inv.transform(mapped);

    assert!((roundtrip.x - pt.x).abs() < 1e-4);
    assert!((roundtrip.y - pt.y).abs() < 1e-4);
}

#[test]
fn synthetic_sequence_ground_truth_trajectory_matches_analytical_homography() {
    let builder = SyntheticSequenceBuilder::new(320, 240, 10)
        .with_motion(SyntheticMotion::LinearTranslation {
            dx_per_frame: 2.5,
            dy_per_frame: -1.2,
        })
        .with_background(BackgroundPattern::Checkerboard { block_size: 16 });

    let frames = builder.build_frames();
    assert_eq!(frames.len(), 10);

    for frame_idx in 0..10 {
        let quad = builder.ground_truth_quad(frame_idx);
        assert!(quad.validate("synthetic quad").is_ok());

        // Verify homography mapping from frame 0
        let h = builder
            .ground_truth_homography(0, frame_idx)
            .expect("valid homography");
        let base = builder.ground_truth_quad(0);
        for (src_pt, expected_tgt) in [
            (base.tl, quad.tl),
            (base.tr, quad.tr),
            (base.br, quad.br),
            (base.bl, quad.bl),
        ] {
            let mapped = h.transform(PointF {
                x: src_pt.x,
                y: src_pt.y,
            });
            assert!(
                (mapped.x - expected_tgt.x).abs() < 1e-3,
                "frame {frame_idx} x mapped ({}) != expected ({})",
                mapped.x,
                expected_tgt.x
            );
            assert!(
                (mapped.y - expected_tgt.y).abs() < 1e-3,
                "frame {frame_idx} y mapped ({}) != expected ({})",
                mapped.y,
                expected_tgt.y
            );
        }
    }
}

#[test]
fn synthetic_occlusion_modifies_pixels_while_ground_truth_geometry_persists() {
    let occluder = SyntheticOccluder::new(
        2,
        6,
        40.0,
        80.0,
        Point::new(100.0, 60.0),
        Point::new(200.0, 60.0),
    );

    let clean_builder = SyntheticSequenceBuilder::new(320, 240, 8)
        .with_motion(SyntheticMotion::Static)
        .with_background(BackgroundPattern::DiagonalGradient);

    let occluded_builder = SyntheticSequenceBuilder::new(320, 240, 8)
        .with_motion(SyntheticMotion::Static)
        .with_background(BackgroundPattern::DiagonalGradient)
        .with_occluder(occluder);

    let clean_frames = clean_builder.build_frames();
    let occluded_frames = occluded_builder.build_frames();

    // Frame 0: no occlusion yet, frames must be identical
    assert_eq!(
        clean_frames[0].pixels(),
        occluded_frames[0].pixels(),
        "frame 0 without occluder should match exactly"
    );

    // Frame 4: occluder active, pixels in occluder region must differ
    let clean_p4 = clean_frames[4].pixels();
    let occluded_p4 = occluded_frames[4].pixels();
    let differing_pixels = clean_p4
        .as_chunks::<4>()
        .0
        .iter()
        .zip(occluded_p4.as_chunks::<4>().0.iter())
        .filter(|(c, o)| c != o)
        .count();

    assert!(
        differing_pixels > 2000,
        "occluded frame 4 should have substantial pixel differences, found {differing_pixels}"
    );

    // Ground truth quad must remain uncorrupted by the occluder
    assert_eq!(
        clean_builder.ground_truth_quad(4),
        occluded_builder.ground_truth_quad(4),
        "analytical ground truth quad must be independent of visual occlusion"
    );
}

#[test]
fn trajectory_jerk_metric_sensitively_detects_single_frame_jitter() {
    let builder = SyntheticSequenceBuilder::new(320, 240, 10).with_motion(
        SyntheticMotion::LinearTranslation {
            dx_per_frame: 2.0,
            dy_per_frame: 1.0,
        },
    );

    let smooth_quads: Vec<Quad> = (0..10).map(|f| builder.ground_truth_quad(f)).collect();

    // Calculate second-order acceleration/jerk: ||(p[t+1] - p[t]) - (p[t] - p[t-1])||
    let max_acceleration = |quads: &[Quad]| -> f64 {
        let mut max_acc = 0.0_f64;
        for i in 1..(quads.len() - 1) {
            let prev = quads[i - 1].tl;
            let curr = quads[i].tl;
            let next = quads[i + 1].tl;
            let acc_x = (next.x - curr.x) - (curr.x - prev.x);
            let acc_y = (next.y - curr.y) - (curr.y - prev.y);
            max_acc = max_acc.max(acc_x.hypot(acc_y));
        }
        max_acc
    };

    let smooth_acc = max_acceleration(&smooth_quads);
    assert!(
        smooth_acc < 1e-9,
        "constant velocity trajectory should have near-zero acceleration, got {smooth_acc}"
    );

    // Inject a small 1.5 pixel jitter into frame 5
    let mut jittery_quads = smooth_quads.clone();
    jittery_quads[5].tl.x += 1.5;
    jittery_quads[5].tl.y -= 1.0;

    let jittery_acc = max_acceleration(&jittery_quads);
    assert!(
        jittery_acc > 1.0,
        "single-frame jitter must be sensitively detected by acceleration metric, got {jittery_acc}"
    );
}
