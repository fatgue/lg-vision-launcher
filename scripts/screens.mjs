// Reproducible desktop screenshots of fixed scenes (fixed clock, fixed frame time, sample data,
// a fresh data directory per scene), for checking that a refactor changes no pixel.
//   node scripts/screens.mjs <out dir> [--compare <baseline dir>] [--look <name>] [--only a,b]
// Needs a debug build (cargo build) with SDL2.dll next to it.
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const exe = join(root, 'target', 'debug', process.platform === 'win32' ? 'vision.exe' : 'vision');
const arg = (name) => (process.argv.includes(name) ? process.argv[process.argv.indexOf(name) + 1] : null);
const out = process.argv[2];
const baseline = arg('--compare');
const look = arg('--look');
const only = arg('--only')?.split(',');

const scenes = {
  home: '',
  'favorites-right': 'right,right,right',
  'rows-down': 'down',
  'rows-bottom': 'down,down,down,right',
  menu: 'hold',
  settings: 'up,ok',
  'settings-items': 'up,ok,right,down',
  'settings-appearance': 'up,ok,down,down,down,down,down,right',
  header: 'up',
  tivione: 'right,right',
  'shelf-open': 'right,right,up',
  'shelf-strip': 'right,right,up,right,right,right,right,right,right,right',
  move: 'hold,ok,down,right',
  bottom: 'down,down,down,down',
};

mkdirSync(out, { recursive: true });
let differ = 0;
for (const [name, keys] of Object.entries(scenes)) {
  if (only && !only.includes(name)) continue;
  const data = mkdtempSync(join(tmpdir(), 'vision-screens-'));
  const path = join(out, `${name}.png`);
  const args = ['--now', '1761400800000', '--fixed-dt', '0.02', '--frames', '240', '--releases', 'http://127.0.0.1:9/none', '--screenshot', path];
  if (keys) args.push('--keys', keys);
  if (look) args.push('--look', look);
  execFileSync(exe, args, { env: { ...process.env, APPDATA: data, HOME: data }, stdio: 'ignore', cwd: root, timeout: 60_000 });
  rmSync(data, { recursive: true, force: true });
  if (baseline) {
    const reference = join(baseline, `${name}.png`);
    const same = existsSync(reference) && readFileSync(reference).equals(readFileSync(path));
    if (!same) differ++;
    console.log(`${same ? 'same   ' : 'DIFFERS'} ${name}`);
  } else {
    console.log(`wrote  ${name}`);
  }
}
if (baseline) {
  console.log(differ ? `${differ} scene(s) differ` : 'all scenes identical');
  process.exitCode = differ ? 1 : 0;
}
