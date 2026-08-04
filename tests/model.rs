use approx::assert_abs_diff_eq;
use plaque_forge::model::{Mat3, PointF};

#[test]
fn transforms_homogeneous_points() {
    let matrix = Mat3 {
        values: [[2.0, 0.0, 3.0], [0.0, 2.0, 4.0], [0.0, 0.0, 1.0]],
    };
    let output = matrix.transform(PointF { x: 5.0, y: 7.0 });
    assert_abs_diff_eq!(output.x, 13.0);
    assert_abs_diff_eq!(output.y, 18.0);
}
