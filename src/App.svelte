<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { open } from '@tauri-apps/plugin-dialog';
  import { getCurrentWebview } from '@tauri-apps/api/webview';

  let activeTab = $state<'main' | 'tts' | 'settings' | 'logs'>('main');
  let modelDir = $state('D:\\nn\\models\\stt\\gigaam-v3');
  let dropFiles = $state<string[]>([]);
  let status = $state('');
  let busy = $state(false);
  let dragOverZone = $state(false);

  // --- TTS (CrispASR) state ---
  let ttsPresets = $state<{
    id: string; label: string; backend: string; has_codec: boolean; has_voice: boolean;
    voice_type: string; voice_placeholder?: string; builtin_voices: string[]; supports_instruct: boolean;
  }[]>([]);
  let ttsEngineBackends = $state<{ id: string; label: string; asset_name: string; url: string; tag: string }[]>([]);

  let ttsEngineDir = $state('');
  let ttsModelsDir = $state('');
  let ttsEngineBackend = $state('cpu');
  let ttsPreset = $state('qwen3-tts');
  let ttsVoice = $state('');          // имя спикера (named) или путь к WAV (clone)
  let ttsInstruct = $state('');
  let ttsSpeed = $state(1.0);
  let ttsText = $state('Привет, это тест синтеза речи.');
  let ttsStatus = $state('');
  let ttsBusy = $state(false);
  let audioEl: HTMLAudioElement | undefined = $state(undefined);

  // прогресс скачивания
  let dl = $state<{ kind: string; name: string; current: number; total: number }>({ kind: '', name: '', current: 0, total: 0 });
  let dlBusy = $state(false);

  // статусы установленного
  let installedModels = $state<{ id: string; label: string; installed: boolean }[]>([]);
  let engineStatus = $state<{ ok: boolean; latest: string; engines: { id: string; label: string; installed: boolean; installed_version: string | null; latest_version: string; update_available: boolean }[] }>({ ok: true, latest: '', engines: [] });
  let updateInfo = $state('');

  // Состояния для прогресса и результатов
  let currentProgress = $state(0);
  let totalProgress = $state(0);
  let combinedResult = $state('');

  let draggedIdx = $state<number | null>(null);
  let logs = $state<string[]>([]);

  const selectedPreset = $derived(ttsPresets.find(p => p.id === ttsPreset));

  function addPaths(paths: string[]) {
    const filtered = paths.filter(Boolean);
    if (filtered.length) dropFiles = [...new Set([...dropFiles, ...filtered])];
  }

  async function saveSettings() {
    try {
      await invoke('tts_save_settings', {
        settings: {
          engine_dir: ttsEngineDir,
          models_dir: ttsModelsDir,
          engine_backend: ttsEngineBackend,
          preset: ttsPreset,
        },
      });
    } catch { /* игнорируем */ }
  }

  async function refreshModels() {
    try {
      installedModels = await invoke('tts_list_models', { modelsDir: ttsModelsDir });
    } catch { installedModels = []; }
  }

  async function refreshEngineStatus() {
    try {
      engineStatus = await invoke('tts_check_update');
    } catch { /* */ }
  }

  onMount(() => {
    const unlistenDrop = getCurrentWebview().onDragDropEvent((event) => {
      if (event.payload.type === 'over') {
        dragOverZone = true;
      } else if (event.payload.type === 'drop') {
        dragOverZone = false;
        const paths = (event.payload.paths as any[]).map(p => typeof p === 'string' ? p : p.path);
        addPaths(paths);
      } else if (event.payload.type === 'leave') {
        dragOverZone = false;
      }
    });

    const unlistenLog = listen<string>('app-log', (event) => {
      logs = [...logs, event.payload];
    });

    const unlistenDl = listen<{ kind: string; name: string; downloaded: number; total: number }>('tts-download', (event) => {
      const p = event.payload;
      dl = { kind: p.kind, name: p.name, current: p.downloaded, total: p.total };
    });

    invoke<{ id: string; label: string; backend: string; has_codec: boolean; has_voice: boolean; voice_type: string; builtin_voices: string[]; supports_instruct: boolean }[]>('tts_presets')
      .then((p) => { ttsPresets = p; })
      .catch(() => {});

    invoke<{ id: string; label: string; asset_name: string; url: string; tag: string }[]>('tts_engine_backends')
      .then((b) => { ttsEngineBackends = b; })
      .catch(() => {});

    invoke<{ engine_dir: string; models_dir: string; engine_backend: string; preset: string }>('tts_get_settings')
      .then((s) => {
        if (s.engine_dir) ttsEngineDir = s.engine_dir;
        if (s.models_dir) ttsModelsDir = s.models_dir;
        if (s.engine_backend) ttsEngineBackend = s.engine_backend;
        if (s.preset) ttsPreset = s.preset;
      })
      .catch(() => {});

    invoke<{ engine_dir: string; models_dir: string }>('tts_default_dirs')
      .then((d) => {
        if (!ttsEngineDir) ttsEngineDir = d.engine_dir;
        if (!ttsModelsDir) ttsModelsDir = d.models_dir;
      })
      .catch(() => {})
      .finally(() => {
        if (!ttsEngineBackend) ttsEngineBackend = 'cpu';
        if (!ttsPreset && ttsPresets.length) ttsPreset = ttsPresets[0].id;
        refreshModels();
        refreshEngineStatus();
      });

    return () => {
      unlistenDrop.then(fn => fn());
      unlistenLog.then(fn => fn());
      unlistenDl.then(fn => fn());
    };
  });

  async function loadModel() {
    busy = true;
    status = 'загружаю модель...';
    try {
      await invoke('set_model_dir', { dir: modelDir });
      const msg = await invoke<string>('load_model');
      status = msg;
    } catch (e) {
      status = 'ошибка: ' + String(e);
    } finally {
      busy = false;
    }
  }

  async function pickFolder() {
    const picked = await open({ directory: true });
    if (picked && typeof picked === 'string') modelDir = picked;
  }

  async function pickFiles() {
    const picked = await open({
      multiple: true,
      filters: [{ name: 'Audio', extensions: ['ogg', 'wav', 'mp3', 'flac'] }]
    });
    if (picked) {
      const arr = Array.isArray(picked) ? picked : [picked];
      dropFiles = [...new Set([...dropFiles, ...arr])];
    }
  }

  async function recognize() {
    if (dropFiles.length === 0) { status = 'перетащи файлы или выбери'; return; }
    busy = true;
    combinedResult = '';
    const pathsToProcess = [...dropFiles];
    totalProgress = pathsToProcess.length;
    currentProgress = 0;
    try {
      await invoke('set_model_dir', { dir: modelDir });
      for (let i = 0; i < pathsToProcess.length; i++) {
        const path = pathsToProcess[i];
        const filename = path.split('\\').pop()?.split('/').pop() || path;
        status = `распознаю ${i + 1} из ${totalProgress}... (${filename})`;
        const res = await invoke<string[]>('recognize', { paths: [path] });
        const text = res[0] || '[Результат пуст]';
        combinedResult += `=== ${filename} ===\n${text}\n\n`;
        currentProgress = i + 1;
        if (text === '[ОТМЕНЕНО]') break;
      }
      status = 'готово';
    } catch (e) {
      status = 'ошибка: ' + String(e);
    } finally {
      busy = false;
    }
  }

  async function stopRecognition() {
    try {
      await invoke('cancel');
      status = 'останавливаю...';
    } catch (e) {
      status = 'ошибка отмены: ' + String(e);
    }
  }

  function copyResult() {
    if (!combinedResult) return;
    navigator.clipboard.writeText(combinedResult.trim());
    const oldStatus = status;
    status = 'скопировано!';
    setTimeout(() => { if (status === 'скопировано!') status = oldStatus; }, 2000);
  }

  function removeFile(i: number) { dropFiles = dropFiles.filter((_, idx) => idx !== i); }
  function moveUp(i: number) { if (i > 0) { const n = [...dropFiles]; [n[i - 1], n[i]] = [n[i], n[i - 1]]; dropFiles = n; } }
  function moveDown(i: number) { if (i < dropFiles.length - 1) { const n = [...dropFiles]; [n[i + 1], n[i]] = [n[i], n[i + 1]]; dropFiles = n; } }
  function handleDragStart(e: DragEvent, i: number) { draggedIdx = i; if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move'; }
  function handleDragOver(e: DragEvent, i: number) { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'; }
  function handleDrop(e: DragEvent, i: number) {
    e.preventDefault();
    if (draggedIdx !== null && draggedIdx !== i) {
      const n = [...dropFiles];
      const [moved] = n.splice(draggedIdx, 1);
      n.splice(i, 0, moved);
      dropFiles = n;
    }
    draggedIdx = null;
  }

  function copyLogs() { navigator.clipboard.writeText(logs.join('\n')); }

  // --- TTS helpers ---
  async function pickEngineDir() {
    const picked = await open({ directory: true });
    if (picked && typeof picked === 'string') { ttsEngineDir = picked; await saveSettings(); await refreshEngineStatus(); }
  }
  async function pickModelsDir() {
    const picked = await open({ directory: true });
    if (picked && typeof picked === 'string') { ttsModelsDir = picked; await saveSettings(); await refreshModels(); }
  }
  async function pickVoiceWav() {
    const picked = await open({ filters: [{ name: 'WAV', extensions: ['wav'] }] });
    if (picked && typeof picked === 'string') ttsVoice = picked;
  }

  const selectedEngine = $derived(engineStatus.engines.find(e => e.id === ttsEngineBackend));

  async function downloadEngine() {
    if (!ttsEngineBackend) { ttsStatus = 'выберите бэкенд'; return; }
    dlBusy = true; ttsStatus = 'скачиваю движок...';
    try {
      await invoke<string>('tts_download_engine', { backendId: ttsEngineBackend, dest: ttsEngineDir });
      ttsStatus = 'движок скачан';
      await refreshEngineStatus();
    } catch (e) { ttsStatus = 'ошибка: ' + String(e); }
    finally { dlBusy = false; dl = { kind: '', name: '', current: 0, total: 0 }; }
  }

  async function checkUpdate() {
    await refreshEngineStatus();
    if (engineStatus.ok) {
      const need = engineStatus.engines.filter(e => e.update_available).map(e => e.label);
      updateInfo = need.length
        ? `Последняя версия: ${engineStatus.latest}. Обновления доступны для: ${need.join(', ')}`
        : `Установлена последняя версия: ${engineStatus.latest}`;
    } else {
      updateInfo = 'не удалось проверить обновления: ' + (engineStatus as any).error;
    }
  }

  async function downloadModel(presetId: string) {
    if (!ttsModelsDir) { ttsStatus = 'укажите папку моделей в Настройках'; return; }
    dlBusy = true; ttsStatus = `скачиваю модель (${presetId})...`;
    try {
      await invoke('tts_download_model', { preset: presetId, dest: ttsModelsDir });
      ttsStatus = 'модель скачана';
      await refreshModels();
    } catch (e) { ttsStatus = 'ошибка: ' + String(e); }
    finally { dlBusy = false; dl = { kind: '', name: '', current: 0, total: 0 }; }
  }

  async function ttsSpeak() {
    if (!ttsText.trim()) { ttsStatus = 'введите текст'; return; }
    if (!selectedPreset) { ttsStatus = 'выберите модель TTS'; return; }
    const isInstalled = installedModels.find(m => m.id === ttsPreset)?.installed;
    if (!isInstalled) { ttsStatus = 'модель не установлена — скачайте её в Настройках'; return; }
    if (selectedPreset.voice_type === 'clone' && !ttsVoice) { ttsStatus = 'выберите референсный WAV для клонирования'; return; }

    ttsBusy = true;
    ttsStatus = 'готовлю движок...';
    const voiceIsWav = selectedPreset.voice_type === 'clone' && ttsVoice.toLowerCase().endsWith('.wav');
    try {
      const wav = await invoke<number[] | Uint8Array>('tts_speak', {
        preset: ttsPreset,
        voice: ttsVoice,
        voiceIsWav,
        instruct: selectedPreset.supports_instruct ? ttsInstruct : '',
        speed: ttsSpeed,
        text: ttsText,
      });
      const blob = new Blob([new Uint8Array(wav as any)], { type: 'audio/wav' });
      if (audioEl) {
        audioEl.src = URL.createObjectURL(blob);
        await audioEl.play();
      }
      ttsStatus = 'воспроизвожу...';
    } catch (e) {
      ttsStatus = 'ошибка: ' + String(e);
    } finally {
      ttsBusy = false;
    }
  }

  async function ttsUnload() {
    try { await invoke('tts_unload'); ttsStatus = 'движок выгружен'; }
    catch (e) { ttsStatus = 'ошибка выгрузки: ' + String(e); }
  }

  function dlPercent() { return dl.total ? Math.min(100, (dl.current / dl.total) * 100) : 0; }
</script>

<main>
  <h1>SpeechLab — ASR + TTS</h1>

  <div class="tabs">
    <button class:active={activeTab === 'main'} onclick={() => activeTab = 'main'}>Распознавание</button>
    <button class:active={activeTab === 'tts'} onclick={() => activeTab = 'tts'}>ТТС</button>
    <button class:active={activeTab === 'settings'} onclick={() => activeTab = 'settings'}>⚙ Настройки</button>
    <button class:active={activeTab === 'logs'} onclick={() => activeTab = 'logs'}>Логи ({logs.length})</button>
  </div>

  {#if activeTab === 'main'}
    <section class="model" aria-label="Настройки модели">
      <label for="model_input">Путь к модели (gigaam-v3):</label>
      <div class="row">
        <input id="model_input" bind:value={modelDir} placeholder="D:\nn\\models\\stt\\gigaam-v3" />
        <button onclick={pickFolder}>выбрать папку</button>
        <button onclick={loadModel} disabled={busy}>загрузить модель</button>
      </div>
    </section>

    <div class="dropzone" class:over={dragOverZone} aria-label="Зона загрузки файлов">
      <p>Перетащи сюда аудиофайлы (несколько сразу) <br/> или</p>
      <button onclick={pickFiles}>выбрать файлы</button>
    </div>

    {#if dropFiles.length}
      <ul class="files">
        {#each dropFiles as f, i}
          <li draggable="true" ondragstart={(e) => handleDragStart(e, i)} ondragover={(e) => handleDragOver(e, i)} ondrop={(e) => handleDrop(e, i)} class:drag-active={draggedIdx === i}>
            <span class="fname">{f}</span>
            <div class="factions">
              <button class="arr" onclick={() => moveUp(i)} disabled={i === 0}>▲</button>
              <button class="arr" onclick={() => moveDown(i)} disabled={i === dropFiles.length - 1}>▼</button>
              <button class="x" onclick={() => removeFile(i)}>✕</button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="row actions">
      <button class="primary" onclick={recognize} disabled={busy || dropFiles.length === 0}>распознать</button>
      <button class="stop" onclick={stopRecognition} disabled={!busy}>остановить</button>
      <span class="status">{status}</span>
    </div>

    {#if totalProgress > 0}
      <div class="progress-wrap">
        <div class="progress-bar" style="width: {(currentProgress / totalProgress) * 100}%"></div>
      </div>
    {/if}

    {#if combinedResult}
      <section class="results" aria-label="Результаты">
        <div class="results-header">
          <h2>Результат</h2>
          <button onclick={copyResult}>Копировать</button>
        </div>
        <textarea class="rtext single-result" readonly bind:value={combinedResult}></textarea>
      </section>
    {/if}

  {:else if activeTab === 'tts'}
    <section class="tts-section" aria-label="Синтез речи">
      <h2>ТТС (CrispASR, локально)</h2>

      <label for="tts_preset">Модель TTS (пресет):</label>
      <div class="row">
        <select id="tts_preset" bind:value={ttsPreset} onchange={() => { ttsVoice = ''; saveSettings(); }}>
          {#each ttsPresets as p}
            <option value={p.id}>{p.label}{installedModels.find(m => m.id === p.id)?.installed ? ' ✓' : ' (не установлено)'}</option>
          {/each}
        </select>
      </div>
      {#if selectedPreset && !installedModels.find(m => m.id === ttsPreset)?.installed}
        <p class="hint warn">Модель не скачана. Откройте «Настройки → Папка моделей TTS» и нажмите «Скачать».</p>
      {/if}

      {#if selectedPreset?.voice_type === 'named'}
        <label for="tts_voice">Имя голоса (встроенный спикер):</label>
        <div class="row">
          <input id="tts_voice" list="voice-list" bind:value={ttsVoice} placeholder="напр. af_heart / vivian" />
          <datalist id="voice-list">
            {#each selectedPreset.builtin_voices as v}<option value={v}></option>{/each}
          </datalist>
        </div>
      {:else if selectedPreset?.voice_type === 'clone'}
        <label>Голос — клонирование из референсного WAV:</label>
        <div class="row">
          <button onclick={pickVoiceWav}>выбрать WAV</button>
          <span class="status">{ttsVoice ? ttsVoice.split('\\').pop() : 'не выбран'}</span>
        </div>
      {:else if selectedPreset?.voice_type === 'ggupack'}
        <p class="hint">Голосовой GGUF-пак скачивается автоматически вместе с моделью (стандартный голос).</p>
      {/if}

      <label for="tts_instruct">Инструкция (стиль/описание голоса):</label>
      {#if selectedPreset?.supports_instruct}
        <div class="row">
          <input id="tts_instruct" bind:value={ttsInstruct} placeholder="напр. spoke very slowly / calm adult male" />
        </div>
      {:else}
        <div class="row">
          <input id="tts_instruct" value="" disabled placeholder="не поддерживается моделью {selectedPreset?.backend ?? ''}" />
        </div>
        <p class="hint">Эта модель не принимает инструкцию по стилю (поддерживают: Qwen3-TTS CustomVoice, Parler-TTS, Irodori-TTS).</p>
      {/if}

      <label for="tts_speed">Скорость: {ttsSpeed.toFixed(2)}</label>
      <div class="row">
        <input id="tts_speed" type="range" min="0.25" max="4" step="0.05" bind:value={ttsSpeed} />
      </div>

      <label for="tts_text">Текст для озвучивания:</label>
      <textarea id="tts_text" class="tts-text" bind:value={ttsText}></textarea>

      <div class="row actions">
        <button class="primary" onclick={ttsSpeak} disabled={ttsBusy}>озвучить</button>
        <button class="stop" onclick={ttsUnload} disabled={ttsBusy}>выгрузить движок</button>
        <span class="status">{ttsStatus}</span>
      </div>

      <audio bind:this={audioEl}></audio>
    </section>

  {:else if activeTab === 'settings'}
    <section class="settings-section" aria-label="Настройки">
      <h2>Настройки — движок CrispASR (TTS)</h2>

      <h3>Движок</h3>
      <label for="tts_backend">Тип бэкенда:</label>
      <div class="row">
        <select id="tts_backend" bind:value={ttsEngineBackend} onchange={() => { saveSettings(); refreshEngineStatus(); }}>
          {#each ttsEngineBackends as b}
            <option value={b.id}>{b.label}</option>
          {/each}
        </select>
        {#if !ttsEngineBackends.length}
          <span class="status">не удалось получить список бинарей (нет сети?)</span>
        {/if}
      </div>

      <p class="hint">
        Статус: {selectedEngine ? (selectedEngine.installed ? `установлен (${selectedEngine.installed_version ?? '?'})` : 'не установлен') : '—'}
      </p>

      <div class="row">
        <button class="primary" onclick={downloadEngine} disabled={dlBusy || !ttsEngineBackends.length}>скачать движок</button>
        <button onclick={checkUpdate} disabled={dlBusy}>проверить обновления</button>
        <button onclick={pickEngineDir}>изменить путь к движку</button>
      </div>
      <p class="hint">Путь к движку: <code>{ttsEngineDir}</code></p>
      {#if updateInfo}<p class="hint">{updateInfo}</p>{/if}

      <hr />

      <h3>Папка моделей TTS</h3>
      <div class="row">
        <input bind:value={ttsModelsDir} placeholder="общая папка для моделей TTS" readonly />
        <button onclick={pickModelsDir}>выбрать папку</button>
      </div>
      <p class="hint">Внутри создаётся подпапка на каждый пресет; все GGUF качаются туда автоматически.</p>

      <h4>Установленные модели:</h4>
      <ul class="models">
        {#each installedModels as m}
          <li>
            <span class="mname">{m.label}</span>
            {#if m.installed}
              <span class="ok">✓ установлено</span>
            {:else}
              <button class="small" onclick={() => downloadModel(m.id)} disabled={dlBusy}>скачать</button>
            {/if}
          </li>
        {/each}
      </ul>
      {#if dlBusy || dl.total > 0}
        <div class="row">
          <span class="status">скачивание: {dl.name} — {Math.round(dlPercent())}%</span>
        </div>
        <div class="progress-wrap">
          <div class="progress-bar" style="width: {dlPercent()}%"></div>
        </div>
      {/if}

      <span class="status">{ttsStatus}</span>
    </section>

  {:else}
    <section class="logs-section">
      <div class="row">
        <button onclick={copyLogs}>Копировать логи</button>
        <button onclick={() => logs = []}>Очистить</button>
      </div>
      <textarea class="logs-area" readonly bind:value={() => logs.join('\n'), (v) => {}}></textarea>
    </section>
  {/if}
</main>

<style>
  :global(body) { font-family: system-ui, sans-serif; margin: 0; background: #1e1e2e; color: #cdd6f4; }
  main { max-width: 900px; margin: 0 auto; padding: 24px; }
  h1 { font-size: 22px; }
  .tabs { display: flex; gap: 8px; margin-bottom: 20px; border-bottom: 1px solid #45475a; padding-bottom: 10px; }
  .tabs button { background: transparent; color: #cdd6f4; font-weight: 600; padding: 6px 12px; font-size: 14px; opacity: 0.6; }
  .tabs button.active { opacity: 1; background: #313244; color: #89b4fa; }
  .tabs button:hover:not(.active) { opacity: 0.8; background: #313244; }

  section { margin-bottom: 18px; }
  label { display: block; margin-bottom: 6px; opacity: 0.8; }
  .row { display: flex; gap: 8px; flex-wrap: wrap; align-items: center; margin-bottom: 10px; }
  input { flex: 1; min-width: 240px; padding: 8px; border-radius: 6px; border: 1px solid #45475a; background: #313244; color: #cdd6f4; }
  button { padding: 8px 14px; border-radius: 6px; border: none; background: #45475a; color: #cdd6f4; cursor: pointer; }
  button:hover:not(:disabled) { background: #585b70; }
  button:disabled { opacity: 0.5; cursor: default; }
  button.primary { background: #89b4fa; color: #1e1e2e; font-weight: 600; }
  button.primary:hover:not(:disabled) { background: #74a0f0; }
  button.stop { background: #f38ba8; color: #1e1e2e; font-weight: 600; }
  button.stop:hover:not(:disabled) { background: #f07193; }
  button.small { padding: 4px 10px; font-size: 12px; }

  .dropzone { border: 2px dashed #585b70; border-radius: 10px; padding: 28px; text-align: center; transition: 0.15s; outline: none; margin-bottom: 18px; }
  .dropzone.over { border-color: #89b4fa; background: #313244; }
  .files { list-style: none; padding: 0; margin: 0; margin-bottom: 18px; }
  .files li { display: flex; justify-content: space-between; align-items: center; padding: 6px 10px; background: #313244; border-radius: 6px; margin-bottom: 4px; cursor: grab; }
  .files li:active { cursor: grabbing; }
  .files li.drag-active { opacity: 0.4; }
  .fname { font-size: 13px; word-break: break-all; }
  .factions { display: flex; gap: 4px; align-items: center; }
  .factions button { padding: 2px 6px; background: transparent; font-size: 12px; opacity: 0.6; }
  .factions button:hover:not(:disabled) { opacity: 1; background: #45475a; }
  .factions button.x { color: #f38ba8; font-size: 14px; }
  .status { opacity: 0.8; font-size: 14px; margin-left: 8px; }
  .hint { opacity: 0.65; font-size: 13px; margin: 4px 0 10px; }
  .hint.warn { color: #f9e2af; }
  code { background: #313244; padding: 1px 6px; border-radius: 4px; word-break: break-all; }

  .progress-wrap { height: 6px; background: #313244; border-radius: 4px; margin: 15px 0 20px; overflow: hidden; width: 100%; }
  .progress-bar { height: 100%; background: #89b4fa; transition: width 0.3s ease; }

  .results { margin-top: 20px; }
  .results-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 10px; }
  .results-header h2 { margin: 0; font-size: 18px; }
  .single-result { width: 100%; box-sizing: border-box; background: #1e1e2e; color: #a6adc8; border: 1px solid #45475a; border-radius: 8px; padding: 14px; font-family: system-ui, sans-serif; font-size: 14px; resize: vertical; line-height: 1.5; outline: none; height: 250px; }

  .logs-section { display: flex; flex-direction: column; height: 70vh; }
  .logs-area { flex: 1; background: #11111b; color: #a6adc8; border: 1px solid #45475a; border-radius: 6px; padding: 12px; font-family: monospace; font-size: 13px; resize: none; outline: none; }

  .tts-section h2 { font-size: 18px; margin: 0 0 14px; }
  .tts-text { width: 100%; box-sizing: border-box; background: #1e1e2e; color: #a6adc8; border: 1px solid #45475a; border-radius: 8px; padding: 14px; font-family: system-ui, sans-serif; font-size: 14px; resize: vertical; line-height: 1.5; outline: none; min-height: 140px; margin-bottom: 12px; }

  .settings-section h2 { font-size: 18px; margin: 0 0 14px; }
  .settings-section h3 { font-size: 15px; margin: 14px 0 8px; opacity: 0.85; }
  .settings-section h4 { font-size: 14px; margin: 12px 0 6px; opacity: 0.8; }
  .settings-section select { flex: 1; min-width: 240px; padding: 8px; border-radius: 6px; border: 1px solid #45475a; background: #313244; color: #cdd6f4; }
  .settings-section hr { border: none; border-top: 1px solid #45475a; margin: 16px 0; }
  .settings-section input[readonly] { opacity: 0.85; }
  .models { list-style: none; padding: 0; margin: 0; }
  .models li { display: flex; justify-content: space-between; align-items: center; padding: 6px 10px; background: #313244; border-radius: 6px; margin-bottom: 4px; }
  .mname { font-size: 13px; }
  .ok { color: #a6e3a1; font-size: 13px; }
</style>
