//! `hq suite measure`: what would shrinking this folder save, and what would it cost?
//!
//! The tool walks a folder, converts each file the way the importer would, pulls the pictures
//! out of document packages, recompresses them with each candidate encoder, and totals what
//! comes out. It writes nothing but its report and sends nothing anywhere.
//!
//! The report holds numbers and format classes only. File names, paths, document text, picture
//! content, hashes and the size of any single file are not in it, so the owner can read the JSON
//! once and decide whether to share it. A test fails if a name or a piece of content leaks.

use std::collections::{BTreeMap, HashSet};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use hq_convert::ingest::sniff::{Kind, sniff};
use hq_convert::ingest::{ConvertOptions, LossKind, OcrMode, capabilities};
use hq_suite_ingest::{FailureKind, ImageFormat, for_each_media, project};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::codec::{self, CodecError, Encoder};
use crate::probe;

/// The label of the baseline every encoder is compared with: pictures left exactly as they are.
pub const AS_IS: &str = "as-is";

/// How to measure.
#[derive(Debug, Clone)]
pub struct Settings {
    pub encoders: Vec<Encoder>,
    /// Pictures larger than this are scaled down to it before encoding, as shrink would.
    pub max_long_edge: u32,
    /// Pictures with more pixels than this are not decoded (and are counted).
    pub max_pixels: u64,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub convert: ConvertOptions,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            encoders: Encoder::defaults(),
            max_long_edge: 1600,
            max_pixels: 50_000_000,
            max_files: 5_000,
            max_file_bytes: 64 * 1024 * 1024,
            // OCR is off so a measurement is the same on every machine and never runs a slow
            // external engine; scans and photos are reported as classes instead.
            convert: ConvertOptions {
                ocr: OcrMode::Off,
                external_office: false,
                ..ConvertOptions::default()
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MeasureError {
    #[error("{0} is not a folder")]
    NotAFolder(String),
    #[error("could not read the folder: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Codec(#[from] CodecError),
}

// ---------------------------------------------------------------------------------------------
// Report

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MeasureReport {
    /// Bumped when a field changes meaning.
    pub schema: u32,
    pub host: Host,
    pub settings: SettingsView,
    pub totals: Totals,
    /// Per format class: `docx`, `xlsx`, `pdf-text`, `pdf-scan`, `image`, ...
    pub classes: BTreeMap<String, ClassStats>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Host {
    pub os: String,
    pub arch: String,
    pub ocr_available: bool,
    pub pdf_tables_available: bool,
    pub legacy_office_available: bool,
    /// Whether per-file peak memory could be read. If false, `peak_rss_kib_max` is missing.
    pub memory_probe: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SettingsView {
    pub encoders: Vec<String>,
    pub max_long_edge: u32,
    pub max_pixels: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Totals {
    pub files: u64,
    pub bytes: u64,
    pub folders_skipped_symlink: u64,
    pub files_unreadable: u64,
    pub files_over_size_cap: u64,
    pub files_over_count_cap: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ClassStats {
    pub files: u64,
    pub original_bytes: u64,
    /// Files identical to one seen earlier. They cost nothing in the record.
    pub duplicate_files: u64,
    pub duplicate_bytes: u64,
    /// Files the importer read.
    pub converted: u64,
    /// Files it could not, by reason (`encrypted`, `needs_ocr`, `unsupported`, ...).
    pub failures: BTreeMap<String, u64>,
    /// Projected fidelity tiers of converted files.
    pub tiers: BTreeMap<String, u64>,
    pub loss_kinds: BTreeMap<String, u64>,
    pub markdown_bytes: u64,
    pub convert_ms: Option<Spread>,
    pub peak_rss_kib_max: Option<u64>,
    pub media: MediaStats,
    pub encoders: BTreeMap<String, EncoderStats>,
    /// Per encoder (and `as-is`): what the files that could be projected would take.
    pub projected: BTreeMap<String, Projected>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct MediaStats {
    pub files_with_media: u64,
    pub items: u64,
    pub bytes: u64,
    /// Pictures seen before, in this file or another. They are stored once.
    pub duplicate_items: u64,
    pub duplicate_bytes: u64,
    pub by_format: BTreeMap<String, FormatCount>,
    /// Pictures not decoded, by reason: `not_recompressible`, `too_many_pixels`, `unreadable`.
    pub not_recompressed: BTreeMap<String, u64>,
    /// Package parts over a safety limit.
    pub parts_over_limit: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct FormatCount {
    pub items: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct EncoderStats {
    pub items: u64,
    /// Size of those pictures before encoding, as found in the file.
    pub bytes_in: u64,
    /// Size after scaling and encoding. Larger than `bytes_in` is possible.
    pub bytes_out: u64,
    /// Pictures where the encoded file was not smaller, so the original would be kept.
    pub kept_original: u64,
    pub alpha_flattened: u64,
    pub encode_ms: Option<Spread>,
    /// Mean peak signal to noise ratio over the pictures where it could be measured.
    pub psnr_db_mean: Option<f64>,
    pub psnr_db_min: Option<f64>,
    pub encode_failures: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Projected {
    pub files: u64,
    pub original_bytes: u64,
    pub record_bytes: u64,
    /// `record / original` per file, so a few huge files do not hide the typical one.
    pub ratio: Option<Quantiles>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Spread {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Quantiles {
    pub p10: f64,
    pub p50: f64,
    pub p90: f64,
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn spread(mut v: Vec<f64>) -> Option<Spread> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(Spread {
        p50: round2(quantile(&v, 0.5)),
        p95: round2(quantile(&v, 0.95)),
        max: round2(*v.last().unwrap()),
    })
}

fn quantiles(mut v: Vec<f64>) -> Option<Quantiles> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    Some(Quantiles {
        p10: round4(quantile(&v, 0.1)),
        p50: round4(quantile(&v, 0.5)),
        p90: round4(quantile(&v, 0.9)),
    })
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

// ---------------------------------------------------------------------------------------------
// Accumulation

#[derive(Default)]
struct EncAcc {
    stats: EncoderStats,
    ms: Vec<f64>,
    psnr: Vec<f64>,
}

#[derive(Default)]
struct ProjAcc {
    files: u64,
    original: u64,
    record: u64,
    ratios: Vec<f64>,
}

#[derive(Default)]
struct ClassAcc {
    stats: ClassStats,
    convert_ms: Vec<f64>,
    enc: BTreeMap<String, EncAcc>,
    proj: BTreeMap<String, ProjAcc>,
}

impl ClassAcc {
    fn finish(mut self) -> ClassStats {
        self.stats.convert_ms = spread(self.convert_ms);
        for (label, acc) in self.enc {
            let mut s = acc.stats;
            s.encode_ms = spread(acc.ms);
            if !acc.psnr.is_empty() {
                s.psnr_db_mean = Some(round2(acc.psnr.iter().sum::<f64>() / acc.psnr.len() as f64));
                s.psnr_db_min = Some(round2(acc.psnr.iter().copied().fold(f64::MAX, f64::min)));
            }
            self.stats.encoders.insert(label, s);
        }
        for (label, acc) in self.proj {
            self.stats.projected.insert(
                label,
                Projected {
                    files: acc.files,
                    original_bytes: acc.original,
                    record_bytes: acc.record,
                    ratio: quantiles(acc.ratios),
                },
            );
        }
        self.stats
    }
}

// ---------------------------------------------------------------------------------------------
// Pictures

/// What recompressing one picture gave, per encoder.
struct EncodeResult {
    label: String,
    out: Result<codec::Encoded, CodecError>,
}

struct ItemResult {
    format: ImageFormat,
    original: u64,
    /// Why it was not recompressed, if it was not.
    skipped: Option<&'static str>,
    encodes: Vec<EncodeResult>,
}

fn process_item(bytes: &[u8], format: ImageFormat, s: &Settings) -> ItemResult {
    let base = |skipped, encodes| ItemResult {
        format,
        original: bytes.len() as u64,
        skipped: Some(skipped),
        encodes,
    };
    if !format.is_raster_we_can_recompress() {
        return base("not_recompressible", Vec::new());
    }
    let img = match codec::decode_limited(bytes, s.max_pixels) {
        Ok(i) => codec::fit_long_edge(i, s.max_long_edge),
        Err(CodecError::TooManyPixels(_)) => return base("too_many_pixels", Vec::new()),
        Err(_) => return base("unreadable", Vec::new()),
    };
    let encodes = s
        .encoders
        .iter()
        .map(|e| EncodeResult {
            label: e.label(),
            out: codec::encode(&img, *e),
        })
        .collect();
    ItemResult {
        format,
        original: bytes.len() as u64,
        skipped: None,
        encodes,
    }
}

/// Everything learned about one file's pictures. Only pictures not seen before are processed.
#[derive(Default)]
struct MediaOutcome {
    items: Vec<ItemResult>,
    seen_items: u64,
    seen_bytes: u64,
    duplicate_items: u64,
    duplicate_bytes: u64,
    parts_over_limit: u64,
}

fn media_pass(
    data: &[u8],
    kind: Kind,
    s: &Settings,
    seen: &mut HashSet<String>,
) -> Result<MediaOutcome, hq_convert::types::ConvertError> {
    let mut out = MediaOutcome::default();
    if kind == Kind::Image {
        let format = hq_suite_ingest::sniff_image(data);
        out.seen_items = 1;
        out.seen_bytes = data.len() as u64;
        // A photo is its own media: no package, and it is never a duplicate of itself here
        // (whole-file duplicates were already dropped).
        out.items.push(process_item(data, format, s));
        return Ok(out);
    }
    let scan = for_each_media(data, kind, &s.convert.limits, |m| {
        out.seen_items += 1;
        out.seen_bytes += m.bytes.len() as u64;
        if seen.insert(m.sha256) {
            out.items.push(process_item(m.bytes, m.format, s));
        } else {
            out.duplicate_items += 1;
            out.duplicate_bytes += m.bytes.len() as u64;
        }
        ControlFlow::Continue(())
    })?;
    out.parts_over_limit = scan.skipped_over_limit as u64;
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// The walk

fn collect_files(
    root: &Path,
    max_files: usize,
    totals: &mut Totals,
) -> Result<Vec<PathBuf>, MeasureError> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if dir == root => return Err(e.into()),
            Err(_) => {
                totals.files_unreadable += 1;
                continue;
            }
        };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else {
                totals.files_unreadable += 1;
                continue;
            };
            if ft.is_symlink() {
                totals.folders_skipped_symlink += 1;
            } else if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if files.len() < max_files {
                    files.push(entry.path());
                } else {
                    totals.files_over_count_cap += 1;
                }
            }
        }
    }
    // The order of a directory listing is the file system's; sort so two runs agree.
    files.sort();
    Ok(files)
}

/// Measures every file under `root`. `progress` is called with (done, total) after each file.
pub async fn measure(
    root: &Path,
    settings: &Settings,
    mut progress: impl FnMut(usize, usize),
) -> Result<MeasureReport, MeasureError> {
    if !root.is_dir() {
        return Err(MeasureError::NotAFolder("the path given".into()));
    }
    let mut totals = Totals::default();
    let files = collect_files(root, settings.max_files, &mut totals)?;
    let total = files.len();
    let memory_probe = probe::peak_rss_kib().is_some() && probe::reset_peak_rss();
    let shared = Arc::new(settings.clone());

    let mut classes: BTreeMap<String, ClassAcc> = BTreeMap::new();
    let mut seen_files: HashSet<String> = HashSet::new();
    let mut seen_media: HashSet<String> = HashSet::new();
    let mut labels: Vec<String> = vec![AS_IS.to_string()];
    labels.extend(settings.encoders.iter().map(Encoder::label));

    for (done, path) in files.into_iter().enumerate() {
        progress(done, total);
        let Ok(meta) = std::fs::metadata(&path) else {
            totals.files_unreadable += 1;
            continue;
        };
        if meta.len() > settings.max_file_bytes {
            totals.files_over_size_cap += 1;
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            totals.files_unreadable += 1;
            continue;
        };
        let original = data.len() as u64;
        totals.files += 1;
        totals.bytes += original;

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        let sniffed = sniff(&data, &ext);
        let kind = sniffed.kind;
        let digest = hex::encode(Sha256::digest(&data));
        let duplicate = !seen_files.insert(digest);

        // The class a file lands in depends on how conversion went, so it is settled below;
        // duplicates and unreadable pictures use the plain kind.
        let mut acc_class = hq_suite_ingest::class_of(kind, None).to_string();

        if duplicate {
            let acc = classes.entry(acc_class).or_default();
            acc.stats.files += 1;
            acc.stats.original_bytes += original;
            acc.stats.duplicate_files += 1;
            acc.stats.duplicate_bytes += original;
            for l in &labels {
                let p = acc.proj.entry(l.clone()).or_default();
                p.files += 1;
                p.original += original;
                p.ratios.push(0.0);
            }
            continue;
        }

        if memory_probe {
            probe::reset_peak_rss();
        }

        // Pictures first, on the blocking pool, then hand the bytes back for conversion.
        let data = Arc::new(data);
        let media_result = if sniffed.encrypted {
            Ok(MediaOutcome::default())
        } else {
            let (d, s) = (Arc::clone(&data), Arc::clone(&shared));
            let mut seen = std::mem::take(&mut seen_media);
            let joined = tokio::task::spawn_blocking(move || {
                let r = media_pass(&d, kind, &s, &mut seen);
                (r, seen)
            })
            .await;
            match joined {
                Ok((r, seen)) => {
                    seen_media = seen;
                    r
                }
                // The pass panicked: the shared set is lost, so start it afresh and say so.
                Err(_) => Ok(MediaOutcome::default()),
            }
        };
        let data = Arc::try_unwrap(data).unwrap_or_else(|a| (*a).clone());

        // Conversion (not for a picture on its own: its text would need OCR, which is off).
        let started = Instant::now();
        let projection = if kind == Kind::Image {
            None
        } else {
            Some(project(data, &ext, &settings.convert).await)
        };
        let convert_ms = started.elapsed().as_secs_f64() * 1000.0;
        let peak = memory_probe.then(probe::peak_rss_kib).flatten();

        let (md_bytes, converted_ok) = match &projection {
            Some(Ok(p)) => {
                acc_class = p.class.to_string();
                (p.md_bytes, true)
            }
            Some(Err(f)) => {
                acc_class = f.class.to_string();
                (0, false)
            }
            None => (0, false),
        };

        let acc = classes.entry(acc_class).or_default();
        acc.stats.files += 1;
        acc.stats.original_bytes += original;
        if let Some(p) = peak {
            let m = acc.stats.peak_rss_kib_max.get_or_insert(0);
            *m = (*m).max(p);
        }
        match &projection {
            Some(Ok(p)) => {
                acc.stats.converted += 1;
                acc.stats.markdown_bytes += p.md_bytes;
                acc.convert_ms.push(convert_ms);
                *acc.stats
                    .tiers
                    .entry(p.fidelity.as_str().to_string())
                    .or_default() += 1;
                for k in &p.loss_kinds {
                    *acc.stats.loss_kinds.entry(loss_label(*k)).or_default() += 1;
                }
            }
            Some(Err(f)) => {
                *acc.stats
                    .failures
                    .entry(f.kind.label().to_string())
                    .or_default() += 1;
                let _: FailureKind = f.kind;
            }
            None => {}
        }

        // A file with no record we can project (a scan, an encrypted file, a failure) is
        // counted but not projected; a failed media pass counts as an unreadable package.
        let media = match media_result {
            Ok(m) => m,
            Err(_) => {
                *acc.stats
                    .failures
                    .entry("media_unreadable".into())
                    .or_default() += 1;
                MediaOutcome::default()
            }
        };
        let projectable = converted_ok || kind == Kind::Image;

        if media.seen_items > 0 {
            acc.stats.media.files_with_media += 1;
        }
        acc.stats.media.items += media.seen_items;
        acc.stats.media.bytes += media.seen_bytes;
        acc.stats.media.duplicate_items += media.duplicate_items;
        acc.stats.media.duplicate_bytes += media.duplicate_bytes;
        acc.stats.media.parts_over_limit += media.parts_over_limit;

        // Per-encoder bytes this file's new pictures would occupy.
        let mut record: BTreeMap<String, u64> =
            labels.iter().map(|l| (l.clone(), md_bytes)).collect();
        for item in &media.items {
            let f = acc
                .stats
                .media
                .by_format
                .entry(item.format.label().to_string())
                .or_default();
            f.items += 1;
            f.bytes += item.original;
            if let Some(why) = item.skipped {
                *acc.stats
                    .media
                    .not_recompressed
                    .entry(why.to_string())
                    .or_default() += 1;
            }
            *record.get_mut(AS_IS).unwrap() += item.original;
            for e in &item.encodes {
                let ea = acc.enc.entry(e.label.clone()).or_default();
                ea.stats.items += 1;
                ea.stats.bytes_in += item.original;
                let kept = match &e.out {
                    Ok(done) => {
                        let out_len = done.bytes.len() as u64;
                        ea.stats.bytes_out += out_len;
                        ea.ms.push(done.took.as_secs_f64() * 1000.0);
                        if let Some(p) = done.psnr_db {
                            ea.psnr.push(p);
                        }
                        if done.alpha_flattened {
                            ea.stats.alpha_flattened += 1;
                        }
                        if out_len >= item.original {
                            ea.stats.kept_original += 1;
                            item.original
                        } else {
                            out_len
                        }
                    }
                    Err(_) => {
                        ea.stats.encode_failures += 1;
                        ea.stats.bytes_out += item.original;
                        item.original
                    }
                };
                *record.get_mut(&e.label).unwrap() += kept;
            }
            // Pictures that were not recompressed cost what they cost under every encoder.
            if item.skipped.is_some() {
                for l in &labels {
                    if l != AS_IS {
                        *record.get_mut(l).unwrap() += item.original;
                    }
                }
            }
        }
        if projectable {
            for (label, bytes) in record {
                let p = acc.proj.entry(label).or_default();
                p.files += 1;
                p.original += original;
                p.record += bytes;
                if original > 0 {
                    p.ratios.push(bytes as f64 / original as f64);
                }
            }
        }
    }
    progress(total, total);

    let caps = capabilities();
    let mut notes = vec![
        "Fidelity tiers are projected from the importer's report. The regenerate check that confirms `safe` arrives with the renderer (M2).".to_string(),
        "Record size is markdown plus pictures stored once. Scans and photos of documents (`pdf-scan`, failures `needs_ocr`) keep page images and are not projected.".to_string(),
        "Compressed sidecars, versions and the search index are not counted.".to_string(),
    ];
    if !memory_probe {
        notes.push("Per-file peak memory could not be read on this system.".to_string());
    }
    Ok(MeasureReport {
        schema: 1,
        host: Host {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            ocr_available: caps.ocr,
            pdf_tables_available: caps.pdf_tables,
            legacy_office_available: caps.legacy_office,
            memory_probe,
        },
        settings: SettingsView {
            encoders: settings.encoders.iter().map(Encoder::label).collect(),
            max_long_edge: settings.max_long_edge,
            max_pixels: settings.max_pixels,
        },
        totals,
        classes: classes.into_iter().map(|(k, v)| (k, v.finish())).collect(),
        notes,
    })
}

fn loss_label(k: LossKind) -> String {
    // The serde name of the kind, such as `charts_omitted`.
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "other".into())
}

impl MeasureReport {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("a report is always serialisable")
    }

    /// A short table for a terminal: one line per class and encoder.
    pub fn summary(&self) -> String {
        let mut out = String::new();
        let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
        out.push_str(&format!(
            "{} files, {:.1} MiB\n",
            self.totals.files,
            mib(self.totals.bytes)
        ));
        out.push_str(&format!(
            "{:<12} {:>6} {:>10} {:>9}  {:<16} {:>10} {:>7}\n",
            "class", "files", "MiB", "read ok", "record under", "MiB", "ratio"
        ));
        for (class, c) in &self.classes {
            if c.projected.is_empty() {
                out.push_str(&format!(
                    "{:<12} {:>6} {:>10.1} {:>9}  (not projected)\n",
                    class,
                    c.files,
                    mib(c.original_bytes),
                    c.converted
                ));
                continue;
            }
            // Without pictures every encoder gives the same record; one line says so.
            let same = c
                .projected
                .values()
                .map(|p| p.record_bytes)
                .collect::<HashSet<_>>()
                .len()
                == 1;
            let rows: Vec<(&String, &Projected)> = if same {
                c.projected.iter().take(1).collect()
            } else {
                c.projected.iter().collect()
            };
            let mut first = true;
            for (label, p) in rows {
                let label = if same { "any encoder" } else { label.as_str() };
                let ratio = p
                    .ratio
                    .as_ref()
                    .map_or("-".to_string(), |r| format!("{:.2}", r.p50));
                if first {
                    out.push_str(&format!(
                        "{:<12} {:>6} {:>10.1} {:>9}  ",
                        class,
                        c.files,
                        mib(c.original_bytes),
                        c.converted
                    ));
                    first = false;
                } else {
                    out.push_str(&format!("{:<12} {:>6} {:>10} {:>9}  ", "", "", "", ""));
                }
                out.push_str(&format!(
                    "{:<16} {:>10.1} {:>7}\n",
                    label,
                    mib(p.record_bytes),
                    ratio
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat as Fmt, RgbImage};
    use std::io::{Cursor, Write};

    fn photo(w: u32, h: u32, fmt: Fmt) -> Vec<u8> {
        let mut img = RgbImage::new(w, h);
        for (x, y, p) in img.enumerate_pixels_mut() {
            let n = ((x * 31 + y * 17) % 29) as u8;
            *p = image::Rgb([(x % 251) as u8 ^ n, (y % 241) as u8, ((x + y) % 233) as u8]);
        }
        let mut out = Vec::new();
        DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), fmt)
            .unwrap();
        out
    }

    fn zip_of(parts: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
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

    const TYPES: &[u8] = br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;

    fn docx_with(text: &str, pics: &[&[u8]]) -> Vec<u8> {
        let doc = format!(
            r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
        );
        let names: Vec<String> = (0..pics.len())
            .map(|i| format!("word/media/image{i}.jpeg"))
            .collect();
        let mut parts: Vec<(&str, &[u8])> = vec![
            ("[Content_Types].xml", TYPES),
            ("word/document.xml", doc.as_bytes()),
        ];
        for (n, p) in names.iter().zip(pics) {
            parts.push((n.as_str(), p));
        }
        zip_of(&parts)
    }

    fn settings() -> Settings {
        Settings {
            encoders: vec![Encoder::Jpeg { quality: 60 }, Encoder::WebpLossless],
            max_long_edge: 100,
            ..Settings::default()
        }
    }

    #[tokio::test]
    async fn measures_a_small_folder_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let big_jpeg = photo(300, 200, Fmt::Jpeg);
        std::fs::write(
            dir.path().join("secret-plan.docx"),
            docx_with("SECRET-CONTENT-MARKER", &[&big_jpeg]),
        )
        .unwrap();
        // The same picture inside a second document is stored once.
        std::fs::write(
            dir.path().join("copy.docx"),
            docx_with("another", &[&big_jpeg]),
        )
        .unwrap();
        // An identical file costs nothing.
        std::fs::write(
            dir.path().join("copy-of-copy.docx"),
            docx_with("another", &[&big_jpeg]),
        )
        .unwrap();
        std::fs::write(dir.path().join("photo.png"), photo(240, 160, Fmt::Png)).unwrap();
        std::fs::write(dir.path().join("notes.csv"), "a,b\n1,2\n3,4\n").unwrap();
        std::fs::create_dir(dir.path().join("sub-folder-name")).unwrap();
        std::fs::write(
            dir.path().join("sub-folder-name/zzz.bin"),
            b"\x00\x01 unknown",
        )
        .unwrap();

        let report = measure(dir.path(), &settings(), |_, _| {}).await.unwrap();
        assert_eq!(report.totals.files, 6);

        let docx = &report.classes["docx"];
        assert_eq!(docx.files, 3);
        assert_eq!(docx.duplicate_files, 1);
        assert_eq!(docx.converted, 2);
        assert_eq!(
            docx.media.items, 2,
            "the duplicate file's pictures are not read twice"
        );
        assert_eq!(docx.media.duplicate_items, 1);
        let jpeg = &docx.encoders["jpeg-q60"];
        assert_eq!(jpeg.items, 1);
        assert!(jpeg.bytes_out < jpeg.bytes_in, "{jpeg:?}");
        assert!(jpeg.psnr_db_mean.unwrap() > 20.0);
        let asis = &docx.projected[AS_IS];
        let small = &docx.projected["jpeg-q60"];
        assert!(small.record_bytes < asis.record_bytes);
        assert_eq!(asis.files, 3, "duplicates are projected too, at nothing");

        let image = &report.classes["image"];
        assert_eq!(image.files, 1);
        assert!(image.projected["webp-lossless"].record_bytes > 0);

        assert_eq!(report.classes["csv"].converted, 1);
        assert!(
            report.classes.values().any(|c| !c.failures.is_empty()),
            "the unknown file is a counted failure"
        );
    }

    #[tokio::test]
    async fn the_report_holds_no_names_paths_or_content() {
        let dir = tempfile::tempdir().unwrap();
        let big = photo(120, 120, Fmt::Jpeg);
        std::fs::write(
            dir.path().join("my-private-statement.docx"),
            docx_with("SECRET-CONTENT-MARKER", &[&big]),
        )
        .unwrap();
        std::fs::create_dir(dir.path().join("hidden-folder-name")).unwrap();
        std::fs::write(
            dir.path().join("hidden-folder-name/other-secret.csv"),
            "x,y\nSECRET-CELL,2\n",
        )
        .unwrap();
        let report = measure(dir.path(), &settings(), |_, _| {}).await.unwrap();
        let json = report.to_json();
        let text = report.summary();
        let hash = hex::encode(Sha256::digest(&big));
        for needle in [
            "private-statement",
            "hidden-folder-name",
            "other-secret",
            "SECRET-CONTENT-MARKER",
            "SECRET-CELL",
            dir.path().to_str().unwrap(),
            &hash,
            "word/media",
        ] {
            assert!(
                !json.contains(needle) && !text.contains(needle),
                "leaked {needle:?}"
            );
        }
    }

    #[tokio::test]
    async fn caps_and_oddities_are_counted_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("f{i}.csv")), format!("a\n{i}\n")).unwrap();
        }
        std::fs::write(dir.path().join("big.txt"), vec![b'x'; 5000]).unwrap();
        let s = Settings {
            max_files: 4,
            max_file_bytes: 1000,
            ..settings()
        };
        let r = measure(dir.path(), &s, |_, _| {}).await.unwrap();
        assert_eq!(r.totals.files_over_count_cap, 2);
        assert!(r.totals.files + r.totals.files_over_size_cap <= 4);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
            let r = measure(dir.path(), &settings(), |_, _| {}).await.unwrap();
            assert_eq!(r.totals.folders_skipped_symlink, 1);
        }
        assert!(matches!(
            measure(&dir.path().join("f0.csv"), &settings(), |_, _| {}).await,
            Err(MeasureError::NotAFolder(_))
        ));
    }

    #[tokio::test]
    async fn a_hostile_picture_is_refused_by_the_pixel_cap() {
        let dir = tempfile::tempdir().unwrap();
        let big = photo(300, 300, Fmt::Png);
        std::fs::write(dir.path().join("a.png"), &big).unwrap();
        let s = Settings {
            max_pixels: 1000,
            ..settings()
        };
        let r = measure(dir.path(), &s, |_, _| {}).await.unwrap();
        let c = &r.classes["image"];
        assert_eq!(c.media.not_recompressed["too_many_pixels"], 1);
        // Not decoded means kept as it is under every encoder.
        assert_eq!(c.projected["jpeg-q60"].record_bytes, big.len() as u64);
    }

    #[tokio::test]
    async fn two_runs_agree_apart_from_timings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.docx"),
            docx_with("one", &[&photo(150, 100, Fmt::Jpeg)]),
        )
        .unwrap();
        std::fs::write(dir.path().join("b.csv"), "a\n1\n").unwrap();
        let strip = |mut r: MeasureReport| {
            for c in r.classes.values_mut() {
                c.convert_ms = None;
                c.peak_rss_kib_max = None;
                for e in c.encoders.values_mut() {
                    e.encode_ms = None;
                }
            }
            r
        };
        let a = strip(measure(dir.path(), &settings(), |_, _| {}).await.unwrap());
        let b = strip(measure(dir.path(), &settings(), |_, _| {}).await.unwrap());
        assert_eq!(a, b);
    }

    #[test]
    fn spreads_and_quantiles_are_sane() {
        assert!(spread(vec![]).is_none());
        let s = spread(vec![5.0, 1.0, 3.0, 2.0, 4.0]).unwrap();
        assert_eq!((s.p50, s.max), (3.0, 5.0));
        let q = quantiles(vec![0.9, 0.1, 0.5]).unwrap();
        assert_eq!((q.p10, q.p50, q.p90), (0.1, 0.5, 0.9));
    }
}
