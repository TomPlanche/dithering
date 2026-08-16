//! Query parameters accepted by the dithering endpoints.
//!
//! Every field has a default, so `POST /api/dither` with no query string runs the pipeline's own defaults. The names
//! here are also what `GET /api/options` reports, so a client can round-trip its defaults straight back into a query
//! string.

use axum::extract::{FromRequestParts, Query};
use axum::http::request::Parts;
use dithering_core::{CropOrigin, DitherOptions, FitOptions, MAX_CROP_ZOOM};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

/// Largest working width or height a request may ask for.
pub const MAX_DIMENSION: u32 = 4096;
/// Largest nearest-neighbour upscale factor.
pub const MAX_SCALE: u32 = 4;
/// Largest `brightness` or `color` boost. Past this the photo is a flat block of primaries.
pub const MAX_BOOST: f64 = 5.0;
/// Largest source image accepted, as a guard against decompression bombs.
pub const MAX_SOURCE_PIXELS: u64 = 50_000_000;

/// An aspect ratio that goes by name.
///
/// The variants mirror [`dithering_core::RATIO_PRESETS`], which is where the ratios come from. Serde does the
/// validating: an unknown name is refused with the list of the ones that work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    InstagramPost,
    InstagramPortrait,
    InstagramLandscape,
    InstagramStory,
    Iphone,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::InstagramPost,
        Preset::InstagramPortrait,
        Preset::InstagramLandscape,
        Preset::InstagramStory,
        Preset::Iphone,
    ];

    /// The name this goes by, in both the query string and the pipeline's own table.
    pub fn name(self) -> &'static str {
        match self {
            Preset::InstagramPost => "instagram-post",
            Preset::InstagramPortrait => "instagram-portrait",
            Preset::InstagramLandscape => "instagram-landscape",
            Preset::InstagramStory => "instagram-story",
            Preset::Iphone => "iphone",
        }
    }

    /// The aspect ratio it names, as `width:height`.
    pub fn ratio(self) -> (u32, u32) {
        dithering_core::preset_ratio(self.name()).expect("every preset names a ratio the pipeline knows")
    }
}

/// What `resize` asks for.
///
/// Three answers to one question, which is how much smaller the photo should come back: the working size, nothing at
/// all, or a fraction of what the framing kept. Whichever it is, `crop` still decides the shape, so `resize` governs
/// the scaling alone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Resize {
    /// `true`: scale to `width`x`height`, reshaped by any `preset`.
    Fit,
    /// `false`: keep the source resolution.
    Keep,
    /// `0.75`: three quarters of each side of what the framing kept, so a quarter off the photo.
    Factor(f64),
}

impl Resize {
    /// The fraction it asks for, or `None` when it names a size instead.
    pub fn factor(self) -> Option<f64> {
        match self {
            Resize::Factor(factor) => Some(factor),
            _ => None,
        }
    }
}

impl Serialize for Resize {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Resize::Fit => serializer.serialize_bool(true),
            Resize::Keep => serializer.serialize_bool(false),
            Resize::Factor(factor) => serializer.serialize_f64(*factor),
        }
    }
}

impl<'de> Deserialize<'de> for Resize {
    /// Reads `true`, `false` or a number, and the same three spelled as the strings a query string carries.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AnyResize;

        impl serde::de::Visitor<'_> for AnyResize {
            type Value = Resize;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("true, false, or a fraction between 0 and 1")
            }

            fn visit_bool<E>(self, yes: bool) -> Result<Resize, E> {
                Ok(if yes { Resize::Fit } else { Resize::Keep })
            }

            fn visit_f64<E>(self, factor: f64) -> Result<Resize, E> {
                Ok(Resize::Factor(factor))
            }

            fn visit_u64<E>(self, factor: u64) -> Result<Resize, E> {
                Ok(Resize::Factor(factor as f64))
            }

            fn visit_i64<E>(self, factor: i64) -> Result<Resize, E> {
                Ok(Resize::Factor(factor as f64))
            }

            fn visit_str<E: serde::de::Error>(self, raw: &str) -> Result<Resize, E> {
                match raw.trim() {
                    "true" => Ok(Resize::Fit),
                    "false" => Ok(Resize::Keep),
                    number => number
                        .parse()
                        .map(Resize::Factor)
                        .map_err(|_| E::custom(format!("resize must be true, false or a number, got `{raw}`"))),
                }
            }
        }

        deserializer.deserialize_any(AnyResize)
    }
}

/// How to encode the PNG that comes back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// Indexed PNG carrying the palette. Smaller, and the default.
    Indexed,
    /// Plain RGB PNG, for viewers that dislike palette images.
    Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DitherParams {
    /// Blend between the pure and the muted palettes, 0.0 to 1.0.
    ///
    /// This names the six colours the photo lands on. It does not touch the photo, which is `color`.
    pub saturation: f64,
    /// Gain applied to the photo before dithering, 0.0 to 5.0. `1.0` leaves it alone.
    pub brightness: f64,
    /// How far the photo's pixels are pushed away from grey before dithering, 0.0 to 5.0.
    ///
    /// `1.0` leaves the photo alone and `0.0` leaves it grey. Above 1.0 the channels are pushed apart, which is what
    /// gives a palette of six colours something to work with.
    pub color: f64,
    /// Working width, used unless `resize` is false. A `preset` reshapes it rather than replacing it.
    pub width: u32,
    /// Working height, used unless `resize` is false. A `preset` reshapes it rather than replacing it.
    pub height: u32,
    /// A named aspect ratio, fitted inside `width`x`height`.
    ///
    /// It picks the shape and the pair still picks the scale, so a request says how many pixels it wants dithered
    /// whichever preset it names.
    ///
    /// Left out of `GET /api/options` when unset, where the `presets` list says what the names are instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<Preset>,
    /// What to scale the photo to: `true` for the working size, `false` for none, or a fraction of its own size.
    pub resize: Resize,
    /// Keep the photo's orientation: a portrait photo resizes to `height`x`width`.
    pub keep_orientation: bool,
    /// Crop to the working size's aspect ratio instead of stretching the photo into it.
    pub crop: bool,
    /// Which part the crop keeps: `center`, `top`, `bottom`, `left`, `right`, or a corner as `X,Y`.
    ///
    /// Refused without `crop`, since there would be nothing for it to place. Left out of `GET /api/options` when
    /// unset, for the same reason: sending the defaults back unchanged has to stay a valid request.
    #[serde(default, with = "crop_from", skip_serializing_if = "Option::is_none")]
    pub crop_from: Option<CropOrigin>,
    /// How far into the photo the crop moves, `1.0` to `10.0`.
    ///
    /// At `1.0` the kept rectangle touches two opposite edges, so `crop_from` can only slide it along one axis. Above
    /// that it keeps a proportionally smaller rectangle, which frees the other axis too. Refused without `crop`, and
    /// left out of `GET /api/options` when unset, like `crop_from`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crop_zoom: Option<f64>,
    /// Nearest-neighbour upscale applied to the result, 1 to 4.
    pub scale: u32,
    pub format: Format,
}

impl Default for DitherParams {
    fn default() -> Self {
        let defaults = DitherOptions::default();

        Self {
            saturation: defaults.saturation,
            brightness: defaults.brightness,
            color: defaults.color,
            width: dithering_core::DEFAULT_SIZE.0,
            height: dithering_core::DEFAULT_SIZE.1,
            preset: None,
            resize: Resize::Fit,
            keep_orientation: false,
            crop: false,
            crop_from: None,
            crop_zoom: None,
            scale: 1,
            format: Format::Indexed,
        }
    }
}

impl DitherParams {
    /// Checks the ranges and builds the pipeline's own options struct.
    pub fn to_options(self) -> Result<DitherOptions, ApiError> {
        if !self.saturation.is_finite() || !(0.0..=1.0).contains(&self.saturation) {
            return Err(ApiError::bad_request(format!(
                "saturation must be between 0.0 and 1.0, got {}",
                self.saturation
            )));
        }

        boost("brightness", self.brightness)?;
        boost("color", self.color)?;

        dimension("width", self.width)?;
        dimension("height", self.height)?;

        // A setting that cannot take effect is a mistake worth naming, the way an unknown parameter is.
        let unusable = [
            ("crop_from", self.crop_from.is_some()),
            ("crop_zoom", self.crop_zoom.is_some()),
        ]
        .into_iter()
        .find_map(|(name, given)| (given && !self.crop).then_some(name));

        if let Some(name) = unusable {
            return Err(ApiError::bad_request(format!(
                "{name} needs crop=true, which is the crop it places"
            )));
        }

        if let Some(factor) = self.resize.factor()
            && (!factor.is_finite() || !(0.0..=1.0).contains(&factor) || factor == 0.0)
        {
            return Err(ApiError::bad_request(format!(
                "resize must be true, false, or a fraction between 0 and 1, got {factor}"
            )));
        }

        if let Some(zoom) = self.crop_zoom
            && (!zoom.is_finite() || !(1.0..=f64::from(MAX_CROP_ZOOM)).contains(&zoom))
        {
            return Err(ApiError::bad_request(format!(
                "crop_zoom must be between 1.0 and {MAX_CROP_ZOOM}, got {zoom}"
            )));
        }

        if self.scale == 0 || self.scale > MAX_SCALE {
            return Err(ApiError::bad_request(format!(
                "scale must be between 1 and {MAX_SCALE}, got {}",
                self.scale
            )));
        }

        Ok(DitherOptions {
            saturation: self.saturation,
            brightness: self.brightness,
            color: self.color,
        })
    }

    /// The size to dither at, or `None` when the source resolution is kept.
    ///
    /// A preset reshapes `width`x`height` rather than replacing it: the largest rectangle of the preset's ratio that
    /// fits inside the pair, which is turned over first when the ratio disagrees with it. So `preset=instagram-story`
    /// against the default 600x400 is 337x600 rather than 225x400, and either way the result is never larger than the
    /// dimensions the request already had checked.
    pub fn working_size(self) -> Option<(u32, u32)> {
        matches!(self.resize, Resize::Fit)
            .then(|| dithering_core::ratio_size((self.width, self.height), self.working_ratio()))
    }

    /// The shape the geometry is measured against, whatever `resize` says.
    ///
    /// `resize=false` keeps the source resolution, but a crop still needs a shape to aim at, and this is it: the
    /// preset's ratio, or `width`x`height` when no preset was named. So `resize=false` means no scaling rather than no
    /// framing, and `crop` keeps working underneath it.
    pub fn working_ratio(self) -> (u32, u32) {
        match self.preset {
            Some(preset) => preset.ratio(),
            None => (self.width, self.height),
        }
    }

    /// How a photo that does not share the working size's shape is fitted to it.
    pub fn fit(self) -> FitOptions {
        FitOptions {
            keep_orientation: self.keep_orientation,
            crop: self.crop,
            crop_from: self.crop_from.unwrap_or_default(),
            crop_zoom: self.crop_zoom.map_or(1.0, |zoom| zoom as f32),
        }
    }
}

/// `crop_from` as the one string the pipeline already parses.
///
/// The pipeline owns the syntax, so the query string, the CLI and `GET /api/options` all read and write the same
/// spelling, and an unusable one comes back as a 400 carrying the parser's own message.
mod crop_from {
    use std::str::FromStr;

    use dithering_core::CropOrigin;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(origin: &Option<CropOrigin>, serializer: S) -> Result<S::Ok, S::Error> {
        match origin {
            Some(origin) => serializer.collect_str(origin),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<CropOrigin>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|raw| CropOrigin::from_str(&raw).map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// [`DitherParams`] pulled from the query string.
///
/// A thin wrapper over [`Query`], so a bad query string fails with the API's own JSON error shape instead of axum's
/// plain text.
#[derive(Debug)]
pub struct Params(pub DitherParams);

impl<S: Send + Sync> FromRequestParts<S> for Params {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(params) = Query::<DitherParams>::from_request_parts(parts, state)
            .await
            .map_err(|e| ApiError::new(e.status(), e.body_text()))?;

        Ok(Params(params))
    }
}

fn boost(name: &str, value: f64) -> Result<(), ApiError> {
    if !value.is_finite() || !(0.0..=MAX_BOOST).contains(&value) {
        return Err(ApiError::bad_request(format!(
            "{name} must be between 0.0 and {MAX_BOOST}, got {value}"
        )));
    }
    Ok(())
}

fn dimension(name: &str, value: u32) -> Result<(), ApiError> {
    if value == 0 || value > MAX_DIMENSION {
        return Err(ApiError::bad_request(format!(
            "{name} must be between 1 and {MAX_DIMENSION}, got {value}"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_match_the_pipeline() {
        let options = DitherParams::default().to_options().expect("the defaults are valid");
        assert_eq!(options, DitherOptions::default());
    }

    #[test]
    fn out_of_range_values_are_refused() {
        let bad = [
            DitherParams {
                saturation: 1.5,
                ..Default::default()
            },
            DitherParams {
                saturation: f64::NAN,
                ..Default::default()
            },
            DitherParams {
                width: 0,
                ..Default::default()
            },
            DitherParams {
                height: MAX_DIMENSION + 1,
                ..Default::default()
            },
            DitherParams {
                scale: MAX_SCALE + 1,
                ..Default::default()
            },
            DitherParams {
                scale: 0,
                ..Default::default()
            },
            DitherParams {
                resize: Resize::Factor(0.0),
                ..Default::default()
            },
            DitherParams {
                resize: Resize::Factor(2.0),
                ..Default::default()
            },
            DitherParams {
                crop: true,
                crop_zoom: Some(0.5),
                ..Default::default()
            },
            DitherParams {
                crop: true,
                crop_zoom: Some(f64::from(MAX_CROP_ZOOM) + 1.0),
                ..Default::default()
            },
        ];

        for params in bad {
            assert!(params.to_options().is_err(), "{params:?} should not be accepted");
        }
    }

    #[test]
    fn the_crop_settings_need_the_crop_itself() {
        let placed = DitherParams {
            crop_from: Some(CropOrigin::Top),
            ..Default::default()
        };
        assert!(placed.to_options().is_err());
        assert!(DitherParams { crop: true, ..placed }.to_options().is_ok());

        let zoomed = DitherParams {
            crop_zoom: Some(2.0),
            ..Default::default()
        };
        assert!(zoomed.to_options().is_err());
        assert!(DitherParams { crop: true, ..zoomed }.to_options().is_ok());
    }

    #[test]
    fn a_preset_reshapes_the_working_size_rather_than_replacing_it() {
        let params = DitherParams {
            preset: Some(Preset::InstagramStory),
            ..Default::default()
        };
        assert_eq!(params.working_size(), Some((337, 600)));
        assert_eq!(params.working_ratio(), (9, 16));
    }

    #[test]
    fn keeping_the_source_resolution_still_leaves_a_shape_to_frame_against() {
        let params = DitherParams {
            resize: Resize::Keep,
            preset: Some(Preset::InstagramPost),
            ..Default::default()
        };
        assert_eq!(params.working_size(), None);
        assert_eq!(params.working_ratio(), (1, 1));
    }

    #[test]
    fn every_preset_names_a_ratio_the_pipeline_knows() {
        for preset in Preset::ALL {
            assert!(dithering_core::preset_ratio(preset.name()).is_some(), "{preset:?}");
        }
        assert_eq!(Preset::ALL.len(), dithering_core::RATIO_PRESETS.len());
    }

    #[test]
    fn a_fraction_governs_the_scaling_alone() {
        let params = DitherParams {
            resize: Resize::Factor(0.75),
            crop: true,
            ..Default::default()
        };
        assert_eq!(params.resize.factor(), Some(0.75));
        // No working size to land on, but the crop still has a shape to aim at.
        assert_eq!(params.working_size(), None);
        assert_eq!(params.working_ratio(), dithering_core::DEFAULT_SIZE);
        assert!(params.fit().crop);
    }
}
