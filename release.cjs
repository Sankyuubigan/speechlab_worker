const fs = require('fs');
const path = require('path');
const { execSync, spawnSync } = require('child_process');
const { buildInstaller, DOCS_KEYS_DIR } = require('./installer.cjs');

const scriptDir = __dirname;

function run(command, args, opts = {}) {
  const r = spawnSync(command, args, { stdio: 'inherit', shell: true, cwd: scriptDir, ...opts });
  if (r.status !== 0) {
    throw new Error(`Команда завершилась с кодом ${r.status}: ${command} ${args.join(' ')}`);
  }
}

function runOut(command, args, opts = {}) {
  return execSync(`${command} ${args.join(' ')}`, { cwd: scriptDir, encoding: 'utf8', ...opts }).trim();
}

// Инкремент патча (YY.M.P) во всех манифестах. Возвращает новую версию.
function bumpVersion() {
  const confPath = path.join(scriptDir, 'src-tauri', 'tauri.conf.json');
  const cargoPath = path.join(scriptDir, 'src-tauri', 'Cargo.toml');
  const pkgPath = path.join(scriptDir, 'package.json');

  const bump = (v) => {
    const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(v || '');
    if (!m) return v;
    return `${m[1]}.${m[2]}.${parseInt(m[3], 10) + 1}`;
  };

  const conf = JSON.parse(fs.readFileSync(confPath, 'utf8'));
  const newVer = bump(conf.version);
  conf.version = newVer;
  fs.writeFileSync(confPath, JSON.stringify(conf, null, 2) + '\n');

  let cargo = fs.readFileSync(cargoPath, 'utf8');
  cargo = cargo.replace(/^version = ".*"$/m, `version = "${newVer}"`);
  fs.writeFileSync(cargoPath, cargo);

  const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf8'));
  pkg.version = bump(pkg.version);
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + '\n');

  console.log(`  📌 Новая версия: ${newVer}`);
  return newVer;
}

// Ищет собранные артефакты инсталлятора (.exe + .sig).
function findArtifacts() {
  const bundleDir = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle');
  const found = { exe: null, sig: null };
  const walk = (dir) => {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith('-setup.exe')) found.exe = p;
      else if (e.name.endsWith('.sig')) found.sig = p;
    }
  };
  if (fs.existsSync(bundleDir)) walk(bundleDir);
  if (!found.exe) throw new Error('Инсталлятор (*-setup.exe) не найден после сборки.');
  return found;
}

async function main() {
  console.log('########################################');
  console.log('# RELEASE: сборка подписанного инсталлятора');
  console.log('########################################');
  await buildInstaller();

  console.log('\n========================================');
  console.log('[R1] Инкремент версии (patch)...');
  const version = bumpVersion();
  const tag = `v${version}`;

  console.log('\n========================================');
  console.log('[R2] Коммит и тег...');
  run('git', ['add', '-A']);
  run('git', ['commit', '-m', `release: ${tag}`]);
  run('git', ['tag', '-a', tag, '-m', tag]);

  console.log('\n========================================');
  console.log('[R3] Поиск артефактов и публикация релиза...');
  const { exe, sig } = findArtifacts();
  const args = ['release', 'create', tag, exe];
  if (sig) args.push(sig);
  args.push('--title', `SpeechLab ${tag}`, '--notes', `Автоматический релиз ${tag}`);
  run('gh', args);

  console.log('\n========================================');
  console.log('[R4] Генерация latest.json ПОСЛЕ публикации (rules.md §3.3)...');
  const repo = runOut('gh', ['repo', 'view', '--json', 'nameWithOwner', '--jq', '..nameWithOwner']);
  const assetsJson = runOut('gh', [
    'api', `repos/${repo}/releases/tags/${tag}`, '--jq', '.assets',
  ]);
  const assets = JSON.parse(assetsJson);
  const exeAsset = assets.find((a) => a.name.endsWith('-setup.exe'));
  const sigAsset = assets.find((a) => a.name.endsWith('.sig'));
  if (!exeAsset) throw new Error('Не найден asset -setup.exe в релизе.');

  const signature = sigAsset ? fs.readFileSync(sig, 'utf8').trim() : '';
  const latest = {
    version: tag,
    notes: `Автоматический релиз ${tag}`,
    pub_date: new Date().toISOString(),
    platforms: {
      'windows-x86_64': {
        signature,
        url: exeAsset.browser_download_url,
      },
    },
  };
  fs.writeFileSync(path.join(scriptDir, 'latest.json'), JSON.stringify(latest, null, 2) + '\n');

  console.log('\n========================================');
  console.log('[R5] Публикация latest.json в main...');
  run('git', ['add', 'latest.json']);
  run('git', ['commit', '-m', `chore: latest.json для ${tag}`]);
  run('git', ['push', '--follow-tags', 'origin', 'main']);

  console.log('\n✅ Релиз завершён:', tag);
}

if (require.main === module) {
  main().catch((e) => {
    console.error('\n========================================');
    console.error('❌ ОШИБКА РЕЛИЗА:', e.message);
    console.error('========================================');
    process.exit(1);
  });
}

module.exports = { main, bumpVersion, findArtifacts };
