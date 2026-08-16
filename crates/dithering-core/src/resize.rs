//! Resizing a photo to the size it gets dithered at.

use std::fmt;
use std::str::FromStr;

use image::RgbImage;
use image::imageops::{self, FilterType};
use rayon::prelude::*;

/// The landscape size the pipeline dithers at unless told otherwise.
pub const DEFAULT_SIZE: (u32, u32) = (600, 400);

/// As far into a photo as a crop will go.
///
/// Past this the kept rectangle is a stamp being blown up to the working size, which the resize can only blur.
pub const MAX_CROP_ZOOM: f32 = 10.0;

/// Aspect ratios that go by a name, for a caller that would rather not work the shape out itself.
///
/// A preset names a shape, never a pixel count. [`ratio_size`] fits it inside whatever working size was asked for, so
/// how much to dither stays the caller's to pick and a preset only ever reframes it.
///
/// A preset names one orientation. The other one is [`FitOptions::keep_orientation`], which transposes whichever of
/// these is asked for: `iphone` on a portrait photo dithers at 3:4.
pub const RATIO_PRESETS: [(&str, (u32, u32)); 5] = [
    ("instagram-post", (1, 1)),
    ("instagram-portrait", (4, 5)),
    // 1.91:1, which the feed states in decimals rather than whole sides.
    ("instagram-landscape", (191, 100)),
    ("instagram-story", (9, 16)),
    // The iPhone's default 4:3 photo.
    ("iphone", (4, 3)),
];

/// The aspect ratio a preset names, or `None` when nothing goes by that name.
pub fn preset_ratio(name: &str) -> Option<(u32, u32)> {
    RATIO_PRESETS
        .iter()
        .find(|(preset, _)| *preset == name)
        .map(|(_, ratio)| *ratio)
}

/// The preset names, in the order [`RATIO_PRESETS`] lists them.
pub const fn preset_names() -> [&'static str; RATIO_PRESETS.len()] {
    let mut names = [""; RATIO_PRESETS.len()];
    let mut i = 0;

    while i < RATIO_PRESETS.len() {
        names[i] = RATIO_PRESETS[i].0;
        i += 1;
    }

    names
}

/// The largest `ratio`-shaped size that fits inside `bounds`.
///
/// This is what a preset resolves to: the ratio picks the shape and `bounds` picks the scale, so the working size a
/// caller asked for still says how many pixels get dithered.
///
/// `bounds` is turned over first when the ratio disagrees with it, which is what keeps a portrait ratio against a
/// landscape working size from being squeezed into its short side: `instagram-story` inside 600x400 is 337x600, not
/// 225x400. A caller that wants the photo's orientation rather than the ratio's has [`FitOptions::keep_orientation`],
/// which transposes the result the same way.
///
/// A zero side leaves nothing to fit, so `bounds` comes back unchanged.
pub fn ratio_size(bounds: (u32, u32), ratio: (u32, u32)) -> (u32, u32) {
    if bounds.0 == 0 || bounds.1 == 0 || ratio.0 == 0 || ratio.1 == 0 {
        return bounds;
    }

    ratio_fit(orient_target(ratio, bounds), ratio)
}

/// How a photo that does not share the working size's shape is made to fit it.
///
/// The flags are off by default, which is the plainest thing to do: the photo is stretched into the working size
/// whatever shape it arrived in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitOptions {
    /// Transpose the working size for a photo of the other orientation, so a portrait photo stays portrait.
    pub keep_orientation: bool,
    /// Crop to the working size's aspect ratio rather than stretching the photo into it.
    pub crop: bool,
    /// Which part of the photo the crop keeps. Ignored unless `crop`.
    pub crop_from: CropOrigin,
    /// How far into the photo the crop moves, 1.0 being as much of it as the ratio allows. Ignored unless `crop`.
    ///
    /// At 1.0 the kept rectangle touches two opposite edges, so it can only slide along one axis. Anything above that
    /// keeps a proportionally smaller rectangle, which frees the other axis too: 2.0 keeps half the width and half the
    /// height, so [`CropOrigin::At`] can then reach anywhere in the photo.
    pub crop_zoom: f32,
}

impl Default for FitOptions {
    fn default() -> Self {
        Self {
            keep_orientation: false,
            crop: false,
            crop_from: CropOrigin::Center,
            crop_zoom: 1.0,
        }
    }
}

/// Which part of a photo a crop keeps.
///
/// An anchor takes the largest rectangle the target's ratio allows and puts it against a side. Such a rectangle spans
/// the photo's full width or its full height, never neither, so an anchor can only move it along the axis that has
/// slack: `Top` on a photo that is losing its sides behaves like `Center`.
///
/// [`CropOrigin::At`] works the other way round, and is the one to reach for when a coordinate has to mean what it
/// says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CropOrigin {
    /// The middle, which is what a photographer usually framed for.
    #[default]
    Center,
    Top,
    Bottom,
    Left,
    Right,
    /// Where the crop starts, in source pixels, `0,0` being the photo's top-left.
    ///
    /// The corner is kept as given and the rectangle grows from it, so `0,200` always drops the top 200 rows. What it
    /// costs is size: the rectangle is only as large as what is left below and to the right of the corner, so a corner
    /// far into the photo leaves a small one to blow back up.
    At {
        x: u32,
        y: u32,
    },
}

impl fmt::Display for CropOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CropOrigin::Center => f.write_str("center"),
            CropOrigin::Top => f.write_str("top"),
            CropOrigin::Bottom => f.write_str("bottom"),
            CropOrigin::Left => f.write_str("left"),
            CropOrigin::Right => f.write_str("right"),
            CropOrigin::At { x, y } => write!(f, "{x},{y}"),
        }
    }
}

impl FromStr for CropOrigin {
    type Err = String;

    /// Parses an anchor name, or `X,Y` for a corner. [`fmt::Display`] writes back what this reads.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim() {
            "center" => Ok(CropOrigin::Center),
            "top" => Ok(CropOrigin::Top),
            "bottom" => Ok(CropOrigin::Bottom),
            "left" => Ok(CropOrigin::Left),
            "right" => Ok(CropOrigin::Right),
            corner => {
                let (x, y) = corner
                    .split_once(',')
                    .ok_or_else(|| format!("expected center, top, bottom, left, right or X,Y, got `{raw}`"))?;

                Ok(CropOrigin::At {
                    x: x.trim().parse().map_err(|_| format!("bad crop x `{x}`"))?,
                    y: y.trim().parse().map_err(|_| format!("bad crop y `{y}`"))?,
                })
            },
        }
    }
}

/// Scales a photo to the working size, fitted the way `fit` asks for.
///
/// Turning both flags on is what keeps a photo of any shape undistorted: the target follows the photo's orientation,
/// and whatever ratio is left over comes off the long side as a crop.
pub fn resize_to_fit(image: &RgbImage, target: (u32, u32), fit: FitOptions) -> RgbImage {
    let source = image.dimensions();

    resize_region(
        image,
        fitted_rect(source, target, fit),
        fitted_size(source, target, fit),
    )
}

/// The part of a photo [`resize_to_fit`] would read, kept at its own resolution.
///
/// The crop without the scale, for a caller that asked to keep the source pixels. `target` is then read for its aspect
/// ratio alone, since nothing is being fitted to its size: a 1536x2048 photo cropped to 9:16 from `0,200` comes out
/// 1039x1848, not 1080x1920.
pub fn crop_to_fit(image: &RgbImage, target: (u32, u32), fit: FitOptions) -> RgbImage {
    scale_to_fit(image, target, fit, 1.0)
}

/// The part of a photo [`resize_to_fit`] would read, scaled by `factor`.
///
/// Between [`resize_to_fit`], which lands on a size, and [`crop_to_fit`], which lands on none: this one keeps the
/// photo's own proportions and asks only how much smaller, so 0.75 takes three quarters of each side of whatever the
/// crop kept. `target` is therefore read for its shape alone, the way [`crop_to_fit`] reads it.
///
/// A factor above 1.0 enlarges, which the triangle filter can only interpolate. A factor that is not a positive finite
/// number is treated as 1.0. The result is never empty: each side rounds to at least one pixel.
pub fn scale_to_fit(image: &RgbImage, target: (u32, u32), fit: FitOptions, factor: f64) -> RgbImage {
    let rect = fitted_rect(image.dimensions(), target, fit);
    let factor = if factor.is_finite() && factor > 0.0 {
        factor
    } else {
        1.0
    };

    let scaled = |side: u32| ((f64::from(side) * factor).round() as u32).max(1);

    resize_region(image, rect, (scaled(rect.2), scaled(rect.3)))
}

/// The size [`resize_to_fit`] would produce for a photo of `source` dimensions.
///
/// That is `target` itself, unless `keep_orientation` turns it over to follow the photo.
pub fn fitted_size(source: (u32, u32), target: (u32, u32), fit: FitOptions) -> (u32, u32) {
    if fit.keep_orientation {
        orient_target(source, target)
    } else {
        target
    }
}

/// The region of the photo [`resize_to_fit`] reads, as `(x, y, width, height)`.
///
/// The whole photo unless `crop`, which is what a caller reports when it wants to say which part of an upload was
/// used, and why a coordinate it asked for did not move anything.
pub fn fitted_rect(source: (u32, u32), target: (u32, u32), fit: FitOptions) -> (u32, u32, u32, u32) {
    if fit.crop {
        cover_rect(source, fitted_size(source, target, fit), fit.crop_from, fit.crop_zoom)
    } else {
        (0, 0, source.0, source.1)
    }
}

/// Scales a photo to the working size, stretching it when the two shapes disagree.
///
/// A triangle filter widens its kernel when downscaling, so it behaves like an area average rather than point sampling.
/// Run alone on a 45 MP photo it costs more than the rest of the pipeline put together, because the kernel then spans a
/// hundred source pixels per output pixel.
///
/// So the descent happens in two steps. An integer box average takes the photo to within one whole factor of the
/// target, and the triangle filter covers the fractional remainder. Both passes area-average, which is why the result
/// stays within about one 8-bit level of the single-pass version.
pub fn resize_image(image: &RgbImage, target: (u32, u32)) -> RgbImage {
    resize_region(image, (0, 0, image.width(), image.height()), target)
}

/// Scales the part of a photo that already has the target's aspect ratio, taken from `origin` at `zoom`.
///
/// Nothing is distorted, and what it costs instead is the edges: a 3:2 photo against a 2:3 target keeps the middle
/// third or so of its width. The crop is never materialised, so this reads the source once, the same as a plain resize.
pub fn resize_cropped(image: &RgbImage, target: (u32, u32), origin: CropOrigin, zoom: f32) -> RgbImage {
    resize_region(image, cover_rect(image.dimensions(), target, origin, zoom), target)
}

/// `target`, transposed when it and `source` disagree on orientation.
///
/// A square source counts as landscape, so a square photo against a landscape target is left alone.
pub fn orient_target(source: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    if (source.1 > source.0) == (target.1 > target.0) {
        target
    } else {
        (target.1, target.0)
    }
}

/// A rectangle of `source` with `target`'s aspect ratio, placed at `origin`, as `(x, y, width, height)`.
///
/// The two origins work from opposite ends. An anchor asks for the largest rectangle there is and then puts it against
/// a side, so it can only slide along whichever axis has slack. A corner is the other way round: the corner is what was
/// asked for, so it is kept, and the rectangle is the largest that fits in what the photo still offers below and to the
/// right of it. That is what makes `0,200` drop the top 200 rows on any photo, rather than being ignored on one whose
/// rectangle already spans the full height.
///
/// A corner near the far edge therefore leaves very little to keep, which the resize can only blur back up to the
/// working size. [`fitted_rect`] reports what was kept, so a caller can see it coming.
///
/// `zoom` shrinks whatever the origin settled on, staying at 1.0 for all of it. Below 1.0 there is nothing more to
/// keep, so it is treated as 1.0.
pub fn cover_rect(source: (u32, u32), target: (u32, u32), origin: CropOrigin, zoom: f32) -> (u32, u32, u32, u32) {
    let (sw, sh) = source;
    if target.0 == 0 || target.1 == 0 || sw == 0 || sh == 0 {
        return (0, 0, sw, sh);
    }

    // Both sides shrink by the same factor, so the ratio holds to within the rounding.
    let zoom = if zoom.is_finite() { zoom.max(1.0) } else { 1.0 };
    let shrink = |(width, height): (u32, u32)| {
        if zoom > 1.0 {
            (
                ((width as f32 / zoom).round() as u32).clamp(1, width),
                ((height as f32 / zoom).round() as u32).clamp(1, height),
            )
        } else {
            (width, height)
        }
    };

    if let CropOrigin::At { x, y } = origin {
        // The corner cannot be past the last pixel, since a rectangle has to have something to cover.
        let (x, y) = (x.min(sw - 1), y.min(sh - 1));
        let (width, height) = shrink(ratio_fit((sw - x, sh - y), target));

        return (x, y, width, height);
    }

    let (width, height) = shrink(ratio_fit(source, target));

    // What the rectangle leaves over. At a zoom of 1.0 that is only ever one axis.
    let (free_x, free_y) = (sw - width, sh - height);
    let (x, y) = match origin {
        CropOrigin::Top => (free_x / 2, 0),
        CropOrigin::Bottom => (free_x / 2, free_y),
        CropOrigin::Left => (0, free_y / 2),
        CropOrigin::Right => (free_x, free_y / 2),
        // `At` returned above, so this is `Center`.
        _ => (free_x / 2, free_y / 2),
    };

    (x, y, width, height)
}

/// The largest `target`-shaped rectangle that fits inside `space`.
///
/// The comparison is `sw * th` against `tw * sh` rather than a pair of divisions, so the ratios are exact and a space
/// that already matches the target keeps every pixel of it. Rounding down keeps the rectangle inside.
fn ratio_fit(space: (u32, u32), target: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = space;
    let (sw64, sh64) = (u64::from(sw), u64::from(sh));
    let (tw64, th64) = (u64::from(target.0), u64::from(target.1));

    if sw64 * th64 > tw64 * sh64 {
        // The space is the wider of the two, so the sides come off.
        (((sh64 * tw64 / th64) as u32).clamp(1, sw), sh)
    } else {
        (sw, ((sw64 * th64 / tw64) as u32).clamp(1, sh))
    }
}

/// Scales the `(x, y, width, height)` region of a photo to `target`.
fn resize_region(image: &RgbImage, rect: (u32, u32, u32, u32), target: (u32, u32)) -> RgbImage {
    let (x, y, width, height) = rect;
    if (width, height) == target {
        // Nothing to scale, so the region is copied out as it is, which for the whole photo is a plain clone.
        return imageops::crop_imm(image, x, y, width, height).to_image();
    }

    match prefilter_factor((width, height), target) {
        // A crop view costs an offset per pixel, and this arm only ever runs on a photo under twice the target.
        1 => imageops::resize(
            &*imageops::crop_imm(image, x, y, width, height),
            target.0,
            target.1,
            FilterType::Triangle,
        ),
        factor => {
            let reduced = box_reduce(image, rect, factor);
            imageops::resize(&reduced, target.0, target.1, FilterType::Triangle)
        },
    }
}

/// The largest integer reduction that still leaves both sides at or above the target, so the triangle filter that
/// follows only ever downscales.
fn prefilter_factor(source: (u32, u32), target: (u32, u32)) -> u32 {
    let (w, h) = source;
    let (tw, th) = target;

    if tw == 0 || th == 0 {
        return 1;
    }

    (w / tw).min(h / th).max(1)
}

/// Averages whole `factor` x `factor` blocks of the `(x, y, width, height)` region.
///
/// The last few source rows and columns are dropped when the side is not a multiple of `factor`. That is at most
/// `factor - 1` pixels off an edge that is thousands wide, and it keeps every block the same weight.
///
/// The region is read in place. A crop is only ever an offset and a shorter row here, so it never costs the copy that
/// materialising the sub-image would.
///
/// Output rows are independent and each reads a disjoint band of the source, so they run in parallel. This is the
/// stage that reads every source pixel, which is why it is the one worth spreading.
fn box_reduce(image: &RgbImage, rect: (u32, u32, u32, u32), factor: u32) -> RgbImage {
    let (x0, y0, width, height) = rect;
    let f = factor as usize;
    let out_w = (width as usize) / f;
    let out_h = (height as usize) / f;
    let src_stride = (image.width() as usize) * 3;
    let origin = (y0 as usize) * src_stride + (x0 as usize) * 3;
    let src = image.as_raw();

    // Round to nearest rather than truncating, which halves the drift.
    let count = (f * f) as u32;
    let half = count / 2;

    let mut out = vec![0u8; out_w * out_h * 3];
    out.par_chunks_mut(out_w * 3).enumerate().for_each(|(y, row)| {
        let block_top = origin + y * f * src_stride;

        for (x, cell) in row.chunks_exact_mut(3).enumerate() {
            let block_left = x * f * 3;
            let mut acc = [0u32; 3];

            for by in 0..f {
                let start = block_top + by * src_stride + block_left;

                for px in src[start..start + f * 3].chunks_exact(3) {
                    acc[0] += px[0] as u32;
                    acc[1] += px[1] as u32;
                    acc[2] += px[2] as u32;
                }
            }

            cell[0] = ((acc[0] + half) / count) as u8;
            cell[1] = ((acc[1] + half) / count) as u8;
            cell[2] = ((acc[2] + half) / count) as u8;
        }
    });

    RgbImage::from_raw(out_w as u32, out_h as u32, out).expect("buffer matches the dimensions")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> RgbImage {
        RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        })
    }

    #[test]
    fn resize_to_the_same_size_is_a_no_op() {
        let image = gradient(3, 2);

        assert_eq!(resize_image(&image, (3, 2)), image);
    }

    #[test]
    fn downscaling_averages_rather_than_point_samples() {
        // A black and white checkerboard must reduce to mid grey, not to one colour.
        let image = RgbImage::from_fn(64, 64, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 255 };
            image::Rgb([v, v, v])
        });
        let small = resize_image(&image, (8, 8));

        assert!(
            small.pixels().all(|p| (100..=155).contains(&p.0[0])),
            "expected mid grey, got {:?}",
            small.get_pixel(4, 4)
        );
    }

    #[test]
    fn the_two_stage_descent_matches_a_single_triangle_pass() {
        let image = gradient(400, 400);
        let staged = resize_image(&image, (40, 40));
        let direct = imageops::resize(&image, 40, 40, FilterType::Triangle);

        for (a, b) in staged.as_raw().iter().zip(direct.as_raw()) {
            assert!(a.abs_diff(*b) <= 2, "{a} against {b}");
        }
    }

    #[test]
    fn presets_are_all_reachable_by_name() {
        for name in preset_names() {
            assert!(preset_ratio(name).is_some(), "{name} is listed but does not resolve");
        }

        assert_eq!(preset_ratio("nothing-by-that-name"), None);
    }

    #[test]
    fn a_preset_keeps_its_own_orientation_inside_the_bounds() {
        // A portrait ratio against a landscape working size turns the bounds over rather than using the short side.
        assert_eq!(ratio_size((600, 400), (9, 16)), (337, 600));
        assert_eq!(ratio_size((600, 400), (1, 1)), (400, 400));
        assert_eq!(ratio_size((600, 400), (191, 100)), (600, 314));
    }

    #[test]
    fn a_zero_side_leaves_the_bounds_alone() {
        assert_eq!(ratio_size((0, 400), (1, 1)), (0, 400));
        assert_eq!(ratio_size((600, 400), (0, 1)), (600, 400));
    }

    #[test]
    fn orientation_follows_the_source() {
        assert_eq!(orient_target((400, 600), (600, 400)), (400, 600));
        assert_eq!(orient_target((600, 400), (600, 400)), (600, 400));
        // A square source counts as landscape.
        assert_eq!(orient_target((500, 500), (400, 600)), (600, 400));
    }

    #[test]
    fn keep_orientation_transposes_the_working_size() {
        let fit = FitOptions {
            keep_orientation: true,
            ..Default::default()
        };

        assert_eq!(fitted_size((1000, 2000), DEFAULT_SIZE, fit), (400, 600));
        assert_eq!(fitted_size((2000, 1000), DEFAULT_SIZE, fit), (600, 400));
        assert_eq!(
            fitted_size((1000, 2000), DEFAULT_SIZE, FitOptions::default()),
            (600, 400)
        );
    }

    #[test]
    fn a_portrait_photo_stays_portrait() {
        let fit = FitOptions {
            keep_orientation: true,
            ..Default::default()
        };

        assert_eq!(
            resize_to_fit(&gradient(300, 600), DEFAULT_SIZE, fit).dimensions(),
            (400, 600)
        );
    }

    #[test]
    fn without_crop_the_whole_photo_is_read() {
        assert_eq!(
            fitted_rect((1000, 800), (600, 400), FitOptions::default()),
            (0, 0, 1000, 800)
        );
    }

    #[test]
    fn a_centred_crop_takes_the_middle() {
        // 1000x1000 against 3:2 keeps a full-width band, centred vertically.
        assert_eq!(
            cover_rect((1000, 1000), (600, 400), CropOrigin::Center, 1.0),
            (0, 167, 1000, 666)
        );
    }

    #[test]
    fn an_anchor_only_moves_along_the_axis_with_slack() {
        let source = (1000, 1000);
        let target = (600, 400);

        assert_eq!(cover_rect(source, target, CropOrigin::Top, 1.0), (0, 0, 1000, 666));
        assert_eq!(cover_rect(source, target, CropOrigin::Bottom, 1.0), (0, 334, 1000, 666));
        // The rectangle already spans the full width, so a horizontal anchor behaves like centre.
        assert_eq!(
            cover_rect(source, target, CropOrigin::Left, 1.0),
            cover_rect(source, target, CropOrigin::Center, 1.0)
        );
    }

    #[test]
    fn a_corner_is_kept_as_given() {
        let rect = cover_rect((1536, 2048), (9, 16), CropOrigin::At { x: 0, y: 200 }, 1.0);

        assert_eq!(rect, (0, 200, 1039, 1848));
    }

    #[test]
    fn a_corner_past_the_last_pixel_is_pulled_back_in() {
        let (x, y, width, height) = cover_rect((100, 100), (1, 1), CropOrigin::At { x: 500, y: 500 }, 1.0);

        assert_eq!((x, y), (99, 99));
        assert_eq!((width, height), (1, 1));
    }

    #[test]
    fn zoom_shrinks_the_rectangle_and_frees_the_other_axis() {
        let (x, y, width, height) = cover_rect((1000, 1000), (600, 400), CropOrigin::Center, 2.0);

        assert_eq!((width, height), (500, 333));
        assert_eq!((x, y), (250, 333));
    }

    #[test]
    fn a_zoom_below_one_keeps_everything_the_ratio_allows() {
        let full = cover_rect((1000, 1000), (600, 400), CropOrigin::Center, 1.0);

        assert_eq!(cover_rect((1000, 1000), (600, 400), CropOrigin::Center, 0.25), full);
        assert_eq!(cover_rect((1000, 1000), (600, 400), CropOrigin::Center, f32::NAN), full);
    }

    #[test]
    fn cropping_keeps_the_photos_proportions() {
        // A 3:2 photo cropped to 2:3 must not be squeezed: the sides come off instead.
        let fit = FitOptions {
            crop: true,
            ..Default::default()
        };
        let out = resize_to_fit(&gradient(900, 600), (400, 600), fit);

        assert_eq!(out.dimensions(), (400, 600));
        assert_eq!(fitted_rect((900, 600), (400, 600), fit), (250, 0, 400, 600));
    }

    #[test]
    fn both_flags_together_reframe_without_distorting() {
        let fit = FitOptions {
            keep_orientation: true,
            crop: true,
            ..Default::default()
        };
        let source = (2000, 3000);

        assert_eq!(fitted_size(source, DEFAULT_SIZE, fit), (400, 600));
        // 2:3 into 2:3, so the crop takes everything.
        assert_eq!(fitted_rect(source, DEFAULT_SIZE, fit), (0, 0, 2000, 3000));
    }

    #[test]
    fn crop_to_fit_keeps_the_source_pixels() {
        let fit = FitOptions {
            crop: true,
            ..Default::default()
        };

        // The target is read for its shape alone, so the result is the kept rectangle at its own resolution.
        assert_eq!(
            crop_to_fit(&gradient(1536, 2048), (1080, 1920), fit).dimensions(),
            (1152, 2048)
        );
        assert_eq!(
            crop_to_fit(&gradient(900, 600), (400, 600), fit).dimensions(),
            (400, 600)
        );
    }

    #[test]
    fn scale_to_fit_takes_a_fraction_of_what_the_crop_kept() {
        let fit = FitOptions {
            crop: true,
            ..Default::default()
        };
        let out = scale_to_fit(&gradient(1000, 1000), (600, 400), fit, 0.5);

        assert_eq!(out.dimensions(), (500, 333));
    }

    #[test]
    fn a_scale_factor_that_makes_no_sense_is_treated_as_one() {
        let fit = FitOptions::default();
        let image = gradient(40, 30);

        for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(scale_to_fit(&image, (4, 3), fit, factor).dimensions(), (40, 30));
        }

        // Nothing ever scales away to nothing.
        assert_eq!(scale_to_fit(&image, (4, 3), fit, 0.001).dimensions(), (1, 1));
    }

    #[test]
    fn resize_cropped_lands_on_the_target_size() {
        let out = resize_cropped(&gradient(1200, 800), (400, 400), CropOrigin::Right, 1.0);

        assert_eq!(out.dimensions(), (400, 400));
    }

    #[test]
    fn crop_origins_round_trip_through_text() {
        for origin in [
            CropOrigin::Center,
            CropOrigin::Top,
            CropOrigin::Bottom,
            CropOrigin::Left,
            CropOrigin::Right,
            CropOrigin::At { x: 12, y: 340 },
        ] {
            assert_eq!(origin.to_string().parse(), Ok(origin));
        }
        assert!("sideways".parse::<CropOrigin>().is_err());
        assert!("12,".parse::<CropOrigin>().is_err());
    }
}
