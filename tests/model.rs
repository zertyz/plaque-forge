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

#[test]
fn matrix_inverse_round_trips_points() {
    let matrix = Mat3 {
        values: [[1.1, 0.2, 3.0], [-0.1, 0.9, 4.0], [0.0002, -0.0001, 1.0]],
    };
    let point = PointF { x: 50.0, y: 70.0 };
    let output = matrix.inverse().unwrap().transform(matrix.transform(point));

    assert_abs_diff_eq!(output.x, point.x, epsilon = 1.0e-9);
    assert_abs_diff_eq!(output.y, point.y, epsilon = 1.0e-9);
}
