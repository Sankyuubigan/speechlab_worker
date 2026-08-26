const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

const scriptDir = __dirname;
const EXE_NAME = 'speechlab.exe';

// Каталог общих ключей подписи (rules.md §3.1):
// <docs>/software/настройки/tauri_signed_keys/
const DOCS_KEYS_DIR = path.join(
  'D:\\Projects\\docusaurus-starter\\docs\\Sega Mega Note\\Моя картотека\\software\\настройки',
  'tauri_signed_keys'
);

function runCommand(command, args = [], options = {}) {
  return new Promise((resolve, reject) => {
    console.log(`> ${command} ${args.join(' ')}`);
    const proc = spawn(command, args, { stdio: 'inherit', shell: true, cwd: scriptDir, ...options });
    proc.on('close', (code) => {
      if (code === 0) resolve();
      else reject(new Error(`Команда завершилась с кодом ошибки ${code}`));
    });
  });
}

// Читает приватный ключ из TAURI_PRIVATE_KEY_ORIGINAL (env) либо из общего
// каталога документации, прокидывает TAURI_SIGNING_PRIVATE_KEY /
// TAURI_SIGNING_PRIVATE_KEY_PASSWORD и вшивает публичный ключ в tauri.conf.json
// (plugins.updater.pubkey + bundle.createUpdaterArtifacts).
function setupSigning() {
  const keyPath = process.env.TAURI_PRIVATE_KEY_ORIGINAL || path.join(DOCS_KEYS_DIR, 'tauri.key');
  if (!fs.existsSync(keyPath)) {
    console.warn('⚠️ Приватный ключ не найден — сборка БЕЗ подписи updater (TAURI_SIGNING_PRIVATE_KEY отсутствует).');
    return false;
  }
  const key = fs.readFileSync(keyPath, 'utf8').replace(/\s+/g, '');
  process.env.TAURI_SIGNING_PRIVATE_KEY = key;

  const pwPath = path.join(DOCS_KEYS_DIR, 'TAURI_KEY_PASSWORD.txt');
  if (fs.existsSync(pwPath)) {
    process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD = fs.readFileSync(pwPath, 'utf8').trim();
  }

  const pubPath = path.join(DOCS_KEYS_DIR, 'tauri.key.pub');
  if (fs.existsSync(pubPath)) {
    injectPubkey(fs.readFileSync(pubPath, 'utf8').trim());
  }
  console.log('  ✅ Ключи подписи загружены из общего каталога docs.');
  return true;
}

function injectPubkey(pubkey) {
  const cfgPath = path.join(scriptDir, 'src-tauri', 'tauri.conf.json');
  const cfg = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
  cfg.plugins = cfg.plugins || {};
  cfg.plugins.updater = cfg.plugins.updater || {};
  cfg.plugins.updater.pubkey = pubkey;
  if (!Array.isArray(cfg.plugins.updater.endpoints) || cfg.plugins.updater.endpoints.length === 0) {
    cfg.plugins.updater.endpoints = ['https://raw.githubusercontent.com/USER/REPO/main/latest.json'];
  }
  cfg.bundle = cfg.bundle || {};
  cfg.bundle.createUpdaterArtifacts = true;
  fs.writeFileSync(cfgPath, JSON.stringify(cfg, null, 2) + '\n');
}

async function buildInstaller() {
  console.log('========================================');
  console.log('[1/4] Установка зависимостей Node.js...');
  await runCommand('npm', ['install']);

  console.log('\n========================================');
  console.log('[2/4] Проверка и генерация иконок...');
  const iconsDir = path.join(scriptDir, 'src-tauri', 'icons');
  const sourceIconPath = path.join(iconsDir, 'icon.png');
  if (!fs.existsSync(iconsDir)) fs.mkdirSync(iconsDir, { recursive: true });
  if (fs.existsSync(sourceIconPath)) {
    try {
      await runCommand('npx', ['tauri', 'icon', 'src-tauri/icons/icon.png']);
    } catch {
      console.warn('⚠️ Не удалось сгенерировать иконки — продолжаем сборку.');
    }
  } else {
    console.warn('⚠️ Базовая иконка (icon.png) не найдена.');
  }

  console.log('\n========================================');
  console.log('[3/4] Настройка ключей подписи (rules.md §3.1)...');
  setupSigning();

  console.log('\n========================================');
  console.log('[4/4] Сборка release-инсталлятора (npx tauri build)...');
  await runCommand('npx', ['tauri', 'build']);

  const bundleDir = path.join(scriptDir, 'src-tauri', 'target', 'release', 'bundle');
  console.log(`  ✅ Сборка завершена. Инсталлятор(ы) в: ${bundleDir}`);
}

module.exports = { buildInstaller, setupSigning, DOCS_KEYS_DIR };

if (require.main === module) {
  buildInstaller().catch((e) => {
    console.error('\n========================================');
    console.error('❌ ОШИБКА:', e.message);
    console.error('========================================');
    process.exit(1);
  });
}
