//! The dithering stage: reduce a photo to the fixed 6-colour palette.

use std::borrow::Cow;

use image::RgbImage;

use crate::adjust;
use crate::diffusion;
use crate::indexed::IndexedImage;
use crate::palette::Palette;

/// Everything the dithering stage reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DitherOptions {
    /// How far the palette is blended toward its muted end. See [`crate::palette::palette_blend`].
    ///
    /// This names the six colours the photo lands on. It does not touch the photo, which is [`Self::color`].
    pub saturation: f64,
    /// Gain applied to the photo before dithering. See [`adjust::brightness`].
    pub brightness: f64,
    /// How far the photo's pixels are pushed away from grey before dithering. See [`adjust::color`].
    pub color: f64,
}

impl Default for DitherOptions {
    /// The boosts are on by default, because six colours cannot hold a midtone and a flat photo dithers to mud.
    fn default() -> Self {
        Self {
            saturation: 0.6,
            brightness: 1.1,
            color: 1.4,
        }
    }
}

/// Dithers a photo to the palette `options` asks for.
///
/// The photo is taken at the size it arrives in. Resizing, cropping and reframing are [`crate::resize`]'s, and run
/// before this.
///
/// The two boosts run first, brightness then colour, and only when they have something to do: at a factor of 1.0 each
/// the photo goes into the dither untouched, without the copy the boosts would need.
pub fn apply_dithering(image: &RgbImage, options: &DitherOptions) -> IndexedImage {
    let work = if options.brightness == 1.0 && options.color == 1.0 {
        Cow::Borrowed(image)
    } else {
        let mut work = image.clone();
        adjust::brightness(&mut work, options.brightness as f32);
        adjust::color(&mut work, options.color as f32);
        Cow::Owned(work)
    };

    let palette = Palette::new(options.saturation);
    let indices = diffusion::diffuse(&work, &palette);

    IndexedImage::new(indices, palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::PALETTE_COLORS;

    fn solid(width: u32, height: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(width, height, image::Rgb(rgb))
    }

    /// The pipeline with both boosts off, which is the photo as it arrived.
    fn plain(saturation: f64) -> DitherOptions {
        DitherOptions {
            saturation,
            brightness: 1.0,
            color: 1.0,
        }
    }

    #[test]
    fn white_stays_white() {
        let out = apply_dithering(&solid(8, 8, [255, 255, 255]), &DitherOptions::default());

        assert!(
            out.indices().iter().all(|&slot| slot == 1),
            "white should map to slot 1"
        );
    }

    #[test]
    fn the_output_keeps_the_input_size() {
        let out = apply_dithering(&solid(13, 7, [200, 30, 30]), &DitherOptions::default());

        assert_eq!(out.size(), (13, 7));
    }

    #[test]
    fn every_slot_stays_inside_the_palette() {
        let image = RgbImage::from_fn(24, 24, |x, y| image::Rgb([x as u8 * 10, y as u8 * 10, 60]));
        let out = apply_dithering(&image, &DitherOptions::default());

        assert!(out.indices().iter().all(|&slot| (slot as usize) < PALETTE_COLORS));
    }

    #[test]
    fn saturation_picks_the_palette_the_output_expands_through() {
        let image = solid(4, 4, [255, 0, 0]);
        let pure = apply_dithering(&image, &plain(0.0));
        let muted = apply_dithering(&image, &plain(1.0));

        assert_eq!(pure.to_rgb().get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(muted.to_rgb().get_pixel(0, 0).0, [156, 72, 75]);
    }

    #[test]
    fn the_boosts_run_before_the_dither() {
        // A dull olive: too dark and too grey to reach the yellow slot on its own.
        let dull = solid(16, 16, [120, 110, 60]);

        let untouched = apply_dithering(&dull, &plain(0.6));
        let boosted = apply_dithering(&dull, &DitherOptions::default());

        assert_ne!(
            untouched.indices(),
            boosted.indices(),
            "the boosts should change the result"
        );
    }

    #[test]
    fn boosts_of_one_take_the_photo_as_it_is() {
        // The borrow that skips the copy has to give what the copy would have given.
        let image = RgbImage::from_fn(12, 12, |x, y| image::Rgb([x as u8 * 20, y as u8 * 15, 70]));

        let borrowed = apply_dithering(&image, &plain(0.6));

        let mut copy = image.clone();
        adjust::brightness(&mut copy, 1.0);
        adjust::color(&mut copy, 1.0);
        let copied = apply_dithering(&copy, &plain(0.6));

        assert_eq!(borrowed.indices(), copied.indices());
    }
}
