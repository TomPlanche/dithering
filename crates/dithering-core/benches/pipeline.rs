//! Benchmark of the pipeline over every photo in `assets/`.
//!
//! Run it with `cargo bench -p dithering-core`. There is no external harness: the stages run in the tens of
//! milliseconds, so a warmup pass plus a few timed passes separate them well enough.
//!
//! Each photo is timed on its own, under two groups of cases:
//!
//! * `stage` breaks the default run into decode, resize, dither and encode.
//! * `pipeline` runs a decoded photo through to PNG bytes under several option sets.
//!
//! The run writes `benchmarks/<label>.json` at the repository root, with one entry per photo per case. Set the label
//! with `BENCH_LABEL`, which defaults to `latest`. Keep a reference point before an optimisation with
//! `BENCH_LABEL=baseline cargo bench -p dithering-core`, then compare the two files.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dithering_core::{CropOrigin, DitherOptions, FitOptions, IndexedImage, RgbImage, apply_dithering, io, resize};
use rayon::prelude::*;

/// The size the pipeline dithers at unless a case says otherwise.
const WORKING: (u32, u32) = resize::DEFAULT_SIZE;

/// One decoded sample photo, kept at full resolution.
struct Asset {
    name: String,
    bytes: Vec<u8>,
    full: RgbImage,
}

impl Asset {
    /// Source pixels, in megapixels.
    fn megapixels(&self) -> f64 {
        (self.full.width() as f64 * self.full.height() as f64) / 1e6
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every JPEG in `assets/`, sorted by name so two runs measure the same work in the same order.
fn load_assets() -> Vec<Asset> {
    let dir = repo_root().join("assets");
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"))
        })
        .collect();
    paths.sort();

    assert!(!paths.is_empty(), "no photos in {}", dir.display());

    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path).expect("readable photo");
            let full = io::decode_rgb(&bytes).expect("decodable photo");

            Asset {
                name: path.file_name().unwrap().to_string_lossy().into_owned(),
                bytes,
                full,
            }
        })
        .collect()
}

/// Which encoder the case ends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Indexed,
    Rgb,
}

/// One set of options, run over every photo.
struct Config {
    name: &'static str,
    /// The working size, which a preset reshapes rather than replaces.
    size: (u32, u32),
    preset: Option<(&'static str, (u32, u32))>,
    fit: FitOptions,
    saturation: f64,
    upscale: u32,
    format: Format,
}

impl Config {
    /// The shape the framing is measured against.
    fn target(&self) -> (u32, u32) {
        match self.preset {
            Some((_, ratio)) => resize::ratio_size(self.size, ratio),
            None => self.size,
        }
    }

    /// What the case asked for, spelled out for the report.
    fn options(&self) -> String {
        let mut out = format!("size={}x{}", self.size.0, self.size.1);
        if let Some((name, _)) = self.preset {
            let _ = write!(out, " preset={name}");
        }
        if self.fit.keep_orientation {
            out.push_str(" keep-orientation");
        }
        if self.fit.crop {
            let _ = write!(out, " crop={} zoom={}", self.fit.crop_from, self.fit.crop_zoom);
        }
        if self.upscale > 1 {
            let _ = write!(out, " upscale={}x", self.upscale);
        }
        let defaults = DitherOptions::default();
        let _ = write!(
            out,
            " saturation={} brightness={} color={} format={:?}",
            self.saturation, defaults.brightness, defaults.color, self.format
        );
        out.to_lowercase()
    }

    /// Resize, dither, upscale, encode. Returns the encoded size, which is worth watching alongside the timings.
    fn run(&self, photo: &RgbImage) -> usize {
        let working = resize::resize_to_fit(photo, self.target(), self.fit);
        let options = DitherOptions {
            saturation: self.saturation,
            ..Default::default()
        };
        let dithered = apply_dithering(&working, &options).scale_nearest(self.upscale);

        match self.format {
            Format::Indexed => io::encode_indexed_png(&dithered).expect("encodable").len(),
            Format::Rgb => io::encode_rgb_png(&dithered.to_rgb()).expect("encodable").len(),
        }
    }
}

/// The option sets the `pipeline` group runs.
fn configs() -> Vec<Config> {
    let crop_center = FitOptions {
        crop: true,
        ..Default::default()
    };

    vec![
        Config {
            name: "default",
            size: WORKING,
            preset: None,
            fit: FitOptions::default(),
            saturation: 0.6,
            upscale: 1,
            format: Format::Indexed,
        },
        Config {
            name: "crop center",
            size: WORKING,
            preset: None,
            fit: crop_center,
            saturation: 0.6,
            upscale: 1,
            format: Format::Indexed,
        },
        Config {
            name: "crop top, zoom 2",
            size: WORKING,
            preset: None,
            fit: FitOptions {
                crop: true,
                crop_from: CropOrigin::Top,
                crop_zoom: 2.0,
                ..Default::default()
            },
            saturation: 0.6,
            upscale: 1,
            format: Format::Indexed,
        },
        Config {
            name: "instagram story, upright",
            size: WORKING,
            preset: Some(("instagram-story", (9, 16))),
            fit: FitOptions {
                keep_orientation: true,
                crop: true,
                ..Default::default()
            },
            saturation: 0.6,
            upscale: 1,
            format: Format::Indexed,
        },
        Config {
            name: "1200x800",
            size: (1200, 800),
            preset: None,
            fit: crop_center,
            saturation: 0.6,
            upscale: 1,
            format: Format::Indexed,
        },
        Config {
            name: "upscale 2x, rgb png",
            size: WORKING,
            preset: None,
            fit: FitOptions::default(),
            saturation: 0.6,
            upscale: 2,
            format: Format::Rgb,
        },
    ]
}

/// One photo under one case.
struct Sample {
    image: String,
    best: Duration,
    median: Duration,
    /// Encoded output, for the cases that produce one.
    bytes: Option<usize>,
}

/// One case, over every photo.
struct Case {
    group: &'static str,
    name: String,
    options: String,
    samples: Vec<Sample>,
}

impl Case {
    /// The whole batch, as the sum of the per-photo bests.
    fn total(&self) -> Duration {
        self.samples.iter().map(|s| s.best).sum()
    }

    fn total_bytes(&self) -> Option<usize> {
        self.samples.iter().map(|s| s.bytes).sum()
    }
}

/// The whole photo set under one case, timed as a single unit.
///
/// This is what the CLI spends on `dithering-core assets/*.jpg`. It exists next to the per-photo cases because the
/// batch loop is where whole photos run in parallel, which per-photo timings cannot show.
struct Batch {
    name: &'static str,
    options: String,
    best: Duration,
    median: Duration,
    bytes: usize,
}

/// Keeps the optimiser from deleting a stage whose result is otherwise unused.
#[inline]
fn keep<T>(value: T) -> T {
    std::hint::black_box(value)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Times `body` on every photo: `warmup` untimed passes, then `reps` timed ones.
///
/// The best pass is the one to compare against, since it is the least disturbed by the scheduler. The median comes
/// along to show the spread. When the two are far apart, the machine was busy and the run is worth repeating.
fn measure(
    group: &'static str,
    name: impl Into<String>,
    options: impl Into<String>,
    assets: &[Asset],
    warmup: usize,
    reps: usize,
    mut body: impl FnMut(usize, &Asset) -> Option<usize>,
) -> Case {
    let mut samples = Vec::with_capacity(assets.len());

    for (index, asset) in assets.iter().enumerate() {
        for _ in 0..warmup {
            keep(body(index, asset));
        }

        let mut passes = Vec::with_capacity(reps);
        let mut bytes = None;
        for _ in 0..reps {
            let started = Instant::now();
            let produced = body(index, asset);
            passes.push(started.elapsed());
            bytes = keep(produced);
        }
        passes.sort_unstable();

        samples.push(Sample {
            image: asset.name.clone(),
            best: passes[0],
            median: passes[passes.len() / 2],
            bytes,
        });
    }

    Case {
        group,
        name: name.into(),
        options: options.into(),
        samples,
    }
}

/// Times `body` over the whole set: `warmup` untimed passes, then `reps` timed ones.
fn measure_batch(
    name: &'static str,
    options: impl Into<String>,
    warmup: usize,
    reps: usize,
    mut body: impl FnMut() -> usize,
) -> Batch {
    for _ in 0..warmup {
        keep(body());
    }

    let mut passes = Vec::with_capacity(reps);
    let mut bytes = 0;
    for _ in 0..reps {
        let started = Instant::now();
        let produced = body();
        passes.push(started.elapsed());
        bytes = keep(produced);
    }
    passes.sort_unstable();

    Batch {
        name,
        options: options.into(),
        best: passes[0],
        median: passes[passes.len() / 2],
        bytes,
    }
}

/// Decode included, since a batch starts at the file.
fn full_run(config: &Config, asset: &Asset) -> usize {
    config.run(&io::decode_rgb(&asset.bytes).expect("decodable"))
}

/// The batch loop, run the way the CLI runs it and the way it used to.
fn measure_batches(assets: &[Asset]) -> Vec<Batch> {
    let config = configs().remove(0);
    let options = format!("decode..encode, {}", config.options());

    vec![
        measure_batch("sequential", &options, 1, 3, || {
            assets.iter().map(|asset| full_run(&config, asset)).sum()
        }),
        measure_batch("parallel, one photo per core", &options, 1, 3, || {
            assets.par_iter().map(|asset| full_run(&config, asset)).sum()
        }),
    ]
}

fn measure_all(assets: &[Asset]) -> Vec<Case> {
    let options = DitherOptions::default();
    let fit = FitOptions::default();

    // Inputs for the later stages, so each stage is timed on its own.
    let sized: Vec<RgbImage> = assets
        .iter()
        .map(|a| resize::resize_to_fit(&a.full, WORKING, fit))
        .collect();
    let dithered: Vec<IndexedImage> = sized.iter().map(|image| apply_dithering(image, &options)).collect();
    let expanded: Vec<RgbImage> = dithered.iter().map(IndexedImage::to_rgb).collect();

    let working = format!("{}x{}", WORKING.0, WORKING.1);
    let mut cases = vec![
        measure("stage", "decode jpeg", "", assets, 1, 3, |_, asset| {
            keep(io::decode_rgb(&asset.bytes).expect("decodable"));
            None
        }),
        measure("stage", "resize", &working, assets, 1, 3, |_, asset| {
            keep(resize::resize_to_fit(&asset.full, WORKING, fit));
            None
        }),
        measure("stage", "dither", &working, assets, 1, 5, |i, _| {
            keep(apply_dithering(&sized[i], &options));
            None
        }),
        measure("stage", "encode indexed png", &working, assets, 1, 5, |i, _| {
            Some(io::encode_indexed_png(&dithered[i]).expect("encodable").len())
        }),
        measure("stage", "encode rgb png", &working, assets, 1, 5, |i, _| {
            Some(io::encode_rgb_png(&expanded[i]).expect("encodable").len())
        }),
    ];

    for config in configs() {
        cases.push(measure(
            "pipeline",
            config.name,
            config.options(),
            assets,
            1,
            3,
            |_, asset| Some(config.run(&asset.full)),
        ));
    }

    cases
}

/// The stdout report: one line per case, then the per-photo detail.
fn report(assets: &[Asset], cases: &[Case], batches: &[Batch]) -> String {
    let count = assets.len() as f64;
    let mut out = String::new();

    writeln!(
        out,
        "{:<10} {:<26} {:>11} {:>11} {:>11}",
        "group", "case", "batch (ms)", "per photo", "output (KiB)"
    )
    .unwrap();
    writeln!(out, "{}", "-".repeat(74)).unwrap();
    for case in cases {
        let bytes = match case.total_bytes() {
            Some(bytes) => format!("{:.1}", bytes as f64 / 1024.0),
            None => "-".to_string(),
        };
        writeln!(
            out,
            "{:<10} {:<26} {:>11.1} {:>11.2} {:>11}",
            case.group,
            case.name,
            ms(case.total()),
            ms(case.total()) / count,
            bytes,
        )
        .unwrap();
    }

    for batch in batches {
        writeln!(
            out,
            "{:<10} {:<26} {:>11.1} {:>11.2} {:>11.1}",
            "batch",
            batch.name,
            ms(batch.best),
            ms(batch.best) / count,
            batch.bytes as f64 / 1024.0,
        )
        .unwrap();
    }

    writeln!(out).unwrap();
    writeln!(out, "per photo, best of the timed passes, in ms").unwrap();
    writeln!(out, "{}", "-".repeat(74)).unwrap();

    // Case names are sentences, so the columns are numbered and the legend below says what each one is.
    let mut header = format!("{:<46}", "photo");
    for (index, _) in cases.iter().enumerate() {
        let _ = write!(header, " {:>8}", format!("#{}", index + 1));
    }
    writeln!(out, "{header}").unwrap();

    for (index, asset) in assets.iter().enumerate() {
        let mut row = format!("{:<46}", short_name(&asset.name));
        for case in cases {
            let _ = write!(row, " {:>8.2}", ms(case.samples[index].best));
        }
        writeln!(out, "{row}").unwrap();
    }

    writeln!(out).unwrap();
    for (index, case) in cases.iter().enumerate() {
        writeln!(out, "  #{:<3} {:<10} {}", index + 1, case.group, case.name).unwrap();
    }

    out
}

/// A photo name that fits, keeping the tail, which is what tells two of them apart.
fn short_name(name: &str) -> String {
    if name.len() <= 46 {
        return name.to_string();
    }
    format!("...{}", &name[name.len() - 43..])
}

fn json_string(raw: &str) -> String {
    let escaped: String = raw
        .chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            c => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

/// The export, written by hand so the bench keeps no dependencies of its own.
fn json(assets: &[Asset], cases: &[Case], batches: &[Batch], label: &str) -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };

    let mut out = String::new();
    writeln!(out, "{{").unwrap();
    writeln!(out, "  \"label\": {},", json_string(label)).unwrap();
    writeln!(out, "  \"generated_unix\": {seconds},").unwrap();
    writeln!(out, "  \"os\": {},", json_string(std::env::consts::OS)).unwrap();
    writeln!(out, "  \"arch\": {},", json_string(std::env::consts::ARCH)).unwrap();
    writeln!(out, "  \"threads\": {threads},").unwrap();
    writeln!(out, "  \"profile\": {},", json_string(profile)).unwrap();
    writeln!(out, "  \"working_size\": [{}, {}],", WORKING.0, WORKING.1).unwrap();

    writeln!(out, "  \"photos\": [").unwrap();
    for (i, asset) in assets.iter().enumerate() {
        let comma = if i + 1 == assets.len() { "" } else { "," };
        writeln!(
            out,
            "    {{\"name\": {}, \"width\": {}, \"height\": {}, \"file_bytes\": {}}}{comma}",
            json_string(&asset.name),
            asset.full.width(),
            asset.full.height(),
            asset.bytes.len(),
        )
        .unwrap();
    }
    writeln!(out, "  ],").unwrap();

    writeln!(out, "  \"cases\": [").unwrap();
    for (i, case) in cases.iter().enumerate() {
        let comma = if i + 1 == cases.len() { "" } else { "," };
        writeln!(out, "    {{").unwrap();
        writeln!(out, "      \"group\": {},", json_string(case.group)).unwrap();
        writeln!(out, "      \"name\": {},", json_string(&case.name)).unwrap();
        writeln!(out, "      \"options\": {},", json_string(&case.options)).unwrap();
        writeln!(out, "      \"batch_best_ms\": {:.3},", ms(case.total())).unwrap();
        match case.total_bytes() {
            Some(bytes) => writeln!(out, "      \"output_bytes\": {bytes},").unwrap(),
            None => writeln!(out, "      \"output_bytes\": null,").unwrap(),
        }
        writeln!(out, "      \"photos\": [").unwrap();
        for (j, sample) in case.samples.iter().enumerate() {
            let comma = if j + 1 == case.samples.len() { "" } else { "," };
            let bytes = match sample.bytes {
                Some(bytes) => bytes.to_string(),
                None => "null".to_string(),
            };
            writeln!(
                out,
                "        {{\"name\": {}, \"best_ms\": {:.3}, \"median_ms\": {:.3}, \"output_bytes\": {bytes}}}{comma}",
                json_string(&sample.image),
                ms(sample.best),
                ms(sample.median),
            )
            .unwrap();
        }
        writeln!(out, "      ]").unwrap();
        writeln!(out, "    }}{comma}").unwrap();
    }
    writeln!(out, "  ],").unwrap();

    writeln!(out, "  \"batches\": [").unwrap();
    for (i, batch) in batches.iter().enumerate() {
        let comma = if i + 1 == batches.len() { "" } else { "," };
        writeln!(
            out,
            "    {{\"name\": {}, \"options\": {}, \"best_ms\": {:.3}, \"median_ms\": {:.3}, \"output_bytes\": {}}}{comma}",
            json_string(batch.name),
            json_string(&batch.options),
            ms(batch.best),
            ms(batch.median),
            batch.bytes,
        )
        .unwrap();
    }
    writeln!(out, "  ]").unwrap();
    writeln!(out, "}}").unwrap();

    out
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("warning: this is a debug build, so the numbers mean nothing. Run `cargo bench`.");
    }

    let assets = load_assets();
    let megapixels: f64 = assets.iter().map(Asset::megapixels).sum();
    println!(
        "dithering-core benchmark: {} photos, {megapixels:.1} MP total, working size {}x{}",
        assets.len(),
        WORKING.0,
        WORKING.1
    );
    println!();

    let cases = measure_all(&assets);
    let batches = measure_batches(&assets);
    print!("{}", report(&assets, &cases, &batches));

    let label = std::env::var("BENCH_LABEL").unwrap_or_else(|_| "latest".to_string());
    let dir = repo_root().join("benchmarks");
    fs::create_dir_all(&dir).expect("the benchmarks directory is writable");
    let path = dir.join(format!("{label}.json"));
    fs::write(&path, json(&assets, &cases, &batches, &label)).expect("the report is writable");
    let shown = fs::canonicalize(&path).unwrap_or(path);

    println!();
    println!("written to {}", shown.display());
    println!("options, one line per pipeline case:");
    for case in cases.iter().filter(|c| c.group == "pipeline") {
        println!("  {:<26} {}", case.name, case.options);
    }
}
