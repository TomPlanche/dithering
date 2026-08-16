//! The fixed palette everything is reduced to.
//!
//! Six colours, each blended between a pure primary and a muted version of it. The blend is what `saturation` picks,
//! and the nearest-colour search over the result is what the dithering stage quantises through.

/// Number of colours in the palette.
pub const PALETTE_COLORS: usize = 6;

/// The pure primaries, fully saturated.
pub const PURE_PALETTE: [[u8; 3]; PALETTE_COLORS] = [
    [0, 0, 0],       // Black
    [255, 255, 255], // White
    [0, 255, 0],     // Green
    [0, 0, 255],     // Blue
    [255, 0, 0],     // Red
    [255, 255, 0],   // Yellow
];

/// The muted counterparts, which is what those primaries look like on ink.
pub const MUTED_PALETTE: [[u8; 3]; PALETTE_COLORS] = [
    [57, 48, 57],    // Muted Black
    [255, 255, 255], // White
    [40, 91, 58],    // Muted Green
    [0, 128, 255],   // Muted Blue
    [156, 72, 75],   // Muted Red
    [208, 190, 71],  // Muted Yellow
];

/// Maps each output palette slot to its source primary.
///
/// The slots are ordered black, white, yellow, red, blue, green rather than following the primaries' own order, so
/// slot 0 and slot 1 are the two extremes a nearest-colour search falls back on.
pub const PALETTE_ORDER: [usize; PALETTE_COLORS] = [0, 1, 5, 4, 3, 2];

/// Blends the pure and the muted palettes.
///
/// At `0.0` the result is [`PURE_PALETTE`], at `1.0` it is [`MUTED_PALETTE`]. Anything outside that range is clamped
/// into it, since there is no colour past either end.
pub fn palette_blend(saturation: f64) -> [[u8; 3]; PALETTE_COLORS] {
    let saturation = saturation.clamp(0.0, 1.0);
    let mut out = [[0u8; 3]; PALETTE_COLORS];

    for (slot, &src) in PALETTE_ORDER.iter().enumerate() {
        for channel in 0..3 {
            let muted = MUTED_PALETTE[src][channel] as f64 * saturation;
            let pure = PURE_PALETTE[src][channel] as f64 * (1.0 - saturation);
            out[slot][channel] = (muted + pure) as u8;
        }
    }

    out
}

/// The blended palette, with the nearest-colour search over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    colors: [[u8; 3]; PALETTE_COLORS],
}

impl Palette {
    /// Builds the palette for a given saturation.
    pub fn new(saturation: f64) -> Self {
        Self {
            colors: palette_blend(saturation),
        }
    }

    /// The blended colours, one per slot.
    pub fn colors(&self) -> &[[u8; 3]; PALETTE_COLORS] {
        &self.colors
    }

    /// The palette flattened for a PNG `PLTE` chunk, padded to 256 entries.
    pub fn plte(&self) -> Vec<u8> {
        let mut flat: Vec<u8> = self.colors.iter().flatten().copied().collect();
        flat.resize(256 * 3, 0);

        flat
    }

    /// The slot closest to an arbitrary colour, by squared RGB distance. Ties go to the lower slot.
    ///
    /// Error diffusion wants this rather than a perceptual metric. CIELAB weights lightness heavily, so on a palette
    /// this small it pulls midtones toward white and black and visibly drains the colour out of the result. Diffusing
    /// the error afterwards is what recovers the intermediate tones, and that works best when the match minimises the
    /// error the diffusion has to carry.
    pub fn nearest(&self, rgb: [u8; 3]) -> usize {
        let mut best = 0;
        let mut best_dist = i32::MAX;

        for (slot, entry) in self.colors.iter().enumerate() {
            let dist: i32 = (0..3)
                .map(|c| {
                    let d = rgb[c] as i32 - entry[c] as i32;
                    d * d
                })
                .sum();

            if dist < best_dist {
                best_dist = dist;
                best = slot;
            }
        }

        best
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new(0.6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blend_lands_between_the_two_palettes() {
        assert_eq!(
            palette_blend(0.6),
            [
                [34, 28, 34],
                [255, 255, 255],
                [226, 216, 42],
                [195, 43, 45],
                [0, 76, 255],
                [24, 156, 34],
            ]
        );
    }

    #[test]
    fn the_ends_of_the_blend_are_the_palettes_themselves() {
        for (slot, &src) in PALETTE_ORDER.iter().enumerate() {
            assert_eq!(palette_blend(0.0)[slot], PURE_PALETTE[src]);
            assert_eq!(palette_blend(1.0)[slot], MUTED_PALETTE[src]);
        }
    }

    #[test]
    fn saturation_outside_the_range_is_clamped() {
        assert_eq!(palette_blend(-2.0), palette_blend(0.0));
        assert_eq!(palette_blend(7.5), palette_blend(1.0));
    }

    #[test]
    fn every_slot_is_a_distinct_colour() {
        let colors = palette_blend(0.6);

        for (slot, color) in colors.iter().enumerate() {
            assert!(
                !colors[..slot].contains(color),
                "slot {slot} repeats an earlier colour, so it can never be picked"
            );
        }
    }

    #[test]
    fn primaries_map_to_their_own_slots() {
        let palette = Palette::new(0.6);

        assert_eq!(palette.nearest([0, 0, 0]), 0);
        assert_eq!(palette.nearest([255, 255, 255]), 1);
        assert_eq!(palette.nearest([255, 255, 0]), 2);
        assert_eq!(palette.nearest([255, 0, 0]), 3);
        assert_eq!(palette.nearest([0, 0, 255]), 4);
        assert_eq!(palette.nearest([0, 255, 0]), 5);
    }

    #[test]
    fn plte_is_padded_to_256_entries() {
        let palette = Palette::new(0.6);
        let plte = palette.plte();

        assert_eq!(plte.len(), 768);
        assert_eq!(&plte[..3], &palette.colors()[0]);
        assert!(plte[PALETTE_COLORS * 3..].iter().all(|&b| b == 0));
    }

    #[test]
    fn nearest_always_lands_on_a_real_slot() {
        let palette = Palette::new(0.6);

        for value in 0..=255u8 {
            assert!(palette.nearest([value, value, value]) < PALETTE_COLORS);
        }
    }
}
