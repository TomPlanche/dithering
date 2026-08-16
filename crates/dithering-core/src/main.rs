//! Command line front end for the core dithering pipeline.

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use clap::builder::TypedValueParser;
use clap::{Parser, ValueEnum};
use dithering_core::{CropOrigin, DitherOptions, FitOptions, MAX_CROP_ZOOM, RgbImage, apply_dithering, io, resize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum FormatArg {
    /// Indexed PNG carrying the palette. Smaller, and the default.
    Indexed,
    /// Plain RGB PNG, for viewers that dislike palette images.
    Rgb,
}

/// Dither photos to a fixed colour palette with Floyd-Steinberg error diffusion.
#[derive(Debug, Parser)]
#[command(name = "dithering-core", version, about, long_about = None)]
struct Cli {
    /// Images to process.
    #[arg(required = true, value_name = "IMAGE")]
    inputs: Vec<PathBuf>,

    /// Output file for a single input, or a directory for several.
    #[arg(short, long, value_name = "PATH")]
    output: Option<PathBuf>,

    /// Blend between the pure and the muted palettes, 0.0 to 1.0.
    #[arg(long, default_value_t = 0.6, value_name = "F", value_parser = parse_saturation)]
    saturation: f64,

    /// Working size, as WIDTHxHEIGHT. With --preset it is the box the ratio is fitted inside.
    #[arg(long, default_value = "600x400", value_name = "WxH", value_parser = parse_size)]
    size: (u32, u32),

    /// Aspect ratio by name, for the shapes a platform expects. Reshapes --size rather than replacing it.
    #[arg(
        long,
        value_name = "NAME",
        value_parser = clap::builder::PossibleValuesParser::new(resize::preset_names())
            .map(|name| resize::preset_ratio(&name).expect("the parser only accepts preset names")),
    )]
    preset: Option<(u32, u32)>,

    /// Dither at the source resolution instead of resizing first.
    #[arg(long)]
    no_resize: bool,

    /// Scale by a fraction of the source instead of to the working size: 0.75 takes a quarter off.
    #[arg(long, conflicts_with = "no_resize", value_name = "F", value_parser = parse_factor)]
    resize: Option<f64>,

    /// Keep the photo's orientation: a portrait photo resizes to the transpose of the working size.
    #[arg(long)]
    keep_orientation: bool,

    /// Crop to the working size's aspect ratio instead of stretching the photo into it.
    #[arg(long)]
    crop: bool,

    /// Which part the crop keeps: center, top, bottom, left, right, or a corner as X,Y.
    #[arg(long, requires = "crop", default_value = "center", value_name = "WHERE")]
    crop_from: CropOrigin,

    /// How far into the photo the crop moves. Above 1.0 it keeps a smaller rectangle, which frees both axes.
    #[arg(long, requires = "crop", default_value_t = 1.0, value_name = "F", value_parser = parse_zoom)]
    crop_zoom: f32,

    /// Double the output with nearest-neighbour.
    #[arg(long)]
    upscale_2x: bool,

    /// Output encoding.
    #[arg(short, long, value_enum, default_value = "indexed")]
    format: FormatArg,

    /// Report what would be written without writing it.
    #[arg(long)]
    dry_run: bool,

    /// Print per-image timings.
    #[arg(short, long)]
    verbose: bool,
}

fn parse_size(raw: &str) -> Result<(u32, u32), String> {
    let (w, h) = raw
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("expected WIDTHxHEIGHT, got `{raw}`"))?;

    let w: u32 = w.trim().parse().map_err(|_| format!("bad width `{w}`"))?;
    let h: u32 = h.trim().parse().map_err(|_| format!("bad height `{h}`"))?;

    if w == 0 || h == 0 {
        return Err("width and height must be non-zero".into());
    }

    Ok((w, h))
}

/// The palette blend, which is a position between the two palettes rather than a multiplier.
fn parse_saturation(raw: &str) -> Result<f64, String> {
    let saturation: f64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("expected a number, got `{raw}`"))?;

    if !(0.0..=1.0).contains(&saturation) {
        return Err(format!("expected 0.0 to 1.0, got {saturation}"));
    }

    Ok(saturation)
}

/// The resize fraction, which is a part of the photo's own size rather than a size of its own.
fn parse_factor(raw: &str) -> Result<f64, String> {
    let factor: f64 = raw
        .trim()
        .parse()
        .map_err(|_| format!("expected a number, got `{raw}`"))?;

    if !factor.is_finite() || factor <= 0.0 || factor > 1.0 {
        return Err(format!("expected a fraction between 0 and 1, got {factor}"));
    }

    Ok(factor)
}

/// The crop zoom, which has to be at least 1.0: below that there is nothing left inside the photo to keep.
fn parse_zoom(raw: &str) -> Result<f32, String> {
    let zoom: f32 = raw
        .trim()
        .parse()
        .map_err(|_| format!("expected a number, got `{raw}`"))?;

    if !zoom.is_finite() || !(1.0..=MAX_CROP_ZOOM).contains(&zoom) {
        return Err(format!("expected 1.0 to {MAX_CROP_ZOOM}, got {zoom}"));
    }

    Ok(zoom)
}

impl Cli {
    fn dither_options(&self) -> DitherOptions {
        DitherOptions {
            saturation: self.saturation,
        }
    }

    /// The size to dither at: `--size`, reshaped to `--preset`'s ratio when one was named.
    ///
    /// A preset only ever picks the shape, so `--size` still says how much gets dithered. It is fitted inside the pair
    /// rather than replacing it, and the pair is turned over first when the ratio disagrees with it, so
    /// `--preset instagram-story` against the default 600x400 is 337x600 rather than 225x400.
    fn working_size(&self) -> (u32, u32) {
        match self.preset {
            Some(ratio) => resize::ratio_size(self.size, ratio),
            None => self.size,
        }
    }

    /// The shape the geometry is measured against, whatever `--no-resize` says.
    ///
    /// `--no-resize` keeps the source resolution, but a crop still needs a shape to aim at, and this is it. So it means
    /// no scaling rather than no framing, and `--crop` keeps working underneath it.
    fn working_ratio(&self) -> (u32, u32) {
        self.preset.unwrap_or_else(|| self.working_size())
    }

    fn fit(&self) -> FitOptions {
        FitOptions {
            keep_orientation: self.keep_orientation,
            crop: self.crop,
            crop_from: self.crop_from,
            crop_zoom: self.crop_zoom,
        }
    }

    /// Where a given input's dithered PNG should land.
    ///
    /// A single input may name its output file directly; otherwise outputs are `<stem>_dithered.png`, alongside the
    /// input.
    fn output_path(&self, input: &Path) -> PathBuf {
        let default_name = {
            let stem = input.file_stem().unwrap_or_default().to_string_lossy();
            PathBuf::from(format!("{stem}_dithered.png"))
        };

        match &self.output {
            // A lone input plus a path that is not an existing directory names the file.
            Some(path) if self.inputs.len() == 1 && !path.is_dir() => path.clone(),
            Some(dir) => dir.join(default_name),
            None => input.parent().map(|p| p.join(&default_name)).unwrap_or(default_name),
        }
    }
}

/// Runs the pipeline for one input and returns what should be printed for it.
fn process(cli: &Cli, input: &Path) -> Result<String, Box<dyn Error>> {
    let started = Instant::now();
    let photo = io::load_rgb(input)?;
    let source_size = photo.dimensions();

    // The shape the framing is measured against, which is the working size itself when scaling to it.
    let target = if cli.no_resize || cli.resize.is_some() {
        cli.working_ratio()
    } else {
        cli.working_size()
    };

    // What the crop kept, so `--verbose` can say why a coordinate did not move anything.
    let kept = cli.crop.then(|| resize::fitted_rect(source_size, target, cli.fit()));

    let working: RgbImage = match cli.resize {
        Some(factor) => resize::scale_to_fit(&photo, target, cli.fit(), factor),
        // Keeping the source pixels is no reason to stop framing them.
        None if cli.no_resize && cli.crop => resize::crop_to_fit(&photo, target, cli.fit()),
        None if cli.no_resize => photo,
        None => resize::resize_to_fit(&photo, target, cli.fit()),
    };

    let dithered = apply_dithering(&working, &cli.dither_options());

    let final_image = if cli.upscale_2x {
        dithered.scale_nearest(2)
    } else {
        dithered
    };

    let out_path = cli.output_path(input);

    if cli.dry_run {
        return Ok(format!("{} -> {}", input.display(), out_path.display()));
    }

    if let Some(parent) = out_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    match cli.format {
        FormatArg::Indexed => io::save_indexed_png(&final_image, &out_path)?,
        FormatArg::Rgb => io::save_rgb_png(&final_image.to_rgb(), &out_path)?,
    }

    if !cli.verbose {
        return Ok(out_path.display().to_string());
    }

    let mut report = format!(
        "{} ({}x{}) -> {} ({}x{}) in {:.0}ms",
        input.display(),
        source_size.0,
        source_size.1,
        out_path.display(),
        final_image.width(),
        final_image.height(),
        started.elapsed().as_secs_f64() * 1000.0,
    );

    if let Some((x, y, width, height)) = kept {
        let _ = write!(report, "\n  crop: {width}x{height} from {x},{y}");
    }

    Ok(report)
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut failed = 0usize;
    for input in &cli.inputs {
        match process(&cli, input) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("error: {}: {e}", input.display());
                failed += 1;
            },
        }
    }

    if failed > 0 {
        eprintln!("{failed} of {} image(s) failed", cli.inputs.len());

        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("the arguments parse")
    }

    #[test]
    fn the_command_definition_is_sound() {
        Cli::command().debug_assert();
    }

    #[test]
    fn a_preset_reshapes_the_working_size_rather_than_replacing_it() {
        let cli = parse(&["dithering-core", "photo.jpg", "--preset", "instagram-story"]);
        assert_eq!(cli.working_size(), (337, 600));

        let cli = parse(&[
            "dithering-core",
            "photo.jpg",
            "--preset",
            "iphone",
            "--size",
            "1200x800",
        ]);
        assert_eq!(cli.working_size(), (1066, 800));
    }

    #[test]
    fn without_a_preset_the_working_size_is_the_size_asked_for() {
        let cli = parse(&["dithering-core", "photo.jpg"]);

        assert_eq!(cli.working_size(), (600, 400));
        assert_eq!(cli.working_ratio(), (600, 400));
    }

    #[test]
    fn no_resize_still_frames_against_the_preset_ratio() {
        let cli = parse(&[
            "dithering-core",
            "photo.jpg",
            "--no-resize",
            "--preset",
            "instagram-post",
        ]);

        assert_eq!(cli.working_ratio(), (1, 1));
    }

    #[test]
    fn a_lone_input_may_name_its_output_file() {
        let cli = parse(&["dithering-core", "photo.jpg", "-o", "out.png"]);

        assert_eq!(cli.output_path(Path::new("photo.jpg")), PathBuf::from("out.png"));
    }

    #[test]
    fn several_inputs_land_beside_themselves_under_a_default_name() {
        let cli = parse(&["dithering-core", "a/one.jpg", "b/two.jpg"]);

        assert_eq!(
            cli.output_path(Path::new("a/one.jpg")),
            PathBuf::from("a/one_dithered.png")
        );
        assert_eq!(
            cli.output_path(Path::new("b/two.jpg")),
            PathBuf::from("b/two_dithered.png")
        );
    }

    #[test]
    fn out_of_range_numbers_are_refused() {
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--saturation", "1.5"]).is_err());
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--resize", "0"]).is_err());
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--crop", "--crop-zoom", "0.5"]).is_err());
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--size", "600x0"]).is_err());
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--preset", "myspace"]).is_err());
    }

    #[test]
    fn the_crop_flags_need_the_crop_itself() {
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--crop-from", "top"]).is_err());
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--crop", "--crop-from", "top"]).is_ok());
    }

    #[test]
    fn scaling_by_a_fraction_and_not_scaling_at_all_are_different_asks() {
        assert!(Cli::try_parse_from(["dithering-core", "p.jpg", "--no-resize", "--resize", "0.5"]).is_err());
    }
}
