use plaque_forge::{
    geometry::{Point, Quad, homography},
    model::{Mat3, PointF},
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
