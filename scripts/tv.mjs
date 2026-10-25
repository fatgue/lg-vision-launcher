// TV helper over PuTTY (plink/pscp):
//   node scripts/tv.mjs install   upload out/*.ipk and install it as a dev app
//   node scripts/tv.mjs launch    start Vision
//   node scripts/tv.mjs log       print /tmp/vision.log
//   node scripts/tv.mjs shot      capture the UI plane to out/tv.jpg
//   node scripts/tv.mjs home-enable | home-disable | home-status   Vision as the Home launcher
// Host/password come from VISION_TV_HOST / VISION_TV_PASSWORD (defaults: 192.168.178.96 / alpine).
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const host = process.env.VISION_TV_HOST ?? '192.168.178.96';
const password = process.env.VISION_TV_PASSWORD ?? 'alpine';
const { id, version } = JSON.parse(readFileSync(join(root, 'appinfo.json'), 'utf8'));
const putty = process.platform === 'win32' ? 'C:\\Program Files\\PuTTY\\' : '';

const ssh = (command) =>
  execFileSync(`${putty}plink`, ['-ssh', '-pw', password, '-batch', `root@${host}`, command], { encoding: 'utf8' });
const sleep = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);

function install() {
  const ipk = join(root, 'out', `${id}_${version}_arm.ipk`);
  execFileSync(`${putty}pscp`, ['-pw', password, '-batch', '-q', ipk, `root@${host}:/tmp/vision.ipk`], { stdio: 'inherit' });
  // luna-send -i keeps the subscription open; poll its output for the final state instead.
  ssh(
    `rm -f /tmp/vision-install.txt; (luna-send -i -w 120000 luna://com.webos.appInstallService/dev/install '{"id":"com.ares.defaultName","ipkUrl":"/tmp/vision.ipk","subscribe":true}' > /tmp/vision-install.txt 2>&1 &)`,
  );
  for (let i = 0; i < 90; i++) {
    sleep(1000);
    const out = ssh('cat /tmp/vision-install.txt 2>/dev/null');
    if (/"state"\s*:\s*"installed"/.test(out)) {
      ssh('killall luna-send 2>/dev/null; rm -f /tmp/vision.ipk');
      // While Vision is Home, the mount still holds the previous binary: re-apply it.
      ssh('[ -x /var/lib/vision-home/home.sh ] && sh /var/lib/vision-home/home.sh refresh; true');
      console.log(`installed ${id} ${version}`);
      return;
    }
    if (/FAILED|"returnValue"\s*:\s*false/.test(out)) {
      ssh('killall luna-send 2>/dev/null');
      throw new Error(`install failed:\n${out}`);
    }
  }
  throw new Error('install timed out');
}

const commands = {
  install,
  launch: () => {
    ssh(`luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"${id}"}' </dev/null`);
    sleep(3000);
    ssh('rm -f /tmp/vision-args');
  },
  // node scripts/tv.mjs bench  → scripted navigation with frame timings in the log
  bench: () => {
    ssh(`echo "--perf --frames 420 --keys right,right,right,right,right,right,right,right,down,right,right,up,left,left,left,left" > /tmp/vision-args`);
    // SAM's close does not match the jailed dev process; end it directly.
    ssh('kill $(pidof vision) 2>/dev/null; sleep 1');
    commands.launch();
    sleep(12000);
    commands.log();
  },
  'home-enable': () => console.log(ssh(`sh /media/developer/apps/usr/palm/applications/${id}/home.sh enable`)),
  'home-disable': () => console.log(ssh('sh /var/lib/vision-home/home.sh disable')),
  'home-status': () => console.log(ssh('[ -x /var/lib/vision-home/home.sh ] && sh /var/lib/vision-home/home.sh status || echo "enabled: no"')),
  log: () => console.log(ssh('cat /tmp/vision.log 2>/dev/null || echo "(no log yet)"')),
  shot: () => {
    ssh(
      `rm -f /tmp/vision-shot.jpg; (luna-send -i -w 4000 luna://com.webos.service.capture/executeOneShot '{"path":"/tmp/vision-shot.jpg","method":"DISPLAY","format":"JPG"}' >/dev/null 2>&1 &); sleep 3; killall luna-send 2>/dev/null; ls /tmp/vision-shot.jpg`,
    );
    execFileSync(`${putty}pscp`, ['-pw', password, '-batch', '-q', `root@${host}:/tmp/vision-shot.jpg`, join(root, 'out', 'tv.jpg')], {
      stdio: 'inherit',
    });
    console.log('out/tv.jpg');
  },
};

const command = process.argv[2];
if (!commands[command]) {
  console.error(`usage: node scripts/tv.mjs ${Object.keys(commands).join('|')}`);
  process.exit(1);
}
commands[command]();
