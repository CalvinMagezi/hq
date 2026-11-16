# Measuring real files

Before the shrink design is locked, the suite needs real numbers: how much do real document
folders shrink, which picture encoder wins, and what does it cost in time and memory on a cheap
phone. `hq suite measure` produces those numbers without writing anything or sending anything.

## Run it

```bash
hq suite measure ~/Downloads/documents --out measure-1.json
```

The folder is read, including its sub-folders. Links are not followed. Nothing is written
except the report file you name with `--out`, which must not exist yet. Nothing leaves the
machine: the tool has no network code, and a test fails if the report can contain a file name,
a path, document text, picture content, a hash or the size of a single file.

Useful options:

| Option | Meaning |
|--------|---------|
| `--encoder NAME[:Q]` | A picture encoder to try. Repeat for several: `--encoder jpeg:60 --encoder jpeg:75 --encoder webp-lossless`. Without it, every encoder this build has is tried. |
| `--max-long-edge N` | Scale pictures down to this long edge before encoding, as shrinking would. Default 1600. |
| `--max-pixels N` | Do not decode a picture with more pixels than this; it is counted instead. Default 50 million. |
| `--max-files N` | Look at no more than this many files. Default 5000. |
| `--json` | Print the JSON report instead of the table. |

Encoders: `jpeg:Q`, `png`, `webp-lossless` and `webp:Q` (lossy WebP through libwebp, the encoder the
shrink uses for photographs, at quality 80 by default).

## Reading the report

The table has one block per format class (`docx`, `xlsx`, `pptx`, `pdf-text`, `pdf-scan`,
`image`, ...). For each class and each encoder it shows what the projected record would take
and the typical (median) ratio of record to original per file. `as-is` is the baseline: the
same record with pictures left exactly as they are, so it shows what deduplication and
markdown alone save.

The JSON has more:

- `duplicate_files`: files identical to one already seen. They cost nothing in the record.
- `media`: pictures found inside documents, how many were repeats, what was not recompressed
  and why (`not_recompressible`, `too_many_pixels`, `unreadable`).
- `encoders`: per encoder, bytes in and out, how many pictures it could not make smaller (the
  original would be kept), encode time (median, 95th percentile, slowest), and quality as peak
  signal to noise ratio in dB (about 40 and up is hard to tell from the original; under 30 is
  visibly worse). AVIF has no quality number because this build cannot read AVIF back.
- `tiers`: the projected fidelity tier of each converted file. This is a projection from the
  importer's report; the check that makes a tier `safe` for certain comes with the renderer.
- `failures`: files the importer could not read, by reason. `needs_ocr` are scans and photos of
  documents; they keep their page images and are not projected.
- `peak_rss_kib_max`: the most memory the process held while handling a file of that class
  (Linux and Android only).

What the numbers do not include: compressed sheet sidecars, version history and the search
index. A measurement is a decision input, not a promise to a user.

## On the reference phone

The numbers that decide the design are the phone's: encode time and peak memory on a
Samsung Galaxy A32 (Helio G80, about 2 GB free). Run the same command there, through the
`hq` binary the app ships, on a folder of copied files, Record each run as an evidence note beside the earlier device spikes: device,
build, the table, and what was not covered.

## The encoder decision

Three lossy candidates were built and measured on 255 real pictures (282 MB, all kinds of
photographs and screenshots), scaled to 1600 pixels on the long edge, on an Apple Silicon laptop:

| Encoder | Bytes out | Share of JPEG 75 | PSNR mean / worst | Median encode |
|---------|-----------|------------------|-------------------|---------------|
| jpeg q75 (the old default) | 43.9 MB | 100% | 40.0 / 31.2 dB | 8 ms |
| webp (libwebp) q75 | 24.7 MB | 56% | 37.7 / 32.0 dB | 51 ms |
| webp (libwebp) q80 | 29.8 MB | 68% | 39.0 / 33.6 dB | 53 ms |
| webp (libwebp) q85 | 36.5 MB | 83% | 40.5 / 35.1 dB | 55 ms |
| webp pure Rust q75 | 24.7 MB | 56% | 37.0 / 31.4 dB | 118 ms |
| webp pure Rust q85 | 36.0 MB | 82% | 39.4 / 34.0 dB | 139 ms |
| avif q55 | 15.2 MB | 35% | not measurable | 92 ms |

Chosen: lossy WebP through libwebp at quality 80. At a quality score that matches JPEG 75 it is
17% smaller with a better worst case; at the same size as pure-Rust WebP it scores higher and
encodes about two and a half times faster. AVIF is smaller still, but this build cannot decode it
to verify a record, Word and Google's apps do not take it in a document, and it costs more time
and memory per picture on a phone. Pure-Rust WebP is a young crate. Graphics (PNG and WebP
sources) stay lossless.

An earlier run printed a worst-case PSNR near zero for both WebP encoders. That was the measuring
tool: the decoded picture was compared without the white it was flattened onto, so transparent
pixels counted as errors. It is fixed, and a test keeps it fixed.

Reading the per-encoder columns: they encode every picture lossily, while the policy is lossy for
photographs only, so they are upper bounds on what the policy saves.

## The measurement table

Four sets of the owner's own files, read with `hq suite measure`. The projection is the record
under lossy WebP q75 against the original, over every class the tool can project; scans are
excluded as they are not projected. Text-only PDFs project very small because the record keeps
text and tables, not layout, and they are in the lossy tier.

| File set | Files | Original MiB | Record MiB | Saved | Tier mix | Notes |
|----------|-------|--------------|------------|-------|----------|-------|
| A: a downloads folder | 484 | 475 | 59 | 88% | 145 safe, 17 lossy | Half of it is pictures; 17 files were not projected (archives, unknown) |
| B: a work folder of reports | 62 | 46 | 2.8 | 94% | 49 safe, 7 lossy | Mostly docx and decks |
| C: a folder of mixed documents | 157 | 89 | 9.8 | 89% | 109 safe | Many PDFs; docx files barely shrink (0.9 and up) |
| D: a project archive | 395 | 335 | 24 | 93% | 342 safe, 12 lossy | Decks with embedded pictures dominate the saving |

Before recompression (`as-is`, markdown and deduplication alone) the same sets save 32%, 86%, 83%
and 58%. The picture encoder is where the big decks and downloads folders come from.

Not yet measured: sets from other people, with consent; the reference phone's own table (see the
evidence note beside the earlier device spikes).
