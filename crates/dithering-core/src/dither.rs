//! The dithering stage: reduce a photo to the fixed 6-colour palette.

use image::RgbImage;

use crate::diffusion;
use crate::indexed::IndexedImage;
use crate::palette::Palette;

/// Everything the dithering stage reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DitherOptions {
    /// How far the palette is blended toward its muted end. See [`crate::palette::palette_blend`].
    pub saturation: f64,
}

impl Default for DitherOptions {
    fn default() -> Self {
        Self { saturation: 0.6 }
    }
}

/// Dithers a photo to the palette `options` asks for.
///
/// The photo is taken at the size it arrives in. Resizing, cropping and reframing are [`crate::resize`]'s, and run
/// before this.
pub fn apply_dithering(image: &RgbImage, options: &DitherOptions) -> IndexedImage {
    let palette = Palette::new(options.saturation);
    let indices = diffusion::diffuse(image, &palette);

    IndexedImage::new(indices, palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette::PALETTE_COLORS;

    fn solid(width: u32, height: u32, rgb: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(width, height, image::Rgb(rgb))
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
        let pure = apply_dithering(&image, &DitherOptions { saturation: 0.0 });
        let muted = apply_dithering(&image, &DitherOptions { saturation: 1.0 });

        assert_eq!(pure.to_rgb().get_pixel(0, 0).0, [255, 0, 0]);
        assert_eq!(muted.to_rgb().get_pixel(0, 0).0, [156, 72, 75]);
    }
}
