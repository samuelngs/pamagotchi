use super::*;

#[test]
fn hsv_red() {
    let c = hsv_to_rgb(0.0, 1.0, 1.0);
    assert_eq!(c, Rgb::new(255, 0, 0));
}

#[test]
fn hsv_green() {
    let c = hsv_to_rgb(120.0, 1.0, 1.0);
    assert_eq!(c, Rgb::new(0, 255, 0));
}

#[test]
fn hsv_blue() {
    let c = hsv_to_rgb(240.0, 1.0, 1.0);
    assert_eq!(c, Rgb::new(0, 0, 255));
}
