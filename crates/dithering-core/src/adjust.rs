//! Brightness and colour boosts, applied before the photo is reduced to the palette.
//!
//! Six colours cannot hold a midtone, so a flat photo lands most of its pixels on the same neighbour. Both adjustments
//! push the pixels apart before that happens, which gives the error diffusion something to work with.
//!
//! Each one interpolates from a reference image toward the photo, and keeps going past it when the factor is over 1.
//! Brightness starts from black, so it is a plain gain. Colour starts from the greyscale version of the photo, so it
//! is a gain on the distance to grey. Both work on gamma-encoded values, which is what a photo arrives in.

use image::RgbImage;
use rayon::prelude::*;

/// One step from `from` toward `to`, clamped back into a `u8`.
///
/// A factor over 1 extrapolates past `to`, which is what turns these into boosts rather than fades.
#[inline]
fn blend(from: u8, to: u8, factor: f32) -> u8 {
    let value = from as f32 + factor * (to as f32 - from as f32);
    value.clamp(0.0, 255.0).round() as u8
}

/// Rec. 601 luma, the grey a pixel is worth.
///
/// Weighted rather than a plain average, because the eye reads green as brighter than blue at the same value. This is
/// the reference [`color`] blends away from, so the choice sets what desaturation looks like.
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32).round() as u8
}

/// Multiplies every channel, clamping at white.
///
/// A factor of 1.0 is a no-op and returns without touching the buffer.
pub fn brightness(image: &mut RgbImage, factor: f32) {
    if factor == 1.0 {
        return;
    }

    for_each_row(image, |row| {
        for channel in row {
            *channel = blend(0, *channel, factor);
        }
    });
}

/// Moves every pixel away from its own grey, clamping at both ends.
///
/// A factor of 1.0 is a no-op, 0.0 leaves a greyscale image, and anything above 1.0 pushes the channels apart.
pub fn color(image: &mut RgbImage, factor: f32) {
    if factor == 1.0 {
        return;
    }

    for_each_row(image, |row| {
        for pixel in row.chunks_exact_mut(3) {
            let grey = luma(pixel[0], pixel[1], pixel[2]);
            for channel in pixel {
                *channel = blend(grey, *channel, factor);
            }
        }
    });
}

/// Runs `body` on every row.
///
/// One pass over the buffer with no pixel depending on another, so the rows go out to the cores. It is a cheap stage at
/// the working size, and not a cheap one under a request that keeps the source resolution.
fn for_each_row(image: &mut RgbImage, body: impl Fn(&mut [u8]) + Sync + Send) {
    let stride = image.width() as usize * 3;
    if stride == 0 {
        return;
    }

    let pixels: &mut [u8] = image;
    pixels.par_chunks_mut(stride).for_each(body);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo() -> RgbImage {
        RgbImage::from_fn(4, 4, |x, y| image::Rgb([x as u8 * 60, y as u8 * 40, 90]))
    }

    #[test]
    fn a_factor_of_one_is_a_no_op() {
        let original = photo();
        let mut image = original.clone();

        brightness(&mut image, 1.0);
        color(&mut image, 1.0);

        assert_eq!(image, original);
    }

    #[test]
    fn brightness_is_a_gain_that_clamps_at_white() {
        let mut image = RgbImage::from_raw(4, 1, vec![0, 1, 100, 200, 231, 232, 254, 255, 128, 10, 20, 30]).unwrap();
        brightness(&mut image, 1.1);

        // 232 * 1.1 is 255.2, so everything above it saturates.
        assert_eq!(image.as_raw()[..8], [0, 1, 110, 220, 254, 255, 255, 255]);
    }

    #[test]
    fn brightness_below_one_darkens_proportionally() {
        let mut image = RgbImage::from_raw(1, 1, vec![200, 100, 50]).unwrap();
        brightness(&mut image, 0.5);

        assert_eq!(image.get_pixel(0, 0).0, [100, 50, 25]);
    }

    #[test]
    fn colour_at_zero_leaves_the_photo_grey() {
        let mut image = photo();
        color(&mut image, 0.0);

        for pixel in image.pixels() {
            let [r, g, b] = pixel.0;
            assert_eq!(r, g, "{pixel:?} is not grey");
            assert_eq!(g, b, "{pixel:?} is not grey");
        }
    }

    #[test]
    fn colour_above_one_pushes_the_channels_apart() {
        let source = [150u8, 90, 50];
        let grey = luma(source[0], source[1], source[2]);
        assert_eq!(grey, 103);

        let mut image = RgbImage::from_raw(1, 1, source.to_vec()).unwrap();
        color(&mut image, 2.0);

        // At 2.0 each channel ends up twice as far from the grey as it started, and the blue runs out of room.
        let [r, g, b] = image.get_pixel(0, 0).0;
        assert_eq!([r, g], [197, 77]);
        assert_eq!(b, 0, "103 - 2 * 53 is below black");
        assert!(r > source[0] && b < source[2]);
    }

    #[test]
    fn a_grey_pixel_is_its_own_reference() {
        let mut image = RgbImage::from_raw(1, 1, vec![120, 120, 120]).unwrap();
        color(&mut image, 3.0);

        assert_eq!(image.get_pixel(0, 0).0, [120, 120, 120]);
    }

    #[test]
    fn luma_weights_green_over_blue() {
        assert_eq!(luma(255, 255, 255), 255);
        assert_eq!(luma(0, 0, 0), 0);
        assert!(luma(0, 255, 0) > luma(255, 0, 0));
        assert!(luma(255, 0, 0) > luma(0, 0, 255));
    }
}
