//! The palette image produced by dithering.
//!
//! Pixels are [`image::GrayImage`] values reinterpreted as palette slots.

use image::{GrayImage, RgbImage};
use rayon::prelude::*;

use crate::palette::Palette;

/// An indexed image: one palette slot per pixel, plus the palette itself.
#[derive(Debug, Clone)]
pub struct IndexedImage {
    indices: GrayImage,
    palette: Palette,
}

impl IndexedImage {
    pub fn new(indices: GrayImage, palette: Palette) -> Self {
        Self { indices, palette }
    }

    pub fn width(&self) -> u32 {
        self.indices.width()
    }

    pub fn height(&self) -> u32 {
        self.indices.height()
    }

    /// `(width, height)`.
    pub fn size(&self) -> (u32, u32) {
        self.indices.dimensions()
    }

    /// The raw palette slots, row major.
    pub fn indices(&self) -> &[u8] {
        self.indices.as_raw()
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    /// Expands the slots back into a full RGB image.
    ///
    /// One output row per source row, and the rows do not depend on each other, so they run in parallel.
    pub fn to_rgb(&self) -> RgbImage {
        let colors = self.palette.colors();
        let (width, height) = self.size();
        if width == 0 || height == 0 {
            return RgbImage::new(width, height);
        }

        let slots = self.indices.as_raw();
        let stride = width as usize;
        let mut out = vec![0u8; stride * (height as usize) * 3];

        out.par_chunks_mut(stride * 3)
            .zip(slots.par_chunks(stride))
            .for_each(|(row, slots)| {
                for (cell, &slot) in row.chunks_exact_mut(3).zip(slots) {
                    cell.copy_from_slice(&colors[slot as usize]);
                }
            });

        RgbImage::from_raw(width, height, out).expect("buffer matches the dimensions")
    }

    /// Nearest-neighbour integer upscale, which keeps the dither pattern crisp rather than smearing it.
    ///
    /// A factor of 1 or 0 hands the image back as it is.
    ///
    /// Written out rather than left to `imageops::resize`: an integer factor is plain replication, so an output row is
    /// a source row with every byte repeated, and the rows run in parallel.
    pub fn scale_nearest(&self, factor: u32) -> IndexedImage {
        let (width, height) = self.size();
        if factor <= 1 || width == 0 || height == 0 {
            return self.clone();
        }

        let f = factor as usize;
        let src_stride = width as usize;
        let out_stride = src_stride * f;
        let mut out = vec![0u8; out_stride * (height as usize) * f];
        let slots = self.indices.as_raw();

        out.par_chunks_mut(out_stride).enumerate().for_each(|(y, row)| {
            let source = &slots[(y / f) * src_stride..(y / f) * src_stride + src_stride];
            for (cell, &slot) in row.chunks_exact_mut(f).zip(source) {
                cell.fill(slot);
            }
        });

        IndexedImage {
            indices: GrayImage::from_raw(width * factor, height * factor, out).expect("buffer matches the dimensions"),
            palette: self.palette.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_nearest_replicates_pixels() {
        let indices = GrayImage::from_raw(2, 1, vec![1, 5]).unwrap();
        let scaled = IndexedImage::new(indices, Palette::new(0.6)).scale_nearest(2);

        assert_eq!(scaled.size(), (4, 2));
        assert_eq!(scaled.indices(), &[1, 1, 5, 5, 1, 1, 5, 5]);
    }

    #[test]
    fn scaling_by_one_changes_nothing() {
        let indices = GrayImage::from_raw(2, 1, vec![1, 5]).unwrap();
        let image = IndexedImage::new(indices, Palette::new(0.6));

        assert_eq!(image.scale_nearest(1).indices(), image.indices());
    }

    #[test]
    fn to_rgb_expands_every_slot_through_the_palette() {
        let palette = Palette::new(0.6);
        let indices = GrayImage::from_raw(3, 1, vec![0, 1, 2]).unwrap();
        let rgb = IndexedImage::new(indices, palette.clone()).to_rgb();
        for (x, slot) in [0usize, 1, 2].into_iter().enumerate() {
            assert_eq!(rgb.get_pixel(x as u32, 0).0, palette.colors()[slot]);
        }
    }
}
