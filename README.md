# dithering

<pre>  
       ▄▄     ██               ▄▄                               ██                        
       ██     ▀▀       ██      ██                               ▀▀                        
  ▄███▄██   ████     ███████   ██▄████▄   ▄████▄    ██▄████   ████     ██▄████▄   ▄███▄██ 
 ██▀  ▀██     ██       ██      ██▀   ██  ██▄▄▄▄██   ██▀         ██     ██▀   ██  ██▀  ▀██ 
 ██    ██     ██       ██      ██    ██  ██▀▀▀▀▀▀   ██          ██     ██    ██  ██    ██ 
 ▀██▄▄███  ▄▄▄██▄▄▄    ██▄▄▄   ██    ██  ▀██▄▄▄▄█   ██       ▄▄▄██▄▄▄  ██    ██  ▀██▄▄███ 
   ▀▀▀ ▀▀  ▀▀▀▀▀▀▀▀     ▀▀▀▀   ▀▀    ▀▀    ▀▀▀▀▀    ▀▀       ▀▀▀▀▀▀▀▀  ▀▀    ▀▀   ▄▀▀▀ ██ 
                                                                                  ▀████▀▀ 
</pre>

Dithers photos to a fixed palette of six colors with Floyd-Steinberg error diffusion.

## Crates

| Crate | What it does |
| --- | --- |
| `dithering-core` | The pipeline, and a command line front end for it. |
| `dithering-server` | HTTP backend. A placeholder for now. |

## Command line

Dither one photo:

```sh
cargo run -p dithering-core -- photo.jpg
```

The result goes to `photo_dithered.png`, next to the input file. Give more than one image to run a batch.

Frame the photo for a platform:

```sh
cargo run -p dithering-core -- photo.jpg --preset instagram-story --crop --keep-orientation
```

`--preset` picks a shape. `--size` picks how many pixels the pipeline dithers. `--crop` takes the sides off instead of stretching the photo into the target. `--keep-orientation` turns the target over for a portrait photo. `--help` lists the rest.

## Library

```rust
use dithering_core::{DEFAULT_SIZE, DitherOptions, FitOptions, apply_dithering, io, resize};

let photo = io::load_rgb("photo.jpg")?;

let fit = FitOptions { crop: true, ..Default::default() };
let small = resize::resize_to_fit(&photo, DEFAULT_SIZE, fit);

let dithered = apply_dithering(&small, &DitherOptions::default());
io::save_indexed_png(&dithered, "out.png")?;
```

An `RgbImage` goes in. An `IndexedImage` comes out, which holds one palette slot per pixel.

## Features

| Feature | What it adds |
| --- | --- |
| `image-io` | Image decoding, and palette PNG encoding. |
| `cli` | The command line front end. On by default. |

## Tests

```sh
cargo test --workspace
```

## Benchmark

```sh
cargo bench -p dithering-core
```

The bench times every photo in `assets/`, stage by stage and under six option sets. It writes `benchmarks/latest.json`. Give the run another name with `BENCH_LABEL`. Compare a later run against `benchmarks/baseline.json` to see what a change did.

## Credits

The photos in `assets/` come from [Pexels](https://www.pexels.com).
