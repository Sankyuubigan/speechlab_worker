# AGENTS.md — SpeechLab (Rust + Tauri + Svelte)

## 0. Глобальная документация (ОБЯЗАТЕЛЬНО читать первой)

Перед любой работой с тяжёлыми AI-движками (LLM/TTS/ASR вне уже подключённого `ort`)
читай глобальную базу знаний по пути:

`D:\Projects\docusaurus-starter\docs\Sega Mega Note\Моя картотека\software\настройки\global_ai_docs`

Обязательно:
- `core/rules.md` — базовые правила (git, логи, честность, AI-friendly архитектура).
- `desktop_rust_tauri/rules.md` — правила Rust+Tauri, в т.ч. **§6.7: инференс AI — только
  отдельным prebuilt-процессом + HTTP localhost, НЕ нативная линковка** (`crispasr-sys`,
  `llama-cpp-sys` и подобные запрещены: 20-мин CUDA-сборка, поиск DLL, нет graceful-выгрузки VRAM).
- `desktop_rust_tauri/llama_cpp_engine.md` — эталонный паттерн интеграции движка
  (скачивание prebuilt-бинаря, spawn subprocess, HTTP, Drop/Job Object cleanup).
- `desktop_rust_tauri/crispasr_engine.md` — та же схема, применённая к **CrispASR** (TTS/ASR).
  CrispASR ставится ТОЧНО так же, как llama.cpp: отдельный скачанный `crispasr.exe` + HTTP
  (`POST /v1/audio/speech`), без линковки `crispasr`-крейта.

Правило: новый AI-движок = отдельный prebuilt-процесс + HTTP (как llama.cpp/CrispASR),
НЕ добавление `-sys`/`-binding` крейта в `Cargo.toml`.

## Эталонные материалы по проекту
Сверяйся с эталонным репозиторием `onnx-asr` (github.com/istupakov/onnx-asr):
- `preprocessors/gigaam.py`, `src/onnx_asr/models/gigaam.py`, `src/onnx_asr/asr.py`.
- `src/onnx_asr/preprocessors/numpy_preprocessor.py`, `src/onnx_asr/preprocessors/fbanks.py`.
Если интернет недоступен для поиска — прочитай уже скачанные эталонные исходники из репозитория пользователя (если они есть) и только на их основе вноси правки.

## Назначение проекта
Десктопное GUI-приложение (Tauri) для распознавания речи (ASR) с русского языка.
Пользователь перетаскивает в окно `.ogg` (и другие) аудиофайлы, указывает путь к модели gigaam-v3, и получает распознанный текст. Это переписанный на Rust+TAuri аналог рабочего Python-скрипта на базе `onnx_asr` + `gigaam-v3` + `silero`.

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

## Правила работы с кодом
1. **Модульность**: новая фича = новый модуль в `modules/`, зарегистрированный в `mod.rs`. Не сваливать логику в `lib.rs`/`main.rs`.
2. **Модель gigaam**: используем `v3_e2e_ctc.int8.onnx` (CTC, проще RNNT). Путь к папке модели передаётся из GUI (поле `modelDir`), по умолчанию `D:\nn\models\stt\gigaam-v3`. Модель грузится лениво (кнопка "загрузить модель").
3. **Препроцессинг**: в `onnx_asr` это отдельный ONNX (`gigaam_v3.onnx`), которого нет в папке модели. Реализован вручную в `logmel.rs`.
4. **CTC-decode**: greedy argmax по времени + схлопывание повторов + удаление blank. Blank-токен в vocab = `<blk>`. Токен `\u2581` заменяется на пробел (postprocessing).
5. **VAD**: отдельного silero.onnx у пользователя нет. Пока fallback — разбиение файла на чанки по длине (`vad.rs`). Тонкая сегментация через silero — TODO.
6. **Команды Tauri** (в `lib.rs`): `set_model_dir`, `load_model`, `recognize`. Все принимают/возвращают простые типы (String, Vec<String>).
7. **Зависимости**: новые крейты добавлять в `src-tauri/Cargo.toml`. Версии фиксировать по документации (docs.rs / github). НЕ менять `ort` на другой без согласования.

## Важные замечания
- `onnx_asr` (Python) — эталон поведения. При сомнениях в параметрах препроцессинга/декодинга сверяться с исходниками `istupakov/onnx-asr`.
- **УНИВЕРСАЛЬНОСТЬ:** приложение должно работать с ЛЮБЫМИ STT-моделями gigaam, которые есть в папке (CTC и/или RNNT). Авто-детект файлов в `ModelRunner::load`. Приоритет — тот, что есть на диске (RNNT при наличии обоих).
- **Параметры препроцессинга V3 (ТОЧНО по эталону, не V2!):**
  `sample_rate=16000`, `n_fft=320` (`16000//50`), `win_length=320`, `hop_length=160`, `n_mels=64`, `f_min=0`, `f_max=8000`. Окно — **Ханна** `np.hanning(win+1)[:-1]`. **НЕТ pre-emphasis** (V3 не использует). Спектр = `|rfft(frame, n_fft)|**2`, мел-банк HTK (`fbanks.py`: `melscale_fbanks`, `mel_scale="htk"`, без нормализации), `log(clip(mel, 1e-9, 1e9))`. Форма выхода `(1, 64, T)`.
- **Битый файл `gigaam_v3.onnx`:** в папке модели лежит БИТЫЙ файл (14 байт, содержимое `404: Not Found`). Код должен удалять или игнорировать битый ONNX и использовать исправленный ручной fallback `LogMel V3`.
- Модель ждёт 16kHz mono. Любой вход декодируется, усредняется в моно и при необходимости ресемплится (линейно, в `gigaam.rs`, пока без rubato).
- Гигабайтные модели НЕ коммитить. Только исходники.

## ЖЁСТКИЕ ПРАВИЛА (Специфичные для проекта)
1. **НЕ угадывать параметры препроцессинга/модели.** Перед любой правкой брать эталон из РАБОЧЕГО Python-скрипта или из официального репозитория. НЕ писать «велосипеды» вслепую для аудио-препроцессинга.
2. **Использовать готовый официальный препроцессор `gigaam_v3.onnx`** вместо ручного log-mel, если он присутствует. ВНИМАНИЕ: из-за наличия битого файла `404: Not Found` мы также полагаемся на точный ручной LogMel V3 в качестве fallback.
3. **Универсальная поддержка CTC и RNNT.** CTC: greedy argmax по времени + удаление blank. RNNT: transducer greedy-декодинг по эталону `_AsrWithTransducerDecoding._decoding`. Шаг `t += 1` при blank ИЛИ при достижении `max_tokens_per_step` (для gigaam = 3). 
4. **Препроцессинг-ONNX берет на вход WAV 16k mono float**. Пайплайн Rust: decode → resample 16k mono → `gigaam_v3.onnx` / `logmel.rs` → `v3_e2e_ctc.int8.onnx` → CTC/RNNT-decode.

## Эталонные ссылки
- Препроцессор (исходник): https://github.com/istupakov/onnx-asr/blob/main/preprocessors/gigaam.py
- NumPy-препроцессор (эталон точных вычислений): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/preprocessors/numpy_preprocessor.py
- Мел-банк (HTK, формула): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/preprocessors/fbanks.py
- Модель gigaam (входы/выходы CTC и RNNT): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/models/gigaam.py
- Базовый ASR + декодинг (CTC и transducer): https://github.com/istupakov/onnx-asr/blob/main/src/onnx_asr/asr.py
- Сборка препроцессоров: https://github.com/istupakov/onnx-asr/blob/main/preprocessors/build.py