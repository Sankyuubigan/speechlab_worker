<script lang="ts">
  // Вкладка «Диалог»: full-duplex разговор с Haru через движок speech.cpp.
  // Движок/настройки — команды плагина (plugin:speech|dialog_*),
  // сессия (mic+WS+playback) — команды хоста (dialog_start/dialog_stop).
  import { onMount } from 'svelte';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  type DlgSettings = {
    engine_exe?: string
    model?: string
    port?: number
    language?: string
    name?: string
    persona?: string
    voice?: string
    seed?: number
    speak_first?: boolean
    max_seconds?: number
  };

  // Языки карточки персонажа, поддерживаемые Haru (docs модели).
  const LANGUAGES = ['ru', 'en', 'de', 'es', 'fr', 'ja', 'ko', 'pt', 'zh'];
  const LANG_LABELS: Record<string, string> = {
    ru: 'Русский', en: 'English', de: 'Deutsch', es: 'Español', fr: 'Français',
    ja: '日本語', ko: '한국어', pt: 'Português', zh: '中文',
  };

  let settings = $state<DlgSettings>({ language: 'ru', name: '', persona: '', seed: 0, speak_first: false, max_seconds: 0 });
  let sessionState = $state('stopped'); // connecting | active | stopped | error
  let engineState = $state('stopped'); // статус движка из плагина (dialog-status)
  let detail = $state('');
  let engineBusy = $state(false);
  let engineMsg = $state('');
  let dlEngine = $state<{ current: number; total: number } | null>(null);
  let messages = $state<{ who: 'haru' | 'system'; text: string }[]>([]);
  let speaking = $state(false);
  let feedEl: HTMLElement | undefined = $state(undefined);
  let unlisteners: (() => void)[] = [];

  const active = $derived(sessionState === 'connecting' || sessionState === 'active');
  const engineProgress = $derived(
    dlEngine && dlEngine.total ? Math.min(100, (dlEngine.current / dlEngine.total) * 100) : 0
  );

  const sessionText = $derived(
    engineState === 'starting' ? 'Запуск движка... (первый раз грузит модель ~6.6 GB, это минуты)' :
    sessionState === 'connecting' ? 'Подключение к сессии...' :
    sessionState === 'active' ? 'Разговор идёт — говорите!' :
    sessionState === 'error' ? `Ошибка: ${detail}` :
    'Остановлен'
  );

  async function loadSettings() {
    try { settings = await invoke('plugin:speech|dialog_get_settings'); } catch { /* дефолты */ }
  }

  async function saveSettings() {
    try { await invoke('plugin:speech|dialog_save_settings', { settings }); } catch { /* */ }
  }

  async function downloadEngine() {
    engineBusy = true;
    engineMsg = 'Скачиваю движок speech.cpp (SHA-256 проверяется)...';
    try {
      const exe = await invoke<string>('plugin:speech|dialog_download_engine', { destDir: '' });
      engineMsg = `Движок установлен: ${exe}`;
    } catch (e) {
      engineMsg = `ОШИБКА скачивания движка: ${e}`;
    }
    engineBusy = false;
  }

  async function startDialog() {
    detail = '';
    try {
      await saveSettings();
      messages = [...messages, { who: 'system', text: '— запуск диалога —' }];
      await invoke('dialog_start');
    } catch (e) {
      sessionState = 'error';
      detail = String(e);
      messages = [...messages, { who: 'system', text: `ОШИБКА: ${e}` }];
    }
  }

  async function stopDialog() {
    try { await invoke('dialog_stop'); } catch (e) { detail = String(e); }
  }

  // Сервер шлёт `text` перед КАЖДЫМ аудио-кадром — либо дельту, либо
  // нарастающий текст. Нарастающий определяем по startsWith и заменяем,
  // иначе дописываем в конец реплики.
  function pushHaruText(t: string) {
    if (!t) return;
    const arr = [...messages];
    const last = arr[arr.length - 1];
    if (last && last.who === 'haru' && t.startsWith(last.text)) {
      arr[arr.length - 1] = { who: 'haru', text: t };
    } else {
      arr.push({ who: 'haru', text: t });
    }
    messages = arr;
    speaking = true;
    setTimeout(() => { speaking = false; }, 1200);
  }

  $effect(() => {
    if (feedEl) feedEl.scrollTop = feedEl.scrollHeight;
  });

  onMount(() => {
    loadSettings();
    const setup = async () => {
      unlisteners.push(await listen<{ state: string; detail: string }>('dialog://status', (ev) => {
        sessionState = ev.payload.state;
        detail = ev.payload.detail || '';
      }));
      unlisteners.push(await listen<string>('dialog-status', (ev) => {
        engineState = ev.payload;
      }));
      unlisteners.push(await listen<{ text: string }>('dialog://text', (ev) => pushHaruText(ev.payload.text)));
      unlisteners.push(await listen<{ kind: string; current: number; total: number }>('downloader:progress', (ev) => {
        if (ev.payload.kind !== 'engine') return;
        dlEngine = { current: ev.payload.current, total: ev.payload.total };
      }));
    };
    setup();
    return () => {
      unlisteners.forEach((u) => u());
      unlisteners = [];
    };
  });
</script>

<section class="dialog" aria-label="Диалог с Haru">
  <div class="warn">
    🎧 Используй наушники! Эхоподавления нет — из динамиков модель услышит
    саму себя и разговор сломается.
  </div>

  <div class="row controls">
    {#if !active}
      <button class="primary" onclick={startDialog} disabled={engineBusy}>Начать разговор</button>
    {:else}
      <button class="stop" onclick={stopDialog}>Завершить</button>
    {/if}
    <button onclick={downloadEngine} disabled={engineBusy || active}>Скачать движок</button>
    <span class="status" class:err={sessionState === 'error'}>{sessionText}</span>
  </div>

  {#if engineBusy || dlEngine}
    <div class="progress-wrap">
      <div class="progress-bar" style="width: {engineProgress}%"></div>
    </div>
    {#if engineMsg}<div class="engine-msg">{engineMsg}</div>{/if}
  {:else if engineMsg}
    <div class="engine-msg">{engineMsg}</div>
  {/if}

  <details class="card">
    <summary>Карточка персонажа и настройки</summary>
    <div class="row">
      <label for="dlg_lang">Язык:</label>
      <select id="dlg_lang" bind:value={settings.language}>
        {#each LANGUAGES as l}
          <option value={l}>{LANG_LABELS[l] ?? l}</option>
        {/each}
      </select>
      <label for="dlg_name">Имя:</label>
      <input id="dlg_name" bind:value={settings.name} placeholder="Haru" />
      <label for="dlg_seed">Seed (0 = авто):</label>
      <input id="dlg_seed" type="number" bind:value={settings.seed} />
    </div>
    <div class="row">
      <label for="dlg_persona">Персона:</label>
    </div>
    <textarea id="dlg_persona" rows="3" bind:value={settings.persona}
      placeholder="Например: дружелюбная собеседница, говорит живо и коротко"></textarea>
    <div class="row">
      <label>
        <input type="checkbox" bind:checked={settings.speak_first} />
        Персонаж говорит первым
      </label>
      <label for="dlg_maxsec">Лимит ответа, сек (0 = нет):</label>
      <input id="dlg_maxsec" type="number" min="0" bind:value={settings.max_seconds} />
    </div>
    <div class="row">
      <span class="hint">Модель: {settings.model || 'haru-4b:q8_0'} · голос: {settings.voice || 'по умолчанию'}</span>
    </div>
  </details>

  <div class="feed" bind:this={feedEl} aria-live="polite">
    {#if messages.length === 0}
      <div class="empty">Реплики Haru появятся здесь во время разговора.</div>
    {/if}
    {#each messages as m}
      <div class="msg {m.who}">
        <span class="who">{m.who === 'haru' ? 'Haru' : '•'}</span>
        <span class="text">{m.text}</span>
      </div>
    {/each}
  </div>
  <div class="mic-indicator" class:on={speaking}>🎙</div>
</section>

<style>
  .dialog { display: flex; flex-direction: column; gap: 10px; }
  .warn {
    background: #3a2f1b; border: 1px solid #f9e2af; color: #f9e2af;
    padding: 8px 12px; border-radius: 8px; font-size: 13px;
  }
  .row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .controls { margin-top: 4px; }
  .status { font-size: 13px; color: #a6adc8; }
  .status.err { color: #f38ba8; }
  .progress-wrap {
    height: 8px; background: #313244; border-radius: 4px; overflow: hidden;
  }
  .progress-bar { height: 100%; background: #89b4fa; transition: width 0.3s; }
  .engine-msg { font-size: 12px; color: #9399b2; word-break: break-all; }
  .card {
    background: #1e1e2e; border: 1px solid #45475a; border-radius: 8px;
    padding: 8px 12px; font-size: 13px;
  }
  .card summary { cursor: pointer; color: #cdd6f4; }
  .card textarea {
    width: 100%; background: #181825; color: #cdd6f4; border: 1px solid #45475a;
    border-radius: 6px; padding: 6px 8px; resize: vertical; font: inherit;
  }
  .card input, .card select {
    background: #181825; color: #cdd6f4; border: 1px solid #45475a;
    border-radius: 6px; padding: 4px 8px;
  }
  .card label { color: #a6adc8; }
  .hint { color: #6c7086; font-size: 12px; }
  .feed {
    background: #181825; border: 1px solid #45475a; border-radius: 8px;
    min-height: 160px; max-height: 320px; overflow-y: auto; padding: 8px 10px;
    display: flex; flex-direction: column; gap: 6px;
  }
  .empty { color: #6c7086; font-size: 13px; }
  .msg { display: flex; gap: 8px; font-size: 14px; color: #cdd6f4; }
  .msg.system { color: #6c7086; font-style: italic; }
  .msg .who { color: #89b4fa; min-width: 40px; }
  .msg.system .who { color: #6c7086; }
  .mic-indicator { font-size: 20px; opacity: 0.25; transition: opacity 0.2s; }
  .mic-indicator.on { opacity: 1; }
</style>
