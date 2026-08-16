//! End-to-end tests over the router, driven in-process. No socket is bound.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use dithering_core::{RgbImage, io};
use dithering_server::{Config, router};
use tower::ServiceExt;

/// A small landscape PNG with enough colour variation to exercise the dither.
fn source_png() -> Vec<u8> {
    sized_png(120, 90)
}

/// A 3:1 image, black but for a white band down the middle third.
///
/// Both colours survive the dither exactly, so what comes back says which part of the source was kept.
fn banded_png() -> Vec<u8> {
    let (width, height) = (120u32, 40u32);
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for _ in 0..height {
        for x in 0..width {
            let band = (width / 3..2 * width / 3).contains(&x);
            let v = if band { 255u8 } else { 0 };
            pixels.extend_from_slice(&[v, v, v]);
        }
    }

    let image = RgbImage::from_raw(width, height, pixels).expect("the buffer matches the size");
    io::encode_rgb_png(&image).expect("the test image encodes")
}

/// The same image at an arbitrary size, for the orientation tests.
fn sized_png(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);

    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[(x * 2) as u8, (y * 2) as u8, ((x + y) % 256) as u8]);
        }
    }

    let image = RgbImage::from_raw(width, height, pixels).expect("the buffer matches the size");

    io::encode_rgb_png(&image).expect("the test image encodes")
}

/// `(width, height, colour type)` read straight out of the PNG header.
///
/// Colour type 3 is a palette image, 2 is truecolour. Reading the bytes rather than decoding keeps the assertion on
/// what was actually written.
fn png_header(bytes: &[u8]) -> (u32, u32, u8) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");

    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());

    (width, height, bytes[25])
}

async fn call(request: Request<Body>) -> Response {
    call_with(Config::default(), request).await
}

async fn call_with(config: Config, request: Request<Body>) -> Response {
    router(Arc::new(config))
        .oneshot(request)
        .await
        .expect("the router answers")
}

async fn body_bytes(response: Response) -> Bytes {
    axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body reads")
}

fn header(response: &Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .unwrap_or_else(|| panic!("{name} is missing"))
        .to_str()
        .expect("the header is text")
        .to_string()
}

fn post(uri: &str, content_type: &str, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .body(Body::from(body))
        .expect("the request builds")
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("the request builds")
}

/// A `multipart/form-data` body carrying one file field.
fn multipart(field: &str, image: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----dithering-test-boundary";
    let mut body = Vec::new();

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("content-disposition: form-data; name=\"{field}\"; filename=\"photo.png\"\r\n").as_bytes(),
    );
    body.extend_from_slice(b"content-type: image/png\r\n\r\n");
    body.extend_from_slice(image);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    (format!("multipart/form-data; boundary={boundary}"), body)
}

async fn error_message(response: Response) -> String {
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("the error is JSON");

    json["error"].as_str().expect("the error carries a message").to_string()
}

#[tokio::test]
async fn health_reports_ok() {
    let response = call(get("/health")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("health is JSON");
    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "dithering-server");
}

#[tokio::test]
async fn options_reports_the_defaults_and_the_palette() {
    let response = call(get("/api/options")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let json: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).expect("options is JSON");
    assert_eq!(json["defaults"]["width"], 600);
    assert_eq!(json["defaults"]["height"], 400);
    assert_eq!(json["defaults"]["palette_blend"], 0.6);
    assert_eq!(json["defaults"]["brightness"], 1.1);
    assert_eq!(json["defaults"]["color"], 1.4);
    assert_eq!(json["defaults"]["format"], "indexed");
    assert_eq!(json["default_size"], serde_json::json!([600, 400]));
    assert_eq!(json["presets"][0]["name"], "instagram-post");

    // Six slots, blended at the default amount, ready for CSS.
    let palette = json["palette"].as_array().expect("the palette is a list");
    assert_eq!(palette.len(), 6);
    assert_eq!(palette[1], "#ffffff");

    // Settings that need another one to take effect are left out, so the defaults stay a valid request.
    assert!(json["defaults"].get("crop_from").is_none());
    assert!(json["defaults"].get("crop_zoom").is_none());
}

#[tokio::test]
async fn the_reported_defaults_are_a_valid_request() {
    let json: serde_json::Value = serde_json::from_slice(&body_bytes(call(get("/api/options")).await).await).unwrap();
    let defaults = json["defaults"].as_object().expect("defaults is an object");

    let query: Vec<String> = defaults
        .iter()
        .map(|(key, value)| {
            let raw = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            format!("{key}={raw}")
        })
        .collect();

    let response = call(post(
        &format!("/api/dither?{}", query.join("&")),
        "image/png",
        source_png(),
    ))
    .await;

    assert_eq!(response.status(), StatusCode::OK, "the defaults must round-trip");
}

#[tokio::test]
async fn a_raw_body_comes_back_as_an_indexed_png() {
    let response = call(post("/api/dither", "image/png", source_png())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "image/png");
    assert_eq!(header(&response, "cache-control"), "no-store");
    assert_eq!(header(&response, "x-image-size"), "600x400");
    // No crop, so the whole photo was read.
    assert_eq!(header(&response, "x-crop-rect"), "0,0,120,90");

    let (width, height, color) = png_header(&body_bytes(response).await);
    assert_eq!((width, height), (600, 400));
    assert_eq!(color, 3, "the default format is a palette PNG");
}

#[tokio::test]
async fn a_multipart_upload_is_read_from_the_image_field() {
    for field in ["image", "file"] {
        let (content_type, body) = multipart(field, &source_png());
        let response = call(post("/api/dither", &content_type, body)).await;
        assert_eq!(response.status(), StatusCode::OK, "field `{field}` should be read");
        assert_eq!(header(&response, "x-image-size"), "600x400");
    }

    let (content_type, body) = multipart("photo", &source_png());
    let response = call(post("/api/dither", &content_type, body)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(error_message(response).await.contains("`image` field"));
}

#[tokio::test]
async fn the_rgb_format_returns_a_truecolour_png() {
    let response = call(post("/api/dither?format=rgb", "image/png", source_png())).await;
    assert_eq!(response.status(), StatusCode::OK);

    let (_, _, color) = png_header(&body_bytes(response).await);
    assert_eq!(color, 2, "format=rgb should not be indexed");
}

#[tokio::test]
async fn a_preset_reshapes_the_working_size_and_orientation_follows_the_photo() {
    let response = call(post("/api/dither?preset=instagram-story", "image/png", source_png())).await;
    assert_eq!(header(&response, "x-image-size"), "337x600");

    // The same preset on a portrait photo, asked to keep the photo's orientation.
    let response = call(post(
        "/api/dither?preset=instagram-story&keep_orientation=true",
        "image/png",
        sized_png(90, 120),
    ))
    .await;
    assert_eq!(header(&response, "x-image-size"), "337x600");

    // A landscape photo under the same request comes back landscape.
    let response = call(post(
        "/api/dither?preset=instagram-story&keep_orientation=true",
        "image/png",
        source_png(),
    ))
    .await;
    assert_eq!(header(&response, "x-image-size"), "600x337");
}

#[tokio::test]
async fn a_crop_reports_the_region_it_kept() {
    // A 3:1 photo cropped to 3:2 keeps a full-height band, centred.
    let response = call(post("/api/dither?crop=true", "image/png", banded_png())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "x-crop-rect"), "30,0,60,40");
    assert_eq!(header(&response, "x-image-size"), "600x400");

    // The same crop pinned to the left edge starts at x=0.
    let response = call(post("/api/dither?crop=true&crop_from=left", "image/png", banded_png())).await;
    assert_eq!(header(&response, "x-crop-rect"), "0,0,60,40");

    // A corner is kept as given, and the rectangle grows from it.
    let response = call(post("/api/dither?crop=true&crop_from=20,5", "image/png", banded_png())).await;
    assert_eq!(header(&response, "x-crop-rect"), "20,5,52,35");
}

#[tokio::test]
async fn keeping_the_source_resolution_still_frames_the_photo() {
    let response = call(post("/api/dither?resize=false", "image/png", banded_png())).await;
    assert_eq!(header(&response, "x-image-size"), "120x40");

    // No scaling, but the crop still applies.
    let response = call(post("/api/dither?resize=false&crop=true", "image/png", banded_png())).await;
    assert_eq!(header(&response, "x-image-size"), "60x40");
    assert_eq!(header(&response, "x-crop-rect"), "30,0,60,40");
}

#[tokio::test]
async fn a_fraction_takes_a_share_of_what_the_framing_kept() {
    let response = call(post("/api/dither?resize=0.5", "image/png", banded_png())).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "x-image-size"), "60x20");
}

#[tokio::test]
async fn scale_upsizes_the_result_with_nearest_neighbour() {
    let response = call(post("/api/dither?scale=2", "image/png", source_png())).await;
    assert_eq!(header(&response, "x-image-size"), "1200x800");

    let (width, height, _) = png_header(&body_bytes(response).await);
    assert_eq!((width, height), (1200, 800));
}

#[tokio::test]
async fn a_bad_query_string_comes_back_as_json() {
    let cases = [
        ("/api/dither?palette_blend=2", "palette_blend"),
        ("/api/dither?brightness=9", "brightness"),
        ("/api/dither?color=-1", "color"),
        ("/api/dither?width=0", "width"),
        ("/api/dither?scale=99", "scale"),
        ("/api/dither?crop_from=top", "crop=true"),
        ("/api/dither?crop=true&crop_zoom=0.5", "crop_zoom"),
        ("/api/dither?resize=2", "resize"),
        ("/api/dither?preset=myspace", "preset"),
        ("/api/dither?nonsense=1", "nonsense"),
        ("/api/dither?crop=true&crop_from=sideways", "crop_from"),
    ];

    for (uri, expected) in cases {
        let response = call(post(uri, "image/png", source_png())).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri} should be refused");

        let message = error_message(response).await;
        assert!(
            message.contains(expected),
            "{uri} said `{message}`, expected `{expected}`"
        );
    }
}

#[tokio::test]
async fn a_request_without_a_usable_image_is_refused() {
    let empty = call(post("/api/dither", "image/png", Vec::new())).await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    assert!(error_message(empty).await.contains("no image"));

    let junk = call(post("/api/dither", "image/png", b"not an image at all".to_vec())).await;
    assert_eq!(junk.status(), StatusCode::BAD_REQUEST);
    assert!(error_message(junk).await.contains("could not read the image"));
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_as_too_large() {
    let config = Config {
        max_upload_bytes: 64,
        ..Default::default()
    };

    let response = call_with(config, post("/api/dither", "image/png", source_png())).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn the_pipeline_agrees_with_the_core() {
    // The same photo through the crate directly must give the same bytes the route returns.
    let source = source_png();
    let response = call(post("/api/dither", "image/png", source.clone())).await;
    let served = body_bytes(response).await;

    let photo = io::decode_rgb(&source).expect("the test image decodes");
    let working = dithering_core::resize_to_fit(&photo, dithering_core::DEFAULT_SIZE, Default::default());
    let dithered = dithering_core::apply_dithering(&working, &Default::default());
    let expected = io::encode_indexed_png(&dithered).expect("the result encodes");

    assert_eq!(served.as_ref(), expected.as_slice());
}
