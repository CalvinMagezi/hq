//! Pictures inside a document package, read one at a time under the importer's limits.

use std::ops::ControlFlow;

use hq_convert::ingest::package::{Limits, Package};
use hq_convert::ingest::sniff::Kind;
use hq_convert::types::ConvertError;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// What a media part is, judged from its first bytes. The part's name is not trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Jpeg,
    Png,
    Gif,
    Webp,
    Tiff,
    Bmp,
    /// Windows metafiles Office uses for pasted drawings.
    Emf,
    Wmf,
    Svg,
    Other,
}

impl ImageFormat {
    pub fn label(self) -> &'static str {
        match self {
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Png => "png",
            ImageFormat::Gif => "gif",
            ImageFormat::Webp => "webp",
            ImageFormat::Tiff => "tiff",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Emf => "emf",
            ImageFormat::Wmf => "wmf",
            ImageFormat::Svg => "svg",
            ImageFormat::Other => "other",
        }
    }

    /// Whether the shrink pipeline can decode and recompress this format as a still picture.
    /// GIF is left out because it may be animated, and recompressing would keep one frame.
    pub fn is_raster_we_can_recompress(self) -> bool {
        matches!(
            self,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Webp
        )
    }
}

pub fn sniff_image(b: &[u8]) -> ImageFormat {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ImageFormat::Jpeg
    } else if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        ImageFormat::Png
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        ImageFormat::Gif
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else if b.starts_with(&[b'I', b'I', 42, 0]) || b.starts_with(&[b'M', b'M', 0, 42]) {
        ImageFormat::Tiff
    } else if b.starts_with(b"BM") && b.len() > 14 {
        ImageFormat::Bmp
    } else if b.len() >= 44 && b[..4] == [1, 0, 0, 0] && &b[40..44] == b" EMF" {
        ImageFormat::Emf
    } else if b.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) {
        ImageFormat::Wmf
    } else if b
        .iter()
        .take(512)
        .copied()
        .collect::<Vec<u8>>()
        .windows(4)
        .any(|w| w == b"<svg")
    {
        ImageFormat::Svg
    } else {
        ImageFormat::Other
    }
}

/// One picture from a package, valid only inside the callback.
#[derive(Debug)]
pub struct MediaItem<'a> {
    /// The part name inside the package, such as `word/media/image1.png`. Never recorded in a
    /// measurement report: file names are not numbers.
    pub part: &'a str,
    pub bytes: &'a [u8],
    pub format: ImageFormat,
    pub sha256: String,
}

/// What a scan found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaScan {
    pub items: usize,
    pub bytes: u64,
    /// Parts left unread because they were over a safety limit.
    pub skipped_over_limit: usize,
}

/// The folder inside the package that holds media, by document kind.
pub fn media_prefix(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Docx => Some("word/media/"),
        Kind::Pptx => Some("ppt/media/"),
        Kind::Xlsx => Some("xl/media/"),
        Kind::Odt | Kind::Ods | Kind::Odp => Some("Pictures/"),
        _ => None,
    }
}

/// Calls `each` for every picture in the package, one at a time, so a document with many large
/// pictures never has more than one in memory here. Return `Break` to stop early.
///
/// Parts over the package limits are counted in `skipped_over_limit`; an encrypted part is an
/// error, since the picture cannot be read and the file is not safe to treat as understood.
pub fn for_each_media(
    bytes: &[u8],
    kind: Kind,
    limits: &Limits,
    mut each: impl FnMut(MediaItem<'_>) -> ControlFlow<()>,
) -> Result<MediaScan, ConvertError> {
    let mut scan = MediaScan::default();
    let Some(prefix) = media_prefix(kind) else {
        return Ok(scan);
    };
    let mut pkg = Package::open(bytes, limits)?;
    let names: Vec<String> = pkg
        .names()
        .iter()
        .filter(|n| n.starts_with(prefix) && !n.ends_with('/'))
        .cloned()
        .collect();
    for name in names {
        let data = match pkg.read(&name) {
            Ok(Some(d)) => d,
            Ok(None) => continue,
            Err(ConvertError::TooLarge(_)) => {
                scan.skipped_over_limit += 1;
                continue;
            }
            Err(e) => return Err(e),
        };
        scan.items += 1;
        scan.bytes += data.len() as u64;
        let item = MediaItem {
            part: &name,
            format: sniff_image(&data),
            sha256: hex::encode(Sha256::digest(&data)),
            bytes: &data,
        };
        if each(item).is_break() {
            break;
        }
    }
    Ok(scan)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in parts {
                z.start_file(*name, opts).unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];
    const JPG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xE0, 0, 0];

    #[test]
    fn formats_are_judged_by_content() {
        assert_eq!(sniff_image(PNG), ImageFormat::Png);
        assert_eq!(sniff_image(JPG), ImageFormat::Jpeg);
        assert_eq!(sniff_image(b"GIF89a.."), ImageFormat::Gif);
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), ImageFormat::Webp);
        assert_eq!(sniff_image(b"<?xml ?><svg xmlns=''/>"), ImageFormat::Svg);
        assert_eq!(sniff_image(b"nothing"), ImageFormat::Other);
        assert_eq!(sniff_image(b""), ImageFormat::Other);
        assert!(ImageFormat::Jpeg.is_raster_we_can_recompress());
        assert!(!ImageFormat::Emf.is_raster_we_can_recompress());
        assert!(!ImageFormat::Gif.is_raster_we_can_recompress());
    }

    #[test]
    fn finds_pictures_under_the_documents_media_folder_only() {
        let docx = zip_of(&[
            ("word/document.xml", b"<w:document/>"),
            ("word/media/image1.png", PNG),
            ("word/media/image2.jpeg", JPG),
            ("docProps/thumbnail.jpeg", JPG),
            ("ppt/media/other.png", PNG),
        ]);
        let mut seen = Vec::new();
        let scan = for_each_media(&docx, Kind::Docx, &Limits::default(), |m| {
            seen.push((m.format, m.bytes.len(), m.sha256.len()));
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!(scan.items, 2);
        assert_eq!(scan.bytes, (PNG.len() + JPG.len()) as u64);
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|s| s.2 == 64));
    }

    #[test]
    fn kinds_without_media_folders_find_nothing() {
        let scan = for_each_media(b"not a zip", Kind::Pdf, &Limits::default(), |_| {
            ControlFlow::Continue(())
        })
        .unwrap();
        assert_eq!(scan, MediaScan::default());
    }

    #[test]
    fn a_part_over_the_limit_is_counted_not_read() {
        let docx = zip_of(&[
            ("word/media/big.png", &vec![7u8; 5000]),
            ("word/media/s.png", PNG),
        ]);
        let limits = Limits {
            max_entry_bytes: 1000,
            ..Limits::default()
        };
        let scan =
            for_each_media(&docx, Kind::Docx, &limits, |_| ControlFlow::Continue(())).unwrap();
        assert_eq!((scan.items, scan.skipped_over_limit), (1, 1));
    }

    #[test]
    fn the_callback_can_stop_the_scan() {
        let docx = zip_of(&[("word/media/a.png", PNG), ("word/media/b.png", PNG)]);
        let mut n = 0;
        for_each_media(&docx, Kind::Docx, &Limits::default(), |_| {
            n += 1;
            ControlFlow::Break(())
        })
        .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn a_non_zip_is_an_error_not_a_panic() {
        assert!(
            for_each_media(b"junk", Kind::Docx, &Limits::default(), |_| {
                ControlFlow::Continue(())
            })
            .is_err()
        );
    }
}
