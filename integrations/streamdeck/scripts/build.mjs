import { build } from 'esbuild';
import { mkdir, copyFile, writeFile, readFile, readdir, rm } from 'node:fs/promises';
import { resolve, dirname, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { specs } from '../src/model.mjs';

const root = 'com.piratewave.streamdeck.sdPlugin';
const buildRoot = resolve(root);
const projectRoot = resolve(fileURLToPath(new URL('..', import.meta.url)));
if (dirname(buildRoot) !== projectRoot || basename(buildRoot) !== root) throw new Error('Refusing to clean outside the dedicated plugin build directory');
await rm(buildRoot, { recursive: true, force: true });
await mkdir(`${root}/bin`, { recursive: true });
await mkdir(`${root}/ui`, { recursive: true });
await mkdir(`${root}/images`, { recursive: true });
const bundled = await build({ entryPoints: ['src/plugin.ts'], bundle: true, platform: 'node', target: 'node24', format: 'esm', metafile: true, outfile: `${root}/bin/plugin.mjs`,
  // SDK dependencies use createRequire from their CommonJS implementation.
  banner: { js: "import { createRequire } from 'node:module'; const require = createRequire(import.meta.url);" } });
const packages = new Set(Object.keys(bundled.metafile.inputs).map(path => path.replaceAll('\\', '/').match(/^node_modules\/((?:@[^/]+\/)?[^/]+)\//)?.[1]).filter(Boolean));
let notices = 'Third-party packages bundled with PirateWave Stream Deck\n';
for (const name of [...packages].sort()) {
  const directory = `node_modules/${name}`;
  const pkg = JSON.parse(await readFile(`${directory}/package.json`, 'utf8'));
  const licenses = (await readdir(directory)).filter(file => /^(LICENSE|LICENCE|COPYING)(\.|$)/i.test(file));
  if (!licenses.length) throw new Error(`Missing license text for ${name}`);
  notices += `\n${name} ${pkg.version} — ${pkg.license}\n`;
  for (const file of licenses) notices += await readFile(`${directory}/${file}`, 'utf8') + '\n';
}
await writeFile(`${root}/THIRD_PARTY_NOTICES.txt`, notices);
await copyFile('../../LICENSE', `${root}/LICENSE`);
await build({ entryPoints: ['src/actions.ts'], bundle: true, platform: 'node', target: 'node24', format: 'esm', external: ['@elgato/streamdeck'], outfile: '.test-build/actions.mjs' });
for (const file of ['index.html', 'style.css', 'inspector.mjs']) await copyFile(`ui/${file}`, `${root}/ui/${file}`);
await copyFile('src/model.mjs', `${root}/ui/model.mjs`);
for (const [name, size] of [['plugin',256],['plugin@2x',512],['category',28],['category@2x',56],['action',20],['action@2x',40],['key',72],['key@2x',144]]) {
  await copyFile(`../../assets/branding/piratewave-${size}.png`, `${root}/images/${name}.png`);
}
const manifest = {
  Name: 'PirateWave Audio Mixer', UUID: 'com.piratewave.streamdeck', Version: '1.1.0.0', Author: 'PirateWave contributors',
  Description: 'Control PirateWave audio nodes, effects, soundboard and recording slots using buttons and dials.',
  Icon: 'images/plugin', Category: 'PirateWave Audio Mixer', CategoryIcon: 'images/category', URL: 'https://github.com/MitchCodes/PirateWave',
  CodePath: 'bin/plugin.mjs', Nodejs: { Version: '24' }, SDKVersion: 2,
  OS: [{ Platform: 'windows', MinimumVersion: '10' }], Software: { MinimumVersion: '7.1' },
  Actions: specs.map(spec => ({ UUID: `com.piratewave.streamdeck.${spec.id}`, Name: spec.name, Icon: 'images/action',
    Tooltip: spec.adjustment ? 'Adjust with a dial or configure an increase/decrease/reset button.' : 'Execute a PirateWave action without a keyboard shortcut.',
    Controllers: spec.adjustment ? ['Keypad', 'Encoder'] : ['Keypad'], PropertyInspectorPath: 'ui/index.html',
    DisableAutomaticStates: true, States: [{ Image: 'images/key', TitleAlignment: 'middle', FontSize: 12 }],
    ...(spec.adjustment ? { Encoder: { layout: '$B1', TriggerDescription: { Rotate: 'Adjust value', Push: 'Mute, center or reset', Touch: 'Mute, center or reset' } } } : {}) })),
};
await writeFile(`${root}/manifest.json`, JSON.stringify(manifest, null, 2));
await writeFile(`${root}/.sdignore`, '*.map\n*.log\n.env*\nlogs/\n');
console.log(`Built ${root}`);
