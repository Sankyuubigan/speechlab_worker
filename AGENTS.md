# AGENTS.md — SpeechLab (Rust + Tauri + Svelte)

## ПРАВИЛО №1 (главное, выше всех остальных)
**ПЕРЕД КАЖДЫМ изменением кода, правкой параметров или дебагом — ИЩИ
ДОКУМЕНТАЦИЮ В ИНТЕРНЕТЕ.** Не полагайся на память и не угадывай.
- Сверяйся с эталонным репозиторием `onnx-asr` (github.com/istupakov/onnx-asr):
  `preprocessors/gigaam.py`, `src/onnx_asr/models/gigaam.py`, `src/onnx_asr/asr.py`,
  `src/onnx_asr/preprocessors/numpy_preprocessor.py`, `src/onnx_asr/preprocessors/fbanks.py`.
- Для API крейтов (ort, symphonia, ndarray) — docs.rs / исходники.
- Если интернет недоступен для поиска — прочитай уже скачанные эталонные
  исходники из репозитория и только на их основе вноси правки.
- Обновляй этот раздел, когда находишь новые авторитетные источники/факты.

## Назначение проекта
Десктопное GUI-приложение (Tauri) для распознавания речи (ASR) с русского языка.
Пользователь перетаскивает в окно `.ogg` (и другие) аудиофайлы, указывает путь к
модели gigaam-v3, и получает распознанный текст. Это переписанный на Rust+TAuri
аналог рабочего Python-скрипта на базе `onnx_asr` + `gigaam-v3` + `silero`.

## Стек
- **Tauri 2** (Rust backend + WebView frontend)
- **Svelte 5** (frontend, TypeScript)
- **ort 2.0** — ONNX Runtime для инференса gigaam (CTC) и silero VAD
- **symphonia 0.6** — декод аудио (ogg/vorbis, wav, flac, mp3) без внешних либ
- **hound 3** — чтение/запись WAV
- **anyhow** — обработка ошибок

## Архитектура (модульная)
Бизнес-фича = модуль. Каждый модуль в `src-tauri/src/modules/`.

```
src-tauri/src/
  main.rs              # точка входа
  lib.rs               # Tauri: состояние (AppState), команды (invoke_handler)
  modules/
    audio/
      mod.rs           # decode_to_mono()
      decode.rs        # symphonia: файл -> моно f32 + sample_rate
      wav.rs           # запись WAV (hound)
    asr/
      mod.rs
      gigaam.rs        # GigaamCtc: загрузка CTC-модели, recognize_file()
      logmel.rs        # LogMel: ручной log-mel препроцессинг (как в onnx_asr)
      vad.rs           # разбиение по длине (fallback, silero TODO)
```

Frontend: `src/App.svelte` (drag-drop зона, поле пути к модели, кнопки, вывод).

## Правила работы с кодом
1. **Модульность**: новая фича = новый модуль в `modules/`, зарегистрированный в
   `mod.rs`. Не сваливать логику в `lib.rs`/`main.rs`.
2. **Модель gigaam**: используем `v3_e2e_ctc.int8.onnx` (CTC, проще RNNT). Путь к
   папке модели передаётся из GUI (поле `modelDir`), по умолчанию
   `D:\nn\models\stt\gigaam-v3`. Модель грузится лениво (кнопка "загрузить модель").
3. **Препроцессинг**: в `onnx_asr` это отдельный ONNX (`gigaam_v3.onnx`), которого
   нет в папке модели. Реализован вручную в `logmel.rs`:
   - sample_rate 16000, n_mels 64, n_fft 512, win_length 1024, hop_length 160
   - pre-emphasis 0.97, окно Ханна, log-mel, клиппинг ln(x+1e-10)
   - выход модели: `features` shape `1 x 64 x num_frames` (num_frames = ms/10)
4. **CTC-decode**: greedy argmax по времени + схлопывание повторов + удаление blank.
   Blank-токен в vocab = `<blk>`. Токен `\u2581` заменяется на пробел (postprocessing).
5. **VAD**: отдельного silero.onnx у пользователя нет. Пока fallback — разбиение
   файла на чанки по длине (`vad.rs`). Тонкая сегментация через silero — TODO.
6. **Команды Tauri** (в `lib.rs`): `set_model_dir`, `load_model`, `recognize`.
   Все принимают/возвращают простые типы (String, Vec<String>).
7. **Обработка ошибок**: Rust-сторона возвращает `Result<_, String>`; ошибки
   показываются в GUI как `status`. Не паниковать в командax.
8. **Зависимости**: новые крейты добавлять в `src-tauri/Cargo.toml`. Версии фиксировать
   по документации (docs.rs / github). НЕ менять `ort` на другой без согласования.

## Сборка и запуск
```powershell
# ВАЖНО: НЕ использовать прямой `cargo build` / `cargo test`!
# В окружении стоит sccache в CC/CXX/CMAKE_CXX_COMPILER_LAUNCHER, который ломает
# сборку (vswhom-sys не находит cl). Всегда собирать ТОЛЬКО через build.bat:
cmd /c build.bat
```
`build.bat` делает: vcvarsall x64 → сброс CC/CXX/sccache → `node build.cjs`
(который: npm install, генерация иконок, `tauri build --debug`, `cargo test
ogg_decode`, запуск speechlab.exe). Логи пишутся в папку проекта (`build.log`),
НЕ в какую-либо темп-директорию — читай их оттуда.

Frontend отдельно: `npm run build` (= `vite build`). `npm run dev` — только фронт.

Rust-зависимости тянутся через `cargo` (внутри `tauri dev/build`). Сборка ort
долгая (качает/линкует onnxruntime).

## Запрещённые директории
- **НЕ лезть в `C:\Users\user\AppData\Local\Temp\` и любые темп-папки.** Проект
  собирается в `speechlab/target` и `node_modules`; логи — в папку проекта.
  Временные файлы (тестовые WAV, дампы) писать рядом с проектом или в `target/`.
- **НЕ указывать явный диск/путь `D:\Projects\...` в командах bash.** Рабочая
  директория уже `D:\Projects\speechlab_master_5\speechlab` — достаточно
  `cd speechlab` или запускать команды без `cd` (работают в текущей папке).
  Лишний `cd /d "D:\..."` только мусорит и ломает относительные пути.

## Важные замечания
- `onnx_asr` (Python) — эталон поведения. Исходники: github.com/istupakov/onnx-asr
  (`src/onnx_asr/models/gigaam.py`, `src/onnx_asr/asr.py`,
  `src/onnx_asr/preprocessors/numpy_preprocessor.py`,
  `src/onnx_asr/preprocessors/fbanks.py`, `preprocessors/gigaam.py`). При сомнениях
  в параметрах препроцессинга/декодинга сверяться с ними.
- **УНИВЕРСАЛЬНОСТЬ:** приложение должно работать с ЛЮБЫМИ STT-моделями gigaam,
  которые есть в папке (CTC и/или RNNT). Авто-детект файлов в `ModelRunner::load`
  (CTC если есть `*_ctc*.onnx`, RNNT если есть `*_rnnt_*.onnx`). Оба движка
  реализованы и проверяются; приоритет — тот, что есть на диске (RNNT при наличии
  обоих, т.к. он точнее; но CTC проще и надёжнее).
- **Параметры препроцессинга V3 (ТОЧНО по эталону, не V2!):**
  `sample_rate=16000`, `n_fft=320` (`16000//50`), `win_length=320`, `hop_length=160`,
  `n_mels=64`, `f_min=0`, `f_max=8000`. Окно — **Ханна** `np.hanning(win+1)[:-1]`.
  **НЕТ pre-emphasis** (V3 не использует). Спектр = `|rfft(frame, n_fft)|**2`,
  мел-банк HTK (`fbanks.py`: `melscale_fbanks`, `mel_scale="htk"`, без нормализации),
  `log(clip(mel, 1e-9, 1e9))`. Форма выхода `(1, 64, T)`, `T = (N - win)//hop + 1`.
  (V2 использует `n_fft=400`, `win=400`, `hop=160` и pre-emphasis 0.97 — это ДРУГИЕ
  параметры, не путать!)
- **Битый файл `gigaam_v3.onnx`:** в папке модели `D:\nn\models\stt\gigaam-v3`
  лежит БИТЫЙ файл (14 байт, содержимое `404: Not Found` — скачан ошибочно вместо
  ONNX). Код НЕ должен молча на него полагаться: либо скачать/сгенерировать
  настоящий препроцессор, либо (надёжнее) удалить битый и использовать
  исправленный ручной fallback. Настоящий `gigaam_v3.onnx` в публичном репо НЕ
  лежит как файл — он генерируется скриптом `preprocessors/build.py` из функции
  `GigaamPreprocessorV3` (см. `preprocessors/gigaam.py`). Его можно сгенерировать
  локально (Python + onnxscript) или просто полагаться на точный ручной LogMel V3.
- Модель ждёт 16kHz mono. Любой вход декодируется, усредняется в моно и при
  необходимости ресемплится (линейно, в `gigaam.rs`, пока без rubato).
- Гигабайтные модели НЕ коммитить. Только исходники.

## ЖЁСТКИЕ ПРАВИЛА (на будущее — чтобы не тратить токены впустую)
1. **НЕ угадывать параметры препроцессинга/модели.** Перед любой правкой взять
   эталон из РАБОЧЕГО кода пользователя (его Python-скрипт и т.п.) или из
   официального репозитория. Если у пользователя есть рабочий скрипт — ТРЕБОВАТЬ
   его ДО начала правок. НЕ писать «велосипеды» вслепую (как ручной log-mel).
2. **НЕ тратить токены на итеративную компиляцию ради угадывания API крейтов.**
   Перед правкой сверяться с docs.rs / исходниками крейтов (symphonia, ort) и с
   эталонным кодом (onnx_asr `preprocessors/gigaam.py`). Один точный взгляд в
   доки экономит десятки итераций сборки.
3. **Использовать готовый официальный препроцессор `gigaam_v3.onnx`** вместо
   ручного log-mel. Он лежит в репозитории onnx-asr (models/gigaam_v3.onnx) и
   точно совпадает с тем, что ждёт модель. Ручной log-mel — только если файла
   препроцессора нет и договорились с пользователем.
   ВНИМАНИЕ: в папке модели сейчас лежит БИТЫЙ `gigaam_v3.onnx` (14 байт,
   `404: Not Found`). Код должен либо сгенерировать/заменить его настоящим, либо
   при обнаружении невалидного ONNX — падать с понятной ошибкой, а не молча
   выдавать пустой текст. Надёжнее: удалить битый файл и полагаться на исправленный
   ручной LogMel V3 (параметры — см. «Важные замечания», V3).
4. **Универсальная поддержка CTC и RNNT.** Оба движка реализованы и авто-выбираются
   по наличию файлов. CTC: greedy argmax по времени + схлопывание повторов +
   удаление blank (`<blk>`), `\u2581` → пробел. RNNT: transducer greedy-декодинг
   по эталону `src/onnx_asr/asr.py` (`_AsrWithTransducerDecoding._decoding`):
   encoder→decoder(joiner), на каждом шаге argmax; если не blank — эмитим токен и
   обновляем состояние; шаг t += 1 при blank ИЛИ при достижении `max_tokens_per_step`
   (для gigaam = 3). Входы/выходы RNNT: encoder `audio_signal`(B×64×T)+`length`,
   выход `encoded`+`encoded_len`; decoder `x`(1×1)+`h.1`(1×1×320)+`c.1`→`dec`+`h`+`c`;
   joiner `enc`+`dec`→`joint`.
5. **Препроцессинг-ONNX (`gigaam_v3.onnx`) берёт на вход WAV 16k mono float и
   выдаёт `features` (1 x 64 x T).** Пайплайн Rust:
   decode(ogg/...) → resample→16k mono → `gigaam_v3.onnx`(features) →
   `v3_e2e_ctc.int8.onnx`(logits) → CTC-decode. НЕ считать log-mel руками.
6. **НЕ запускать сборку/тесты без `build.bat`/`run_test.bat`** (sccache ломает
   cc). Уже зафиксировано выше.
7. **НЕ лезть в temp-папки / не указывать явный диск `D:\` в командах.** Уже
   зафиксировано выше.

## Эталонные ссылки
- Препроцессор (исходник): https://github.com/istupakov/onnx-asr/blob/main/preprocessors/gigaam.py
- NumPy-препроцессор (эталон точных вычислений): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/preprocessors/numpy_preprocessor.py
- Мел-банк (HTK, формула): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/preprocessors/fbanks.py
- Модель gigaam (входы/выходы CTC и RNNT): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/models/gigaam.py
- Базовый ASR + декодинг (CTC и transducer): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/asr.py
- Сборка препроцессоров: https://github.com/istupakov/onnx-asr/blob/main/preprocessors/build.py
- Папка моделей репозитория: https://github.com/istupakov/onnx-asr/tree/main/src/onnx_asr/models
- Рабочий Python-скрипт пользователя (эталон): `load_model("gigaam-v3-e2e-rnnt")`
  + `.with_vad(silero)` + конвертация ogg→wav через soundfile. Мы делаем CTC-аналог
  без VAD (fallback по длине), т.к. silero.onnx у пользователя нет.
- ПРОВЕРЕНО (2026-07-19): файл `gigaam_v3.onnx` в публичном репо НЕ лежит как готовый
  бинарник — он только генерируется скриптом `preprocessors/build.py`. Поэтому
  полагаться на ручной LogMel V3 (см. «Важные замечания») — корректно и надёжно.
