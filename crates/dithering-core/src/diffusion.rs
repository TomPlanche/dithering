//! Floyd-Steinberg error diffusion.
//!
//! `image::imageops::dither` clamps the accumulated error back into a `u8` after every step, which throws away most of
//! it on a palette this small and visibly washes the output out. The loop below keeps the running error in `f32` and
//! only clamps the value it matches on.

use image::{GrayImage, Luma, RgbImage};

use crate::palette::Palette;

/// Where the error goes, as `(dx, dy, weight)`.
///
/// Only forward neighbours appear, since the image is traversed left to right, top to bottom.
pub const FLOYD_STEINBERG: [(i32, i32, i32); 4] = [(1, 0, 7), (-1, 1, 3), (0, 1, 5), (1, 1, 1)];

/// What the weights are over. They sum to it, so the kernel conserves the error rather than shedding any.
pub const DIVISOR: i32 = 16;

/// Quantises to `palette`, diffusing the error with Floyd-Steinberg.
///
/// Returns one palette slot per pixel.
///
/// The error buffer covers the whole image rather than just the two rows the kernel can still reach. Keeping only the
/// live rows means addressing them as a ring, and the per-tap cost of that outweighs the locality it buys.
///
/// This stage stays sequential while the rest of the pipeline spreads over the cores. Every pixel reads error that the
/// pixel before it wrote, so rows can only overlap as a wavefront, one row starting once the row above is two pixels
/// ahead. That needs a synchronisation point per row, and at the sizes here the stage is already the cheap one. The
/// cores go to the batch loop instead, where whole photos are independent.
pub fn diffuse(image: &RgbImage, palette: &Palette) -> GrayImage {
    let (width, height) = image.dimensions();
    let mut spill = vec![[0f32; 3]; (width as usize) * (height as usize)];
    let mut indices = GrayImage::new(width, height);
    let divisor = DIVISOR as f32;

    for y in 0..height {
        for x in 0..width {
            let here = (y as usize) * (width as usize) + (x as usize);
            let source = image.get_pixel(x, y).0;

            // Clamp before matching, so a large running error cannot chase the search outside the palette's range.
            let mut wanted = [0u8; 3];
            for channel in 0..3 {
                let v = source[channel] as f32 + spill[here][channel];
                wanted[channel] = v.clamp(0.0, 255.0) as u8;
            }

            let slot = palette.nearest(wanted);
            indices.put_pixel(x, y, Luma([slot as u8]));

            let chosen = palette.colors()[slot];
            let error = [
                wanted[0] as f32 - chosen[0] as f32,
                wanted[1] as f32 - chosen[1] as f32,
                wanted[2] as f32 - chosen[2] as f32,
            ];

            for &(dx, dy, weight) in &FLOYD_STEINBERG {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;

                if nx < 0 || nx >= width as i32 || ny >= height as i32 {
                    continue;
                }

                let there = (ny as usize) * (width as usize) + (nx as usize);
                let share = weight as f32 / divisor;

                for channel in 0..3 {
                    spill[there][channel] += error[channel] * share;
                }
            }
        }
    }

    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::PALETTE_COLORS;

    fn solid(width: u32, height: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(width, height, image::Rgb(rgb))
    }

    #[test]
    fn the_kernel_only_pushes_error_forward() {
        for (dx, dy, weight) in FLOYD_STEINBERG {
            assert!(dy > 0 || (dy == 0 && dx > 0), "({dx},{dy}) looks backwards");
            assert!(weight > 0, "({dx},{dy}) has a non-positive weight");
        }
    }

    #[test]
    fn the_weights_sum_to_the_divisor() {
        let total: i32 = FLOYD_STEINBERG.iter().map(|(_, _, w)| w).sum();

        assert_eq!(total, DIVISOR, "the kernel does not conserve error");
    }

    #[test]
    fn a_palette_colour_comes_back_untouched() {
        let palette = Palette::new(0.6);
        let white = palette.colors()[1];
        let indices = diffuse(&solid(8, 8, white), &palette);

        assert!(
            indices.as_raw().iter().all(|&slot| slot == 1),
            "white should not dither"
        );
    }

    #[test]
    fn a_flat_midtone_uses_more_than_one_slot() {
        let indices = diffuse(&solid(32, 32, [128, 128, 128]), &Palette::new(0.6));
        let mut slots: Vec<u8> = indices.as_raw().to_vec();

        slots.sort_unstable();
        slots.dedup();

        assert!(slots.len() > 1, "a midtone should dither, got {slots:?}");
    }

    #[test]
    fn every_pixel_lands_on_a_real_slot() {
        let image = RgbImage::from_fn(16, 16, |x, y| image::Rgb([x as u8 * 16, y as u8 * 16, 90]));
        let indices = diffuse(&image, &Palette::new(0.6));

        assert!(indices.as_raw().iter().all(|&slot| (slot as usize) < PALETTE_COLORS));
    }

    #[test]
    fn a_single_pixel_image_does_not_panic() {
        assert_eq!(
            diffuse(&solid(1, 1, [200, 30, 30]), &Palette::new(0.6)).dimensions(),
            (1, 1)
        );
    }
}
