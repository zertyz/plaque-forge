use plaque_forge::{color::Rgba, surface::Surface, writable_region::WritableRegion};

#[test]
fn rgba_hex_parsing_and_properties() {
    let gold = Rgba::parse("#C98B3C").expect("valid hex");
    assert_eq!(gold.r, 0xC9);
    assert_eq!(gold.g, 0x8B);
    assert_eq!(gold.b, 0x3C);
    assert_eq!(gold.a, 255);
    assert_eq!(gold.as_array(), [0xC9, 0x8B, 0x3C, 255]);

    let transparent_cyan = Rgba::parse("#00FFFF80").expect("valid 8-digit hex");
    assert_eq!(transparent_cyan.r, 0);
    assert_eq!(transparent_cyan.g, 255);
    assert_eq!(transparent_cyan.b, 255);
    assert_eq!(transparent_cyan.a, 0x80);
    assert_eq!(transparent_cyan.as_array(), [0, 255, 255, 0x80]);
}

#[test]
fn surface_blending_preserves_opaque_background() {
    let mut background = Surface::new(100, 100);
    for y in 0..100 {
        for x in 0..100 {
            background.set_pixel(x, y, Rgba::new(50, 100, 150, 255));
        }
    }

    let mut foreground = Surface::new(50, 50);
    for y in 0..50 {
        for x in 0..50 {
            foreground.set_pixel(x, y, Rgba::new(255, 255, 255, 255));
        }
    }

    // Blend foreground box at (25, 25) with 50% opacity
    background.blend_surface(&foreground, 25, 25, 0.5);

    // Outside the box: unaffected
    let outside = background.pixel(10, 10);
    assert_eq!(outside, Rgba::new(50, 100, 150, 255));

    // Inside the box: blended towards white in linear light, remains fully opaque
    let inside = background.pixel(50, 50);
    assert_eq!(inside.a, 255);
    assert!(inside.r > 50, "red channel should be brighter");
    assert!(inside.g > 100, "green channel should be brighter");
    assert!(inside.b > 150, "blue channel should be brighter");
}

#[test]
fn writable_region_geometry_bounds() {
    // 1. Rect
    let rect_region = WritableRegion::Rect {
        bounds: [100.0, 100.0, 300.0, 200.0],
    };
    assert_eq!(rect_region.bounds(), [100.0, 100.0, 300.0, 200.0]);
    assert!(rect_region.validate("rect").is_ok());

    // 2. Rounded Rect
    let rrect_region = WritableRegion::RoundedRect {
        bounds: [100.0, 100.0, 300.0, 200.0],
        radius: 20.0,
    };
    assert_eq!(rrect_region.bounds(), [100.0, 100.0, 300.0, 200.0]);
    assert!(rrect_region.validate("rrect").is_ok());

    // 3. Ellipse
    let ellipse_region = WritableRegion::Ellipse {
        center: [250.0, 200.0],
        radii: [150.0, 100.0],
        rotation_degrees: 0.0,
    };
    assert_eq!(ellipse_region.bounds(), [100.0, 100.0, 300.0, 200.0]);
    assert!(ellipse_region.validate("ellipse").is_ok());
}

#[test]
fn surface_restore_from_mask_honors_feathered_alpha() {
    let mut current = Surface::new(5, 1);
    for x in 0..5 {
        // Red foreground
        current.set_pixel(x, 0, Rgba::new(255, 0, 0, 255));
    }

    let mut original = Surface::new(5, 1);
    for x in 0..5 {
        // Blue original
        original.set_pixel(x, 0, Rgba::new(0, 0, 255, 255));
    }

    // Mask with 0%, 25%, 50%, 75%, 100% restoration
    let mask = vec![0, 64, 128, 192, 255];
    current
        .restore_from_mask(&original, &mask)
        .expect("valid restore");

    // Pixel 0 (mask 0): pure current (red)
    let p0 = current.pixel(0, 0);
    assert_eq!(p0.r, 255);
    assert_eq!(p0.b, 0);

    // Pixel 4 (mask 255): pure original (blue)
    let p4 = current.pixel(4, 0);
    assert_eq!(p4.r, 0);
    assert_eq!(p4.b, 255);

    // Monotonic transition from red to blue across pixels 0..4
    for x in 1..5 {
        let prev = current.pixel(x - 1, 0);
        let curr = current.pixel(x, 0);
        assert!(
            curr.b >= prev.b,
            "blue channel must increase monotonically with mask weight: {prev:?} -> {curr:?}"
        );
        assert!(
            curr.r <= prev.r,
            "red channel must decrease monotonically with mask weight: {prev:?} -> {curr:?}"
        );
    }
}

#[test]
fn surface_apply_alpha_mask_scales_alpha_channel_only() {
    let mut surface = Surface::new(3, 1);
    surface.set_pixel(0, 0, Rgba::new(100, 150, 200, 255));
    surface.set_pixel(1, 0, Rgba::new(100, 150, 200, 255));
    surface.set_pixel(2, 0, Rgba::new(100, 150, 200, 255));

    let mask = vec![255, 128, 0];
    surface.apply_alpha_mask(&mask).expect("valid mask");

    // RGB must remain untouched
    for x in 0..3 {
        let p = surface.pixel(x, 0);
        assert_eq!(p.r, 100);
        assert_eq!(p.g, 150);
        assert_eq!(p.b, 200);
    }

    assert_eq!(surface.pixel(0, 0).a, 255);
    assert!((surface.pixel(1, 0).a as i32 - 128).abs() <= 1);
    assert_eq!(surface.pixel(2, 0).a, 0);
}

#[test]
fn filter_circular_dilation_exact_radius_bounds() {
    use plaque_forge::render::effects::filters::dilate_alpha_circular;

    let size = 11;
    let center = 5;
    let mut source = vec![0u8; size * size];
    source[center * size + center] = 255;

    let radius = 3;
    let dilated = dilate_alpha_circular(&source, size, size, radius);

    // Center must be 255
    assert_eq!(dilated[center * size + center], 255);

    // Cardinal endpoints (x ± radius, y) and (x, y ± radius) must be reached
    assert_eq!(dilated[center * size + (center + radius)], 255);
    assert_eq!(dilated[center * size + (center - radius)], 255);
    assert_eq!(dilated[(center + radius) * size + center], 255);
    assert_eq!(dilated[(center - radius) * size + center], 255);

    // Diagonal corner (center + radius, center + radius) is distance sqrt(18) = 4.24 > 3, must be 0
    assert_eq!(
        dilated[(center + radius) * size + (center + radius)],
        0,
        "circular dilation must not fill square corners"
    );
}

#[test]
fn shader_procedural_material_interpolates_smoothly_within_bounds() {
    use plaque_forge::render::effects::shaders::{gold_color, lerp_color};

    let dark = Rgba::new(80, 50, 20, 255);
    let mid = Rgba::new(180, 140, 50, 255);
    let light = Rgba::new(240, 210, 100, 255);
    let highlight = Rgba::new(255, 255, 220, 255);

    // Lerp extremes
    assert_eq!(lerp_color(dark, light, 0.0), dark);
    assert_eq!(lerp_color(dark, light, 1.0), light);

    // Gold color evaluation over interval
    for step in 0..=100 {
        let t = step as f32 / 100.0;
        let color = gold_color(dark, mid, light, highlight, t);
        assert_eq!(color.a, 255);
        assert!(color.r >= 80, "red channel stays within expected bounds");
        assert!(color.g >= 50, "green channel stays within expected bounds");
    }
}
