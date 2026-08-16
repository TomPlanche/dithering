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
| `dithering-server` | HTTP backend over the pipeline. |

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

Three settings decide how the result looks. `--saturation` names the six colours, between pure primaries at 0.0 and muted ink at 1.0. `--brightness` and `--color` work on the photo before it reaches them: a gain on every channel, and a push away from each pixel's own grey. Both sit above 1.0 by default, because six colours cannot hold a midtone. `--brightness 1.0 --color 1.0` hands the photo to the dither untouched.

## Server

```sh
cargo run -p dithering-server
```

It listens on `127.0.0.1:3000` and stores nothing between requests.

| Route | What it does |
| --- | --- |
| `GET /health` | Liveness probe. |
| `GET /api/options` | Defaults, accepted values, the palette. |
| `POST /api/dither` | A dithered PNG. |

Send the photo as the raw body, or as a `multipart/form-data` field named `image`. The settings ride in the query string under the same names `GET /api/options` reports, so a client can send the reported defaults straight back. The response carries `x-image-size` and `x-crop-rect`, which say what the pipeline landed on and which part of the upload it read.

```sh
curl -X POST --data-binary @photo.jpg \
  'localhost:3000/api/dither?preset=instagram-story&crop=true&keep_orientation=true' -o out.png
```

`HOST`, `PORT`, `CORS_ORIGINS` and `MAX_UPLOAD_BYTES` configure it. Every one has a default.

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

[BENCHMARKS.md](BENCHMARKS.md) logs what each change did to the numbers.

## Credits

The photos in `assets/` come from [Pexels](https://www.pexels.com).
