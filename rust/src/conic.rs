//! Two-dimensional coordinate guide and conic-section math.
use crate::app::{ConicDirection, ConicKind, CoordGuide};

pub fn to_screen(coord: &CoordGuide, u: f32, v: f32) -> (f32, f32) {
    let (sin, cos) = coord.rotation.sin_cos();
    let sx = u * cos + v * sin;
    let sy = u * sin - v * cos;
    (coord.origin_x + sx * coord.scale, coord.origin_y + sy * coord.scale)
}

pub fn to_math(coord: &CoordGuide, x: f32, y: f32) -> (f32, f32) {
    let (sin, cos) = coord.rotation.sin_cos();
    let dx = (x - coord.origin_x) / coord.scale.max(0.001);
    let dy = (y - coord.origin_y) / coord.scale.max(0.001);
    (dx * cos + dy * sin, dx * sin - dy * cos)
}

pub fn visible_bounds(coord: &CoordGuide, x0: f32, y0: f32, x1: f32, y1: f32) -> (f32, f32, f32, f32) {
    let corners = [to_math(coord, x0, y0), to_math(coord, x1, y0), to_math(coord, x0, y1), to_math(coord, x1, y1)];
    let mut u0 = f32::INFINITY; let mut u1 = f32::NEG_INFINITY;
    let mut v0 = f32::INFINITY; let mut v1 = f32::NEG_INFINITY;
    for &(u, v) in &corners {
        u0 = u0.min(u); u1 = u1.max(u); v0 = v0.min(v); v1 = v1.max(v);
    }
    (u0, u1, v0, v1)
}

/// Normalized size slider (0..1) to a conic dimension multiplier.
pub fn size_scale(size: f32) -> f32 {
    0.4 + 1.2 * size.clamp(0.0, 1.0)
}

/// (semi-major, semi-minor or focal parameter) from the normalized shape slider.
pub fn ellipse_axes(shape: f32, size: f32) -> (f32, f32) {
    let a = 4.0 * size_scale(size);
    let b = a * (0.30 + 0.70 * shape.clamp(0.0, 1.0));
    (a, b)
}
pub fn hyperbola_axes(shape: f32, size: f32) -> (f32, f32) {
    let a = 3.0 * size_scale(size);
    let b = a * (0.25 + 1.25 * shape.clamp(0.0, 1.0));
    (a, b)
}
pub fn parabola_p(shape: f32, size: f32) -> f32 {
    (0.8 + 4.2 * shape.clamp(0.0, 1.0)) * size_scale(size)
}

pub fn foci(coord: &CoordGuide) -> Vec<(f32, f32)> {
    match coord.conic {
        ConicKind::None => Vec::new(),
        ConicKind::Ellipse => {
            let (a, b) = ellipse_axes(coord.shape, coord.size);
            let c = (a * a - b * b).max(0.0).sqrt();
            match coord.direction {
                ConicDirection::Vertical => vec![(0.0, c), (0.0, -c)],
                _ => vec![(c, 0.0), (-c, 0.0)],
            }
        }
        ConicKind::Hyperbola => {
            let (a, b) = hyperbola_axes(coord.shape, coord.size);
            let c = (a * a + b * b).sqrt();
            match coord.direction {
                ConicDirection::Vertical => vec![(0.0, c), (0.0, -c)],
                _ => vec![(c, 0.0), (-c, 0.0)],
            }
        }
        ConicKind::Parabola => {
            let p = parabola_p(coord.shape, coord.size);
            match coord.direction {
                ConicDirection::Left => vec![(-p, 0.0)],
                ConicDirection::Up => vec![(0.0, p)],
                ConicDirection::Down => vec![(0.0, -p)],
                _ => vec![(p, 0.0)],
            }
        }
    }
}

/// Sample the conic into branches of math-space points, limited to the visible
/// math rectangle so a rotated/zoomed guide does not emit huge coordinates.
pub fn branches(coord: &CoordGuide, u0: f32, u1: f32, v0: f32, v1: f32) -> Vec<Vec<(f32, f32)>> {
    const N: usize = 220;
    match coord.conic {
        ConicKind::None => Vec::new(),
        ConicKind::Ellipse => {
            let (a, b) = ellipse_axes(coord.shape, coord.size);
            let points = (0..=N).map(|i| {
                let t = i as f32 / N as f32 * std::f32::consts::TAU;
                match coord.direction {
                    ConicDirection::Vertical => (b * t.sin(), a * t.cos()),
                    _ => (a * t.cos(), b * t.sin()),
                }
            }).collect();
            vec![points]
        }
        ConicKind::Hyperbola => {
            let (a, b) = hyperbola_axes(coord.shape, coord.size);
            let mut left = Vec::new();
            let mut right = Vec::new();
            let tmax = 2.2;
            for i in 0..=N {
                let t = -tmax + 2.0 * tmax * i as f32 / N as f32;
                let (x, y) = (a * t.cosh(), b * t.sinh());
                match coord.direction {
                    // Vertical hyperbola: x = ±b·sinh(t), y = ±a·cosh(t) has
                    // upper and lower branches, not left/right half-curves.
                    ConicDirection::Vertical => {
                        left.push((y, x)); right.push((y, -x));
                    }
                    _ => {
                        left.push((-x, y)); right.push((x, y));
                    }
                }
            }
            vec![left, right]
        }
        ConicKind::Parabola => {
            let p = parabola_p(coord.shape, coord.size);
            let mut points = Vec::new();
            match coord.direction {
                ConicDirection::Left | ConicDirection::Right => {
                    let hi = v1.max(v0.abs()).max(0.1) + 2.0;
                    let lo = v0.min(-v1.abs()).min(-0.1) - 2.0;
                    for i in 0..=N {
                        let y = lo + (hi - lo) * i as f32 / N as f32;
                        let x = y * y / (4.0 * p);
                        points.push(if coord.direction == ConicDirection::Left { (-x, y) } else { (x, y) });
                    }
                }
                direction => {
                    let hi = u1.max(u0.abs()).max(0.1) + 2.0;
                    let lo = u0.min(-u1.abs()).min(-0.1) - 2.0;
                    for i in 0..=N {
                        let x = lo + (hi - lo) * i as f32 / N as f32;
                        let y = x * x / (4.0 * p);
                        points.push(if direction == ConicDirection::Down { (x, -y) } else { (x, y) });
                    }
                }
            }
            vec![points]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::CoordGuide;

    #[test]
    fn foci_follow_direction_and_shape() {
        let mut c = CoordGuide::new(100, 100, ConicKind::Ellipse, ConicDirection::Horizontal);
        c.shape = 0.0;
        let f = foci(&c);
        assert!((f[0].0 - f[1].0).abs() > 0.1);
        c.direction = ConicDirection::Vertical;
        let f = foci(&c);
        assert!((f[0].1 - f[1].1).abs() > 0.1);
        c.conic = ConicKind::Parabola;
        c.direction = ConicDirection::Right;
        assert_eq!(foci(&c).len(), 1);
    }

    #[test]
    fn size_scales_conic_dimensions() {
        let small = ellipse_axes(0.5, 0.0);
        let large = ellipse_axes(0.5, 1.0);
        assert!(large.0 > small.0 && large.1 > small.1);
        assert!(parabola_p(0.5, 1.0) > parabola_p(0.5, 0.0));
        let mut c = CoordGuide::new(300, 200, ConicKind::Ellipse, ConicDirection::Horizontal);
        c.shape = 0.5;
        c.size = 0.0;
        let near = foci(&c)[0].0;
        c.size = 1.0;
        assert!(foci(&c)[0].0 > near);
    }

    #[test]
    fn vertical_hyperbola_has_upper_and_lower_branches() {
        let c = CoordGuide::new(300, 200, ConicKind::Hyperbola, ConicDirection::Vertical);
        let branches = branches(&c, -10.0, 10.0, -10.0, 10.0);
        assert_eq!(branches.len(), 2);
        let mut upper = false;
        let mut lower = false;
        for branch in &branches {
            let positive = branch.iter().filter(|(_, v)| *v > 0.01).count();
            let negative = branch.iter().filter(|(_, v)| *v < -0.01).count();
            assert!(positive == 0 || negative == 0, "a vertical branch cannot cross the axis");
            upper |= positive > 0;
            lower |= negative > 0;
        }
        assert!(upper && lower, "vertical hyperbola needs both halves");
    }

    #[test]
    fn mapping_round_trips() {
        let mut c = CoordGuide::new(300, 200, ConicKind::None, ConicDirection::Horizontal);
        c.rotation = 0.7; c.scale = 37.0;
        let (x, y) = to_screen(&c, 2.0, -3.0);
        let (u, v) = to_math(&c, x, y);
        assert!((u - 2.0).abs() < 0.001 && (v + 3.0).abs() < 0.001);
    }
}
