//! Picture decoding, resizing and the candidate encoders.
//!
//! Decoding is bounded: the header is read first and a picture over the pixel cap is refused
//! before any pixel memory is allocated, so a hostile file cannot use a small download to exhaust
//! a phone. Encoders are measured, not assumed: this module is how `hq suite measure` compares
//! size, speed and quality of each candidate on real files.
//!
//! When a picture is recompressed for real (milestone M1) its EXIF orientation must be applied
//! before the pixels are re-encoded, since the re-encoded file carries no EXIF. The measurement
//! here does not, because orientation changes neither size nor speed.

use std::fmt;
use std::io::Cursor;
use std::time::{Duration, Instant};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::codecs::webp::WebPEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, ImageReader, RgbImage, RgbaImage};

/// One way to turn pixels into a file, with its quality setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Encoder {
    /// Baseline JPEG from the `image` crate's pure-Rust encoder. No alpha: transparency is
    /// flattened onto white.
    Jpeg { quality: u8 },
    /// Lossless PNG at the best compression.
    Png,
    /// Lossless WebP from the `image` crate.
    WebpLossless,
    /// Lossy WebP in pure Rust (`oxideav-webp`), behind `cand-webp-pure`.
    WebpPure { quality: u8 },
    /// Lossy WebP through libwebp, behind `cand-webp-lib`.
    WebpLib { quality: u8 },
    /// AVIF through rav1e, behind `cand-avif`.
    Avif { quality: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    #[error(
        "`{0}` is not an encoder; try jpeg:75, png, webp-lossless, webp-pure:75, webp-lib:75, avif:55"
    )]
    UnknownEncoder(String),
    #[error("{0} is not built into this binary (build with the `{1}` feature)")]
    NotBuilt(String, &'static str),
    #[error("the picture is too large to decode here ({0} pixels)")]
    TooManyPixels(u64),
    #[error("the picture could not be read: {0}")]
    Unreadable(String),
    #[error("encoding failed: {0}")]
    Encode(String),
}

impl Encoder {
    /// The text form used on the command line and as a key in reports: `jpeg-q75`.
    pub fn label(&self) -> String {
        match self {
            Encoder::Jpeg { quality } => format!("jpeg-q{quality}"),
            Encoder::Png => "png".into(),
            Encoder::WebpLossless => "webp-lossless".into(),
            Encoder::WebpPure { quality } => format!("webp-pure-q{quality}"),
            Encoder::WebpLib { quality } => format!("webp-lib-q{quality}"),
            Encoder::Avif { quality } => format!("avif-q{quality}"),
        }
    }

    /// Reads `jpeg:75`, `png`, `webp-lossless`, `webp-pure:75`, `webp-lib:75`, `avif:55`.
    /// Refuses an encoder this binary was not built with, saying which feature adds it.
    pub fn parse(s: &str) -> Result<Encoder, CodecError> {
        let (name, q) = match s.split_once(':') {
            Some((n, q)) => (n, Some(q)),
            None => (s, None),
        };
        let quality = |default: u8| -> Result<u8, CodecError> {
            match q {
                None => Ok(default),
                Some(q) => q
                    .parse::<u8>()
                    .ok()
                    .filter(|v| (1..=100).contains(v))
                    .ok_or_else(|| CodecError::UnknownEncoder(s.to_string())),
            }
        };
        let enc = match name {
            "jpeg" | "jpg" => Encoder::Jpeg {
                quality: quality(75)?,
            },
            "png" if q.is_none() => Encoder::Png,
            "webp-lossless" if q.is_none() => Encoder::WebpLossless,
            "webp-pure" => Encoder::WebpPure {
                quality: quality(75)?,
            },
            "webp-lib" => Encoder::WebpLib {
                quality: quality(75)?,
            },
            "avif" => Encoder::Avif {
                quality: quality(55)?,
            },
            _ => return Err(CodecError::UnknownEncoder(s.to_string())),
        };
        match enc.built_with() {
            Some(feature) => Err(CodecError::NotBuilt(enc.label(), feature)),
            None => Ok(enc),
        }
    }

    /// The feature that must be on for this encoder, or `None` if it is built in.
    fn built_with(&self) -> Option<&'static str> {
        match self {
            Encoder::WebpPure { .. } if !cfg!(feature = "cand-webp-pure") => Some("cand-webp-pure"),
            Encoder::WebpLib { .. } if !cfg!(feature = "cand-webp-lib") => Some("cand-webp-lib"),
            Encoder::Avif { .. } if !cfg!(feature = "cand-avif") => Some("cand-avif"),
            _ => None,
        }
    }

    /// The file extension a picture made by this encoder is filed under.
    pub fn extension(&self) -> &'static str {
        match self {
            Encoder::Jpeg { .. } => "jpg",
            Encoder::Png => "png",
            Encoder::WebpLossless | Encoder::WebpPure { .. } | Encoder::WebpLib { .. } => "webp",
            Encoder::Avif { .. } => "avif",
        }
    }

    /// Whether the output can differ from the input pixels.
    pub fn is_lossy(&self) -> bool {
        !matches!(self, Encoder::Png | Encoder::WebpLossless)
    }

    /// The encoders this binary has, with default settings. Used when none is asked for.
    pub fn defaults() -> Vec<Encoder> {
        let mut v = vec![
            Encoder::Jpeg { quality: 75 },
            Encoder::Png,
            Encoder::WebpLossless,
        ];
        if cfg!(feature = "cand-webp-pure") {
            v.push(Encoder::WebpPure { quality: 75 });
        }
        if cfg!(feature = "cand-webp-lib") {
            v.push(Encoder::WebpLib { quality: 75 });
        }
        if cfg!(feature = "cand-avif") {
            v.push(Encoder::Avif { quality: 55 });
        }
        v
    }

    /// The image format a file made by this encoder is read back with, for quality checks.
    fn read_back(&self) -> Option<image::ImageFormat> {
        match self {
            Encoder::Jpeg { .. } => Some(image::ImageFormat::Jpeg),
            Encoder::Png => Some(image::ImageFormat::Png),
            Encoder::WebpLossless | Encoder::WebpPure { .. } | Encoder::WebpLib { .. } => {
                Some(image::ImageFormat::WebP)
            }
            // The `image` crate here has no AVIF decoder, so quality is not measured.
            Encoder::Avif { .. } => None,
        }
    }
}

impl fmt::Display for Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label())
    }
}

/// A finished encode.
#[derive(Debug)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub took: Duration,
    /// Transparent pixels were drawn onto white because the format cannot hold them.
    pub alpha_flattened: bool,
    /// Peak signal to noise ratio against the input, in dB, for lossy encoders whose output this
    /// build can read back. Higher is closer; 40 dB and up is hard to tell from the original.
    pub psnr_db: Option<f64>,
}

/// Decodes a picture, refusing one over `max_pixels` before allocating for it.
pub fn decode_limited(bytes: &[u8], max_pixels: u64) -> Result<DynamicImage, CodecError> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| CodecError::Unreadable(e.to_string()))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| CodecError::Unreadable(e.to_string()))?;
    let pixels = u64::from(w) * u64::from(h);
    if pixels == 0 || pixels > max_pixels {
        return Err(CodecError::TooManyPixels(pixels));
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| CodecError::Unreadable(e.to_string()))?;
    let mut limits = image::Limits::default();
    // Four bytes a pixel, twice over for the decoder's own working copy.
    limits.max_alloc = Some(pixels.saturating_mul(8).max(64 * 1024));
    limits.max_image_width = Some(w);
    limits.max_image_height = Some(h);
    reader.limits(limits);
    reader
        .decode()
        .map_err(|e| CodecError::Unreadable(e.to_string()))
}

/// Shrinks a picture so its longer side is at most `max_long_edge`. A picture already within
/// the bound is returned as it is.
pub fn fit_long_edge(img: DynamicImage, max_long_edge: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    let long = w.max(h);
    if max_long_edge == 0 || long <= max_long_edge {
        return img;
    }
    let scale = f64::from(max_long_edge) / f64::from(long);
    let nw = ((f64::from(w) * scale).round() as u32).max(1);
    let nh = ((f64::from(h) * scale).round() as u32).max(1);
    img.resize_exact(nw, nh, FilterType::Lanczos3)
}

/// Whether any pixel is not fully opaque.
pub fn has_transparency(img: &DynamicImage) -> bool {
    img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p.0[3] != 255)
}

/// The picture with transparency drawn onto white.
fn flatten(img: &DynamicImage) -> RgbImage {
    if !has_transparency(img) {
        return img.to_rgb8();
    }
    let rgba: RgbaImage = img.to_rgba8();
    let mut out = RgbImage::new(rgba.width(), rgba.height());
    for (x, y, p) in rgba.enumerate_pixels() {
        let a = u32::from(p.0[3]);
        let blend = |c: u8| ((u32::from(c) * a + 255 * (255 - a)) / 255) as u8;
        out.put_pixel(
            x,
            y,
            image::Rgb([blend(p.0[0]), blend(p.0[1]), blend(p.0[2])]),
        );
    }
    out
}

/// Peak signal to noise ratio between two same-sized RGB buffers, in dB. `None` if they are
/// identical (infinite) or differ in size.
pub fn psnr_db(a: &[u8], b: &[u8]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let sum: u64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| {
            let d = i64::from(*x) - i64::from(*y);
            (d * d) as u64
        })
        .sum();
    if sum == 0 {
        return None;
    }
    let mse = sum as f64 / a.len() as f64;
    Some(10.0 * (255.0f64 * 255.0 / mse).log10())
}

/// Encodes `img` with `enc`, timing it and, for a lossy encoder this build can read back,
/// measuring how far the result is from the input.
pub fn encode(img: &DynamicImage, enc: Encoder) -> Result<Encoded, CodecError> {
    if let Some(feature) = enc.built_with() {
        return Err(CodecError::NotBuilt(enc.label(), feature));
    }
    let transparent = has_transparency(img);
    let start = Instant::now();
    let (bytes, flattened) = encode_inner(img, enc, transparent)?;
    let took = start.elapsed();

    let psnr_db = match (enc.is_lossy(), enc.read_back()) {
        (true, Some(format)) => {
            let reference = flatten(img);
            image::load_from_memory_with_format(&bytes, format)
                .ok()
                .and_then(|back| {
                    let back = back.to_rgb8();
                    if back.dimensions() == reference.dimensions() {
                        psnr_db(reference.as_raw(), back.as_raw())
                    } else {
                        None
                    }
                })
        }
        _ => None,
    };
    Ok(Encoded {
        bytes,
        took,
        alpha_flattened: flattened,
        psnr_db,
    })
}

fn encode_inner(
    img: &DynamicImage,
    enc: Encoder,
    transparent: bool,
) -> Result<(Vec<u8>, bool), CodecError> {
    let err = |e: &dyn fmt::Display| CodecError::Encode(e.to_string());
    let mut out = Vec::new();
    let (w, h) = (img.width(), img.height());
    match enc {
        Encoder::Jpeg { quality } => {
            let rgb = flatten(img);
            JpegEncoder::new_with_quality(&mut out, quality)
                .write_image(rgb.as_raw(), w, h, ExtendedColorType::Rgb8)
                .map_err(|e| err(&e))?;
            Ok((out, transparent))
        }
        Encoder::Png => {
            let enc =
                PngEncoder::new_with_quality(&mut out, CompressionType::Best, PngFilter::Adaptive);
            if transparent {
                enc.write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)
            } else {
                enc.write_image(img.to_rgb8().as_raw(), w, h, ExtendedColorType::Rgb8)
            }
            .map_err(|e| err(&e))?;
            Ok((out, false))
        }
        Encoder::WebpLossless => {
            WebPEncoder::new_lossless(&mut out)
                .write_image(img.to_rgba8().as_raw(), w, h, ExtendedColorType::Rgba8)
                .map_err(|e| err(&e))?;
            Ok((out, false))
        }
        Encoder::WebpPure { quality } => {
            encode_webp_pure(img, quality, transparent).map(|b| (b, false))
        }
        Encoder::WebpLib { quality } => {
            encode_webp_lib(img, quality, transparent).map(|b| (b, false))
        }
        Encoder::Avif { quality } => {
            encode_avif(img, quality, transparent).map(|b| (b, transparent))
        }
    }
}

#[cfg(feature = "cand-webp-pure")]
fn encode_webp_pure(
    img: &DynamicImage,
    quality: u8,
    transparent: bool,
) -> Result<Vec<u8>, CodecError> {
    let opts = oxideav_webp::EncodeOptions::default().with_quality(f32::from(quality));
    let (w, h) = (img.width(), img.height());
    if transparent {
        oxideav_webp::encode_rgba8(w, h, img.to_rgba8().as_raw(), &opts)
    } else {
        oxideav_webp::encode_rgb8(w, h, img.to_rgb8().as_raw(), &opts)
    }
    .map_err(|e| CodecError::Encode(e.to_string()))
}
#[cfg(not(feature = "cand-webp-pure"))]
fn encode_webp_pure(_: &DynamicImage, _: u8, _: bool) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::NotBuilt("webp-pure".into(), "cand-webp-pure"))
}

#[cfg(feature = "cand-webp-lib")]
fn encode_webp_lib(
    img: &DynamicImage,
    quality: u8,
    transparent: bool,
) -> Result<Vec<u8>, CodecError> {
    let (w, h) = (img.width(), img.height());
    let mem = if transparent {
        let rgba = img.to_rgba8();
        webp::Encoder::from_rgba(rgba.as_raw(), w, h).encode(f32::from(quality))
    } else {
        let rgb = img.to_rgb8();
        webp::Encoder::from_rgb(rgb.as_raw(), w, h).encode(f32::from(quality))
    };
    Ok(mem.to_vec())
}
#[cfg(not(feature = "cand-webp-lib"))]
fn encode_webp_lib(_: &DynamicImage, _: u8, _: bool) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::NotBuilt("webp-lib".into(), "cand-webp-lib"))
}

#[cfg(feature = "cand-avif")]
fn encode_avif(img: &DynamicImage, quality: u8, _transparent: bool) -> Result<Vec<u8>, CodecError> {
    let rgb = flatten(img);
    let pixels: Vec<rgb::RGB8> = rgb
        .as_raw()
        .chunks_exact(3)
        .map(|c| rgb::RGB8::new(c[0], c[1], c[2]))
        .collect();
    let frame = imgref::Img::new(pixels, rgb.width() as usize, rgb.height() as usize);
    ravif::Encoder::new()
        .with_quality(f32::from(quality))
        .with_speed(8)
        .encode_rgb(frame.as_ref())
        .map(|r| r.avif_file)
        .map_err(|e| CodecError::Encode(e.to_string()))
}
#[cfg(not(feature = "cand-avif"))]
fn encode_avif(_: &DynamicImage, _: u8, _: bool) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::NotBuilt("avif".into(), "cand-avif"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth gradient with a little deterministic texture, so lossy encoders have something
    /// to do and the result is the same everywhere.
    pub(crate) fn sample(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let n = ((x * 7 + y * 13) % 17) as u8;
            *p = image::Rgb([
                (x * 255 / w.max(1)) as u8 ^ n,
                (y * 255 / h.max(1)) as u8,
                ((x + y) * 255 / (w + h).max(1)) as u8,
            ]);
        }
        DynamicImage::ImageRgb8(img)
    }

    fn png_bytes(img: &DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn encoder_names_parse_and_print() {
        assert_eq!(
            Encoder::parse("jpeg:60").unwrap(),
            Encoder::Jpeg { quality: 60 }
        );
        assert_eq!(Encoder::parse("jpeg").unwrap().label(), "jpeg-q75");
        assert_eq!(Encoder::parse("png").unwrap(), Encoder::Png);
        assert_eq!(
            Encoder::parse("webp-lossless").unwrap().label(),
            "webp-lossless"
        );
        for bad in [
            "",
            "gif",
            "jpeg:0",
            "jpeg:101",
            "jpeg:x",
            "png:5",
            "webp-lossless:9",
        ] {
            assert!(
                matches!(Encoder::parse(bad), Err(CodecError::UnknownEncoder(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_candidate_that_is_not_built_says_which_feature_adds_it() {
        if !cfg!(feature = "cand-avif") {
            assert_eq!(
                Encoder::parse("avif:55"),
                Err(CodecError::NotBuilt("avif-q55".into(), "cand-avif"))
            );
        }
        if !cfg!(feature = "cand-webp-lib") {
            assert!(matches!(
                Encoder::parse("webp-lib"),
                Err(CodecError::NotBuilt(_, "cand-webp-lib"))
            ));
        }
    }

    #[test]
    fn every_built_encoder_makes_a_readable_smaller_file() {
        let img = sample(160, 120);
        let raw = 160 * 120 * 3;
        for enc in Encoder::defaults() {
            let out = encode(&img, enc).unwrap_or_else(|e| panic!("{enc}: {e}"));
            assert!(!out.bytes.is_empty(), "{enc}");
            assert!(
                out.bytes.len() < raw,
                "{enc} did not compress: {}",
                out.bytes.len()
            );
            if let Some(psnr) = out.psnr_db {
                assert!(enc.is_lossy() && psnr > 20.0, "{enc}: {psnr}");
            }
        }
    }

    #[test]
    fn lossless_encoders_return_the_same_pixels() {
        let img = sample(64, 48);
        for (enc, fmt) in [
            (Encoder::Png, image::ImageFormat::Png),
            (Encoder::WebpLossless, image::ImageFormat::WebP),
        ] {
            let out = encode(&img, enc).unwrap();
            let back = image::load_from_memory_with_format(&out.bytes, fmt).unwrap();
            assert_eq!(back.to_rgb8().as_raw(), img.to_rgb8().as_raw(), "{enc}");
            assert_eq!(out.psnr_db, None);
        }
    }

    #[test]
    fn higher_jpeg_quality_is_bigger_and_closer() {
        let img = sample(200, 150);
        let low = encode(&img, Encoder::Jpeg { quality: 30 }).unwrap();
        let high = encode(&img, Encoder::Jpeg { quality: 90 }).unwrap();
        assert!(high.bytes.len() > low.bytes.len());
        assert!(high.psnr_db.unwrap() > low.psnr_db.unwrap());
    }

    #[test]
    fn transparency_is_flattened_for_jpeg_and_kept_by_lossless_formats() {
        let mut rgba = RgbaImage::new(32, 32);
        for p in rgba.pixels_mut() {
            *p = image::Rgba([200, 30, 30, 0]);
        }
        let img = DynamicImage::ImageRgba8(rgba);
        assert!(has_transparency(&img));
        assert!(
            encode(&img, Encoder::Jpeg { quality: 75 })
                .unwrap()
                .alpha_flattened
        );
        assert!(!encode(&img, Encoder::WebpLossless).unwrap().alpha_flattened);
        let opaque =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255])));
        assert!(!has_transparency(&opaque));
    }

    #[test]
    fn the_pixel_cap_is_enforced_before_decoding() {
        let bytes = png_bytes(&sample(40, 30));
        assert!(decode_limited(&bytes, 1200).is_ok());
        assert_eq!(
            decode_limited(&bytes, 1199).unwrap_err(),
            CodecError::TooManyPixels(1200)
        );
        assert!(matches!(
            decode_limited(b"not an image", 1000),
            Err(CodecError::Unreadable(_))
        ));
        assert!(matches!(
            decode_limited(&bytes[..20], 1_000_000),
            Err(CodecError::Unreadable(_))
        ));
    }

    #[test]
    fn resizing_keeps_the_aspect_and_never_enlarges() {
        let big = sample(400, 200);
        let fit = fit_long_edge(big.clone(), 100);
        assert_eq!((fit.width(), fit.height()), (100, 50));
        let same = fit_long_edge(big, 1000);
        assert_eq!((same.width(), same.height()), (400, 200));
        let tall = fit_long_edge(sample(30, 300), 100);
        assert_eq!((tall.width(), tall.height()), (10, 100));
        let sliver = fit_long_edge(sample(1000, 1), 10);
        assert_eq!((sliver.width(), sliver.height()), (10, 1));
    }

    #[test]
    fn psnr_behaves() {
        let a = vec![100u8; 300];
        assert_eq!(
            psnr_db(&a, &a),
            None,
            "identical is infinite, reported as none"
        );
        let mut b = a.clone();
        b[0] = 110;
        let close = psnr_db(&a, &b).unwrap();
        b.iter_mut().for_each(|x| *x = x.saturating_add(30));
        assert!(psnr_db(&a, &b).unwrap() < close);
        assert_eq!(psnr_db(&a, &a[..10]), None);
        assert_eq!(psnr_db(&[], &[]), None);
    }
}
