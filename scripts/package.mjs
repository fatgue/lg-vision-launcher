// Builds the TV binary and packs a webOS IPK (ar: debian-binary, control.tar.gz, data.tar.gz).
// Hand-written tar/ar so file modes are exact on Windows (the executable must be 0755).
import { execFileSync } from 'node:child_process';
import { gzipSync } from 'node:zlib';
import { readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const appinfo = JSON.parse(readFileSync(join(root, 'appinfo.json'), 'utf8'));
const { id, version } = appinfo;
const target = 'armv7-unknown-linux-gnueabi';
const skipBuild = process.argv.includes('--no-build');

if (!skipBuild) {
  execFileSync('cargo', ['zigbuild', '--release', '--target', `${target}.2.28`], { cwd: root, stdio: 'inherit' });
}

const appDir = `usr/palm/applications/${id}`;
/** [path inside the package, local source or Buffer, mode] */
const files = [
  [`${appDir}/vision`, join(root, 'target', target, 'release', 'vision'), 0o755],
  [`${appDir}/appinfo.json`, join(root, 'appinfo.json'), 0o644],
  [`${appDir}/icon.png`, join(root, 'branding', 'icon.png'), 0o644],
  [`${appDir}/icon-large.png`, join(root, 'branding', 'icon-large.png'), 0o644],
  [`${appDir}/splash.png`, join(root, 'branding', 'splash.png'), 0o644],
  [`${appDir}/home.sh`, join(root, 'tv', 'home.sh'), 0o755],
  [`${appDir}/lib/libSDL2-2.0.so.0`, join(root, 'third_party', 'sdl-webos', 'libSDL2-2.0.so.0'), 0o755],
  [`${appDir}/licenses/SDL2.txt`, join(root, 'third_party', 'sdl-webos', 'LICENSE-SDL2.txt'), 0o644],
  [`${appDir}/licenses/Outfit-OFL.txt`, join(root, 'assets', 'fonts', 'OFL.txt'), 0o644],
  [`${appDir}/licenses/Inter-OFL.txt`, join(root, 'assets', 'fonts', 'Inter-OFL.txt'), 0o644],
  [
    `usr/palm/packages/${id}/packageinfo.json`,
    Buffer.from(JSON.stringify({ app: id, id, loc_name: appinfo.title, package_format_version: 2, vendor: appinfo.vendor, version }, null, 2)),
    0o644,
  ],
];
for (const font of readdirSync(join(root, 'assets', 'fonts')).filter((f) => f.endsWith('.ttf'))) {
  files.push([`${appDir}/assets/fonts/${font}`, join(root, 'assets', 'fonts', font), 0o644]);
}
for (const wallpaper of readdirSync(join(root, 'assets', 'wallpapers')).filter((f) => f.endsWith('.jpg'))) {
  files.push([`${appDir}/assets/wallpapers/${wallpaper}`, join(root, 'assets', 'wallpapers', wallpaper), 0o644]);
}
files.push([`${appDir}/licenses/wallpapers.txt`, join(root, 'assets', 'wallpapers', 'CREDITS.txt'), 0o644]);

function tarHeader(name, size, mode, type) {
  const h = Buffer.alloc(512);
  const put = (value, offset, length) => h.write(value, offset, length, 'utf8');
  const octal = (value, length) => value.toString(8).padStart(length - 1, '0') + '\0';
  if (Buffer.byteLength(name) > 99) throw new Error(`tar path too long: ${name}`);
  put(name, 0, 100);
  put(octal(mode, 8), 100, 8);
  put(octal(0, 8), 108, 8);
  put(octal(0, 8), 116, 8);
  put(octal(size, 12), 124, 12);
  put(octal(Math.floor(Date.now() / 1000), 12), 136, 12);
  h.fill(' ', 148, 156);
  put(type, 156, 1);
  put('ustar\0', 257, 6);
  put('00', 263, 2);
  put('root', 265, 32);
  put('root', 297, 32);
  let sum = 0;
  for (const b of h) sum += b;
  put(sum.toString(8).padStart(6, '0') + '\0 ', 148, 8);
  return h;
}

function tar(entries) {
  const parts = [];
  const dirs = new Set();
  for (const [path] of entries) {
    const segments = path.split('/');
    for (let i = 1; i < segments.length; i++) dirs.add(segments.slice(0, i).join('/'));
  }
  for (const dir of [...dirs].sort()) parts.push(tarHeader(`./${dir}/`, 0, 0o755, '5'));
  for (const [path, data, mode] of entries) {
    parts.push(tarHeader(`./${path}`, data.length, mode, '0'), data);
    const pad = (512 - (data.length % 512)) % 512;
    if (pad) parts.push(Buffer.alloc(pad));
  }
  parts.push(Buffer.alloc(1024));
  return gzipSync(Buffer.concat(parts), { level: 9 });
}

function ar(members) {
  const parts = [Buffer.from('!<arch>\n')];
  const mtime = Math.floor(Date.now() / 1000);
  for (const [name, data] of members) {
    const header =
      name.padEnd(16) + String(mtime).padEnd(12) + '0'.padEnd(6) + '0'.padEnd(6) + '100644'.padEnd(8) + String(data.length).padEnd(10) + '`\n';
    parts.push(Buffer.from(header, 'ascii'), data);
    if (data.length % 2) parts.push(Buffer.from('\n'));
  }
  return Buffer.concat(parts);
}

const entries = files.map(([path, source, mode]) => [path, Buffer.isBuffer(source) ? source : readFileSync(source), mode]);
const installedSize = entries.reduce((sum, [, data]) => sum + data.length, 0);
const control = [
  `Package: ${id}`,
  `Version: ${version}`,
  'Section: misc',
  'Priority: optional',
  'Architecture: arm',
  `Installed-Size: ${installedSize}`,
  `Maintainer: ${appinfo.vendor}`,
  `Description: ${appinfo.title}`,
  'webOS-Package-Format-Version: 2',
  'webOS-Packager-Version: x.y.x',
  '',
].join('\n');

const ipk = ar([
  ['debian-binary', Buffer.from('2.0\n')],
  ['control.tar.gz', tar([['control', Buffer.from(control), 0o644]])],
  ['data.tar.gz', tar(entries)],
]);
mkdirSync(join(root, 'out'), { recursive: true });
const out = join(root, 'out', `${id}_${version}_arm.ipk`);
writeFileSync(out, ipk);
console.log(`${relative(root, out).split(sep).join('/')}  (${(ipk.length / 1024).toFixed(0)} KB, binary ${(statSync(files[0][1]).size / 1024).toFixed(0)} KB)`);
