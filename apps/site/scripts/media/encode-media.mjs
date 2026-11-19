// Encodes one recording for the site and registers it in src/data/media.json.
//   bun scripts/media/encode-media.mjs <input> <id> <guide|loop> --alt "<text>" --caption "<text>" \
//     [--build v0.9.1-main.598] [--captions <file.vtt>] [--vertical]
// A guide clip is at most 3 MB with sound, a loop at most 600 KB and silent. Over budget is refused
// and nothing is written, so a heavy clip never reaches a visitor paying for data.
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { basename, join } from 'node:path';

const KB = 1024;
const MB = KB * KB;
const BUDGETS = { guide: { bytes: 3 * MB, maxSeconds: 90, audio: true }, loop: { bytes: 600 * KB, maxSeconds: 10, audio: false } };
const WIDTHS = { vertical: 480, wide: 1280 };
const CRF = 28;
const SITE = new URL('../..', import.meta.url).pathname;
const MEDIA_DIR = join(SITE, 'public', 'media');
const MANIFEST = join(SITE, 'src', 'data', 'media.json');
const BUILD_FORMAT = /^v\d+\.\d+\.\d+-main\.\d+$/;

function arg(name, fallback) {
  const i = process.argv.indexOf(`--${name}`);
  return i === -1 ? fallback : process.argv[i + 1];
}
const flag = (name) => process.argv.includes(`--${name}`);
function die(message) {
  console.error(message);
  process.exit(1);
}
function run(cmd, args) {
  const r = spawnSync(cmd, args, { encoding: 'utf8' });
  if (r.status !== 0) die(`${cmd} failed:\n${r.stderr.split('\n').slice(-8).join('\n')}`);
  return r.stdout;
}

const [input, id, kind] = process.argv.slice(2);
if (!input || !id || !BUDGETS[kind]) die('usage: encode-media.mjs <input> <id> <guide|loop> --alt "..." --caption "..." [--build vX] [--captions file.vtt] [--vertical]');
if (!existsSync(input)) die(`no such file: ${input}`);
const alt = arg('alt');
const caption = arg('caption');
const build = arg('build');
if (!alt || alt.length < 15) die('--alt is required and must describe the clip (15 characters or more)');
if (!caption) die('--caption is required');
if (!build || !BUILD_FORMAT.test(build)) die('--build must be the build the clip was recorded on, for example v0.9.1-main.598');
const budget = BUDGETS[kind];
const captions = arg('captions');
if (budget.audio && !captions) die('a guide clip needs --captions <file.vtt>, because many people watch muted');
if (captions && !existsSync(captions)) die(`no such captions file: ${captions}`);

const seconds = Number(run('ffprobe', ['-v', 'error', '-show_entries', 'format=duration', '-of', 'csv=p=0', input]));
if (seconds > budget.maxSeconds) die(`${seconds.toFixed(1)} s is over the ${budget.maxSeconds} s limit for a ${kind}`);

mkdirSync(MEDIA_DIR, { recursive: true });
const out = join(MEDIA_DIR, `${id}.mp4`);
const posterFile = join(MEDIA_DIR, `${id}-poster.jpg`);
const width = flag('vertical') ? WIDTHS.vertical : WIDTHS.wide;
const audio = budget.audio ? ['-c:a', 'aac', '-b:a', '64k', '-ac', '1'] : ['-an'];
run('ffmpeg', ['-y', '-i', input, '-vf', `scale='min(${width},iw)':-2`, '-c:v', 'libx264', '-preset', 'slow', '-crf', String(CRF), '-pix_fmt', 'yuv420p', '-movflags', '+faststart', ...audio, out]);
const bytes = statSync(out).size;
if (bytes > budget.bytes) {
  rmSync(out);
  die(`${(bytes / MB).toFixed(2)} MB is over the ${(budget.bytes / MB).toFixed(2)} MB budget for a ${kind}. Cut it shorter or lower the resolution; nothing was written.`);
}
run('ffmpeg', ['-y', '-ss', String(Math.min(1, seconds / 2)), '-i', out, '-frames:v', '1', '-q:v', '4', posterFile]);
const [w, h] = run('ffprobe', ['-v', 'error', '-select_streams', 'v:0', '-show_entries', 'stream=width,height', '-of', 'csv=p=0:s=x', out]).trim().split('x').map(Number);
if (captions) copyFileSync(captions, join(MEDIA_DIR, `${id}.vtt`));

const manifest = JSON.parse(readFileSync(MANIFEST, 'utf8'));
const entry = {
  id, kind: 'video', src: `/media/${id}.mp4`, poster: `/media/${id}-poster.jpg`, alt, caption, width: w, height: h,
  recordedWith: build, bytes, duration: Math.round(seconds), ...(captions ? { captions: `/media/${id}.vtt` } : {}),
};
const at = manifest.findIndex((m) => m.id === id);
if (at === -1) manifest.push(entry);
else manifest[at] = entry;
writeFileSync(MANIFEST, `${JSON.stringify(manifest, null, 2)}\n`);

console.log(`${basename(out)}  ${(bytes / KB).toFixed(0)} KB of ${(budget.bytes / KB).toFixed(0)} KB  ${seconds.toFixed(1)} s  ${w}x${h}  recorded on ${build}`);
