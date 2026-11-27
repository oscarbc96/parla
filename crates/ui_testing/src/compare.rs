//! Pixel comparison of a rendered image against a golden one.

use image::{Rgba, RgbaImage};

/// How far two images may differ and still match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Largest per-channel difference that still counts as equal.
    pub channel: u8,
    /// Largest share of pixels (0.0 to 1.0) allowed to differ by more.
    pub max_ratio: f64,
}

impl Default for Tolerance {
    fn default() -> Self {
        Self {
            channel: 2,
            max_ratio: 0.001,
        }
    }
}

/// The result of comparing two images of the same size.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub differing: u64,
    pub total: u64,
    /// The actual image, dimmed, with differing pixels in red.
    pub diff: RgbaImage,
}

impl Comparison {
    /// Share of pixels that differ.
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.differing as f64 / self.total as f64
        }
    }

    /// Whether the difference is within `tol`.
    pub fn within(&self, tol: Tolerance) -> bool {
        self.ratio() <= tol.max_ratio
    }
}

/// The two images have different sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("image size mismatch: actual {actual:?}, expected {expected:?}")]
pub struct SizeMismatch {
    pub actual: (u32, u32),
    pub expected: (u32, u32),
}

/// Compare `actual` with `expected`; a pixel differs when any channel is off by more than `tol.channel`.
pub fn compare(
    actual: &RgbaImage,
    expected: &RgbaImage,
    tol: Tolerance,
) -> Result<Comparison, SizeMismatch> {
    if actual.dimensions() != expected.dimensions() {
        return Err(SizeMismatch {
            actual: actual.dimensions(),
            expected: expected.dimensions(),
        });
    }
    let mut diff = RgbaImage::new(actual.width(), actual.height());
    let mut differing = 0u64;
    for (x, y, a) in actual.enumerate_pixels() {
        let e = expected.get_pixel(x, y);
        let off =
            a.0.iter()
                .zip(e.0.iter())
                .any(|(p, q)| p.abs_diff(*q) > tol.channel);
        if off {
            differing += 1;
            diff.put_pixel(x, y, Rgba([255, 0, 0, 255]));
        } else {
            let grey = ((u32::from(a[0]) + u32::from(a[1]) + u32::from(a[2])) / 3) as u8;
            let dim = 128 + grey / 2;
            diff.put_pixel(x, y, Rgba([dim, dim, dim, 255]));
        }
    }
    Ok(Comparison {
        differing,
        total: u64::from(actual.width()) * u64::from(actual.height()),
        diff,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn identical_images_match() {
        let a = solid(4, 4, [10, 20, 30, 255]);
        let c = compare(&a, &a.clone(), Tolerance::default()).unwrap();
        assert_eq!((c.differing, c.total), (0, 16));
        assert!(c.within(Tolerance::default()));
    }

    #[test]
    fn channel_tolerance_edge() {
        let a = solid(10, 10, [10, 20, 30, 255]);
        let mut b = a.clone();
        b.put_pixel(0, 0, Rgba([12, 20, 30, 255]));
        let tol = Tolerance {
            channel: 2,
            max_ratio: 0.0,
        };
        assert_eq!(compare(&a, &b, tol).unwrap().differing, 0);
        b.put_pixel(0, 0, Rgba([13, 20, 30, 255]));
        let c = compare(&a, &b, tol).unwrap();
        assert_eq!(c.differing, 1);
        assert!(!c.within(tol));
    }

    #[test]
    fn ratio_boundary_is_inclusive() {
        let a = solid(100, 10, [0, 0, 0, 255]);
        let mut b = a.clone();
        b.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        let tol = Tolerance::default();
        assert!(compare(&a, &b, tol).unwrap().within(tol), "exactly 0.1%");
        b.put_pixel(1, 0, Rgba([255, 0, 0, 255]));
        assert!(!compare(&a, &b, tol).unwrap().within(tol), "0.2%");
    }

    #[test]
    fn size_mismatch_is_an_error() {
        let err = compare(
            &solid(2, 2, [0; 4]),
            &solid(3, 2, [0; 4]),
            Tolerance::default(),
        );
        assert_eq!(
            err.unwrap_err(),
            SizeMismatch {
                actual: (2, 2),
                expected: (3, 2)
            }
        );
    }

    #[test]
    fn diff_marks_differing_pixels_red() {
        let a = solid(2, 1, [0, 0, 0, 255]);
        let mut b = a.clone();
        b.put_pixel(1, 0, Rgba([200, 0, 0, 255]));
        let c = compare(&a, &b, Tolerance::default()).unwrap();
        assert_eq!(*c.diff.get_pixel(1, 0), Rgba([255, 0, 0, 255]));
        assert_ne!(*c.diff.get_pixel(0, 0), Rgba([255, 0, 0, 255]));
    }
}
