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

Encoders in every build: `jpeg:Q`, `png`, `webp-lossless`. Three more are candidates and are
built only to be measured. Build with `--features suite-candidates`:

| Encoder | What it is |
|---------|------------|
| `webp-pure:Q` | Lossy WebP in pure Rust (`oxideav-webp`). A young crate. |
| `webp-lib:Q` | Lossy WebP through libwebp (C). The reference for size and quality. |
| `avif:Q` | AVIF through rav1e. Slow on a phone CPU. |

```bash
cargo build --release -p hq-cli --features suite-candidates
```

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
`hq` binary the app ships, on a folder of copied files, with a candidate build if you want the
extra encoders. Record each run as an evidence note beside the earlier device spikes: device,
build, the table, and what was not covered.

## The measurement table

| File set | Files | Original bytes | Record bytes | Saved | Tier mix | Notes |
|----------|-------|----------------|--------------|-------|----------|-------|

Rows are added as real file sets are measured. Until then the design uses no ratio as fact.
