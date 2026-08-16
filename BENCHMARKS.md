# Benchmarks

A running log of `crates/dithering-core/benches/pipeline.rs`. One entry per change that moves the numbers. Every claim of a speedup gets a row here: what it was before, what it is after, and what it cost elsewhere.

## Reproduce

```sh
BENCH_LABEL=<name> cargo bench -p dithering-core
```

The bench reads every JPEG in `assets/`, sorted by name, so a run measures the same work every time. It writes `benchmarks/<name>.json`. The files in `benchmarks/` are the record behind this log.

The CLI wall clock is a separate cross-check. It should stay close to the parallel batch figure. If the two drift apart, the bench no longer covers something the CLI does.

```sh
cargo build --release -p dithering-core
time ./target/release/dithering-core assets/*.jpg -o /tmp/bench-out
```

`RAYON_NUM_THREADS=1` runs the same binary on one core. This is how to measure what parallelism buys without building an older version.

## Method

The bench times each photo on its own, under two groups of cases. The `stage` group breaks the default run into decode, resize, dither and encode. The `pipeline` group takes a decoded photo through to PNG bytes under six option sets. A `batch` group times the whole set as one unit, which is what the CLI does.

Each case runs a warmup pass, then three to five timed passes. The reported number is the **best** pass, not the mean. On a laptop the mean mostly measures what else the machine was doing. The best pass is the one least disturbed by the scheduler, so it compares two versions of the same code more reliably. The median goes into the JSON to show the spread. When the best and the median are far apart, the machine was busy and the run is worth repeating.

Two cautions:

- Compare only numbers from the same machine. The environment block below records which one.
- Thermal state matters. Treat any single-digit-percent change as noise until it reproduces.

## Environment

| | |
| --- | --- |
| Machine | Apple silicon, aarch64, 12 threads |
| OS | macOS |
| Profile | `release`, `opt-level = 3` |
| Dataset | 10 JPEG photos in `assets/`, 177.0 MP total, 8256x5504 at the largest |

## Trend

| Date | Change | Pipeline, default (ms) | Batch, all cores (ms) | CLI wall clock | Indexed PNG |
| --- | --- | ---: | ---: | ---: | ---: |
| 2026-08-16 | Baseline, no parallelism | 199.1 | not measured | not measured | 502.1 KiB |
| 2026-08-16 | 1. rayon in the hot stages and the batch loop | 128.4 | 132.6 | 0.13 s | 502.1 KiB |

## History

### 2026-08-16: baseline, no parallelism

File: `benchmarks/baseline.json`.

The first run, with every stage on one core. It set the reference point before any optimization.

| Group | Case | Batch (ms) |
| --- | --- | ---: |
| stage | decode jpeg | 336.5 |
| stage | resize | 111.7 |
| stage | dither | 56.3 |
| stage | encode indexed png | 28.8 |
| stage | encode rgb png | 14.8 |
| pipeline | default | 199.1 |
| pipeline | crop center | 182.4 |
| pipeline | crop top, zoom 2 | 153.1 |
| pipeline | instagram story, upright | 164.6 |
| pipeline | 1200x800 | 610.3 |
| pipeline | upscale 2x, rgb png | 286.8 |

Decoding dominates. Resize costs twice what the dither costs, because it is the stage that reads all 177 MP.

### 2026-08-16: rayon in the hot stages and the batch loop

File: `benchmarks/rayon.json`.

Four places went parallel:

1. `resize::box_reduce`, over the output rows. Each row reads a disjoint band of the source. This is the stage that reads every source pixel.
2. `IndexedImage::to_rgb`, over the rows. A row of slots expands into a row of pixels on its own.
3. `IndexedImage::scale_nearest`, over the rows. The call to `imageops::resize` went away with it. An integer factor is plain replication, so an output row is a source row with every byte repeated.
4. The CLI batch loop, one photo per core. `map` over an indexed parallel iterator keeps the reports in input order.

| Group | Case | Before (ms) | After (ms) | Speedup |
| --- | --- | ---: | ---: | ---: |
| stage | decode jpeg | 336.5 | 341.3 | 0.99x |
| stage | resize | 111.7 | 42.8 | **2.61x** |
| stage | dither | 56.3 | 55.7 | 1.01x |
| stage | encode indexed png | 28.8 | 29.3 | 0.99x |
| stage | encode rgb png | 14.8 | 15.3 | 0.97x |
| pipeline | default | 199.1 | 128.4 | 1.55x |
| pipeline | crop center | 182.4 | 124.9 | 1.46x |
| pipeline | crop top, zoom 2 | 153.1 | 123.8 | 1.24x |
| pipeline | instagram story, upright | 164.6 | 106.0 | 1.55x |
| pipeline | 1200x800 | 610.3 | 495.3 | 1.23x |
| pipeline | upscale 2x, rgb png | 286.8 | 137.6 | **2.08x** |

The `batch` group arrived with this change, so the baseline has no row for it. Both of its cases come from the same run:

| Case | Best (ms) |
| --- | ---: |
| sequential | 510.8 |
| parallel, one photo per core | **132.6** |

The CLI agrees. On the whole asset set it takes 0.56 s under `RAYON_NUM_THREADS=1` and 0.13 s on all cores, a 4.4x gain at 523% CPU.

The output did not change. The four PNG files from a fixed set of CLI runs are identical byte for byte before and after, and the indexed PNG total stays at 502.1 KiB.

What did not move, and why:

- **Decode**, 341 ms and still the most expensive stage. The JPEG decoder in `image` is single threaded, and one photo cannot be split across cores. Only the batch loop attacks this, which is why the batch gains 3.85x while the stage gains nothing.
- **Floyd-Steinberg**, 55.7 ms. Every pixel reads error that the pixel before it wrote. Rows can overlap only as a wavefront, one row starting once the row above is two pixels ahead. That needs a synchronization point per row, and this is already the cheap stage. The cores go to the batch loop instead. This is also why the `1200x800` case gains the least: at 0.96 MP the dither becomes the dominant stage.
- **PNG encoding**, 29.3 ms. The `png` crate encodes on one thread.

## Adding an entry

1. Run the bench before the change, with a label that names the state.
2. Make the change.
3. Run the bench again, with a label that names the change.
4. Check that the output is still identical, unless the change is meant to alter it.
5. Add a row to the trend table and a section under History.
