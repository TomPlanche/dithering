//! Dithers a photo to a fixed six-colour palette.
//!
//! Two stages, in this order:
//!
//! 1. [`resize`] takes the photo to the size it gets dithered at, reframing it on the way if asked.
//! 2. [`dither::apply_dithering`] reduces it to the palette with Floyd-Steinberg error diffusion.
//!
//! Both ends are `image` types: an [`RgbImage`] goes in, an [`IndexedImage`] comes out. Decoding and encoding are the
//! caller's.

pub mod adjust;
pub mod diffusion;
pub mod dither;
pub mod indexed;
pub mod palette;
pub mod resize;

#[cfg(feature = "image-io")]
pub mod io;

pub use diffusion::{FLOYD_STEINBERG, diffuse};
pub use dither::{DitherOptions, apply_dithering};
pub use indexed::IndexedImage;
pub use palette::Palette;
pub use resize::{
    CropOrigin, DEFAULT_SIZE, FitOptions, MAX_CROP_ZOOM, RATIO_PRESETS, cover_rect, crop_to_fit, fitted_rect,
    fitted_size, orient_target, preset_names, preset_ratio, ratio_size, resize_cropped, resize_image, resize_to_fit,
    scale_to_fit,
};

/// Re-exported so callers can build inputs without depending on `image` directly.
pub use image::RgbImage;
