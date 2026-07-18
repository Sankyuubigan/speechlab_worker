const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

const scriptDir = __dirname;
const EXE_NAME = 'speechlab.exe';
const TEST_OGG = 'D:\\Downloads\\audio_2026-07-18_23-59-01.ogg';

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

async function main() {
    try {
        console.log('========================================');
        console.log('[1/5] Установка зависимостей Node.js...');
        await runCommand('npm', ['install']);

        console.log('\n========================================');
        console.log('[2/5] Проверка и генерация иконок...');
        
        // Автоматически подхватываем иконку из корня
        const rootIconPath = path.join(scriptDir, 'ima1111ges.png');
        const iconsDir = path.join(scriptDir, 'src-tauri', 'icons');
        const sourceIconPath = path.join(iconsDir, 'icon.png');

        if (!fs.existsSync(iconsDir)) {
            fs.mkdirSync(iconsDir, { recursive: true });
        }

        if (fs.existsSync(rootIconPath)) {
            console.log(`Найдена новая иконка в корне (${rootIconPath})`);
            console.log(`Копирую её в ${sourceIconPath}...`);
            fs.copyFileSync(rootIconPath, sourceIconPath);
        }

        // Если базовая иконка существует, перегенерируем все нужные форматы (.ico, .icns, etc)
        if (fs.existsSync(sourceIconPath)) {
            console.log('Генерация форматов иконок для Tauri...');
            try {
                await runCommand('npx', ['tauri', 'icon', 'src-tauri/icons/icon.png']);
            } catch (e) {
                console.warn('⚠️ Не удалось сгенерировать иконки — продолжаем сборку.');
            }
        } else {
            console.warn('⚠️ Базовая иконка (icon.png) не найдена. Создайте её, если сборка завершится с ошибкой.');
        }

        console.log('\n========================================');
        console.log('[3/5] Сборка Rust (debug)...');
        
        // Превентивно создаем dist, чтобы tauri-build не ругался на отсутствие папки, за которой он следит
        const distDir = path.join(scriptDir, 'dist');
        if (!fs.existsSync(distDir)) {
            fs.mkdirSync(distDir, { recursive: true });
        }

        // Собираем без временных файлов (чтобы не ломать cargo watch/test)
        await runCommand('npx', ['tauri', 'build', '--debug']);

        const debugDir = path.join(scriptDir, 'src-tauri', 'target', 'debug');
        const exePath = path.join(debugDir, EXE_NAME);
        if (!fs.existsSync(exePath)) {
            throw new Error(`${EXE_NAME} не найден! Сборка не удалась.`);
        }
        console.log('  ✅ Сборка успешна.');

        console.log('\n========================================');
        console.log('[4/5] Тест декодирования + препроцессинга на реальном ogg...');
        if (fs.existsSync(TEST_OGG)) {
            console.log(`Тестовый файл: ${TEST_OGG}`);
            await runCommand('cargo', ['test', '--manifest-path', 'src-tauri/Cargo.toml', 'ogg_decode', '--', '--nocapture'], { cwd: path.join(scriptDir, 'src-tauri') });
        } else {
            console.warn(`⚠️ Тестовый файл ${TEST_OGG} не найден — пропускаем тест.`);
        }

        console.log('\n========================================');
        console.log('[5/5] Запуск приложения...');
        console.log('🚀 Запуск SpeechLab (без консоли)...');
        const child = spawn(exePath, [], {
            detached: true,
            stdio: 'ignore',
            windowsHide: true
        });
        child.unref();
        console.log('✅ Приложение запущено! Консоль закроется через 1.5с.');
        setTimeout(() => process.exit(0), 1500);

    } catch (e) {
        console.error('\n========================================');
        console.error('❌ ОШИБКА:', e.message);
        console.error('========================================');
        process.exit(1);
    }
}

main();