<script lang="ts">
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { open } from '@tauri-apps/plugin-dialog';
  import { getCurrentWebview } from '@tauri-apps/api/webview';

  let activeTab = $state<'main' | 'logs'>('main');
  let modelDir = $state('D:\\nn\\models\\stt\\gigaam-v3');
  let dropFiles = $state<string[]>([]);
  let status = $state('');
  let busy = $state(false);
  let dragOverZone = $state(false);
  
  // Состояния для прогресса и результатов
  let currentProgress = $state(0);
  let totalProgress = $state(0);
  let combinedResult = $state('');

  let draggedIdx = $state<number | null>(null);
  let logs = $state<string[]>([]);

  function addPaths(paths: string[]) {
    const filtered = paths.filter(Boolean);
    if (filtered.length) dropFiles = [...new Set([...dropFiles, ...filtered])];
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

    return () => {
      unlistenDrop.then(fn => fn());
      unlistenLog.then(fn => fn());
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
        
        // Отправляем по одному файлу и ждем результат
        const res = await invoke<string[]>('recognize', { paths: [path] });
        const text = res[0] || '[Результат пуст]';
        
        // Добавляем результат в единое поле (интерфейс сам обновится)
        combinedResult += `=== ${filename} ===\n${text}\n\n`;
        currentProgress = i + 1;
      }
      
      status = 'готово';
    } catch (e) {
      status = 'ошибка: ' + String(e);
    } finally {
      busy = false;
    }
  }

  function copyResult() {
    if (!combinedResult) return;
    navigator.clipboard.writeText(combinedResult.trim());
    
    const oldStatus = status;
    status = 'скопировано!';
    setTimeout(() => {
      if (status === 'скопировано!') status = oldStatus;
    }, 2000);
  }

  // File sorting and remove logic
  function removeFile(i: number) {
    dropFiles = dropFiles.filter((_, idx) => idx !== i);
  }
  function moveUp(i: number) {
    if (i > 0) {
      const newFiles = [...dropFiles];
      [newFiles[i - 1], newFiles[i]] = [newFiles[i], newFiles[i - 1]];
      dropFiles = newFiles;
    }
  }
  function moveDown(i: number) {
    if (i < dropFiles.length - 1) {
      const newFiles = [...dropFiles];
      [newFiles[i + 1], newFiles[i]] = [newFiles[i], newFiles[i + 1]];
      dropFiles = newFiles;
    }
  }

  // Drag and drop sorting events
  function handleDragStart(e: DragEvent, i: number) {
    draggedIdx = i;
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function handleDragOver(e: DragEvent, i: number) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  }
  function handleDrop(e: DragEvent, i: number) {
    e.preventDefault();
    if (draggedIdx !== null && draggedIdx !== i) {
      const newFiles = [...dropFiles];
      const [moved] = newFiles.splice(draggedIdx, 1);
      newFiles.splice(i, 0, moved);
      dropFiles = newFiles;
    }
    draggedIdx = null;
  }

  function copyLogs() {
    navigator.clipboard.writeText(logs.join('\n'));
  }
</script>

<main>
  <h1>SpeechLab — ASR</h1>

  <div class="tabs">
    <button class:active={activeTab === 'main'} onclick={() => activeTab = 'main'}>Распознавание</button>
    <button class:active={activeTab === 'logs'} onclick={() => activeTab = 'logs'}>Логи ({logs.length})</button>
  </div>

  {#if activeTab === 'main'}
    <section class="model" aria-label="Настройки модели">
      <label for="model_input">Путь к модели (gigaam-v3):</label>
      <div class="row">
        <input id="model_input" bind:value={modelDir} placeholder="D:\nn\models\stt\gigaam-v3" />
        <button onclick={pickFolder}>выбрать папку</button>
        <button onclick={loadModel} disabled={busy}>загрузить модель</button>
      </div>
    </section>

    <div
      class="dropzone"
      class:over={dragOverZone}
      aria-label="Зона загрузки файлов"
    >
      <p>Перетащи сюда аудиофайлы (несколько сразу) <br/> или</p>
      <button onclick={pickFiles}>выбрать файлы</button>
    </div>

    {#if dropFiles.length}
      <ul class="files">
        {#each dropFiles as f, i}
          <li
            draggable="true"
            ondragstart={(e) => handleDragStart(e, i)}
            ondragover={(e) => handleDragOver(e, i)}
            ondrop={(e) => handleDrop(e, i)}
            class:drag-active={draggedIdx === i}
          >
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
      <button class="primary" onclick={recognize} disabled={busy || dropFiles.length === 0}>
        распознать
      </button>
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
  {:else}
    <section class="logs-section">
      <div class="row">
        <button onclick={copyLogs}>Копировать логи</button>
        <button onclick={() => logs = []}>Очистить</button>
      </div>
      <!-- Поле с логами -->
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
  
  .dropzone { border: 2px dashed #585b70; border-radius: 10px; padding: 28px; text-align: center; transition: 0.15s; outline: none; margin-bottom: 18px; }
  .dropzone.over { border-color: #89b4fa; background: #313244; }
  
  .files { list-style: none; padding: 0; margin: 0; margin-bottom: 18px; }
  .files li { 
    display: flex; justify-content: space-between; align-items: center; 
    padding: 6px 10px; background: #313244; border-radius: 6px; margin-bottom: 4px; 
    cursor: grab;
  }
  .files li:active { cursor: grabbing; }
  .files li.drag-active { opacity: 0.4; }
  
  .fname { font-size: 13px; word-break: break-all; }
  .factions { display: flex; gap: 4px; align-items: center; }
  .factions button { padding: 2px 6px; background: transparent; font-size: 12px; opacity: 0.6; }
  .factions button:hover:not(:disabled) { opacity: 1; background: #45475a; }
  .factions button.x { color: #f38ba8; font-size: 14px; }
  
  .status { opacity: 0.8; font-size: 14px; margin-left: 8px; }

  /* Прогресс-бар */
  .progress-wrap {
    height: 6px;
    background: #313244;
    border-radius: 4px;
    margin: 15px 0 20px;
    overflow: hidden;
    width: 100%;
  }
  .progress-bar {
    height: 100%;
    background: #89b4fa;
    transition: width 0.3s ease;
  }
  
  .results { margin-top: 20px; }
  .results-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 10px;
  }
  .results-header h2 {
    margin: 0;
    font-size: 18px;
  }
  .single-result { 
    width: 100%; box-sizing: border-box; background: #1e1e2e; color: #a6adc8; 
    border: 1px solid #45475a; border-radius: 8px; padding: 14px; 
    font-family: system-ui, sans-serif; font-size: 14px; resize: vertical; line-height: 1.5; outline: none;
    height: 250px;
  }

  .logs-section { display: flex; flex-direction: column; height: 70vh; }
  .logs-area { flex: 1; background: #11111b; color: #a6adc8; border: 1px solid #45475a; border-radius: 6px; padding: 12px; font-family: monospace; font-size: 13px; resize: none; outline: none; }
</style>