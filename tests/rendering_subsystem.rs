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
