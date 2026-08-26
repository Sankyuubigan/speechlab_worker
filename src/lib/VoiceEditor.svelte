<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { open } from '@tauri-apps/plugin-dialog';
  import type { Voice } from './voiceTypes';

  let {
    modelsDir,
    voice = null,
    onClose,
    onSaved,
  }: {
    modelsDir: string;
    voice?: Voice | null;
    onClose: () => void;
    onSaved: (v: Voice) => void;
  } = $props();

  const editing = $derived(voice !== null);

  let name = $state('');
  let refText = $state('');
  let audio = $state(''); // новый путь к аудио (для edit — замена)
  let avatar = $state(''); // новый путь к аватару
  let removeAvatar = $state(false);
  let busy = $state(false);
  let status = $state('');

  // Компонент монтируется заново при каждом открытии (App управляет editorOpen),
  // поэтому инициализируем поля из пропа voice в эффекте (без захвата начального значения).
  $effect(() => {
    name = voice?.name ?? '';
    refText = voice?.ref_text ?? '';
    audio = '';
    avatar = '';
    removeAvatar = false;
    status = '';
  });

  async function pickAudio() {
    const p = await open({ filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'ogg', 'flac', 'm4a', 'opus'] }] });
    if (p && typeof p === 'string') audio = p;
  }
  async function pickAvatar() {
    const p = await open({ filters: [{ name: 'Image', extensions: ['jpg', 'jpeg', 'png', 'webp'] }] });
    if (p && typeof p === 'string') {
      avatar = p;
      removeAvatar = false;
    }
  }

  async function submit() {
    if (!name.trim()) {
      status = 'введите имя голоса';
      return;
    }
    if (!editing && !audio) {
      status = 'выберите референсное аудио';
      return;
    }
    busy = true;
    try {
      let result: Voice;
      if (editing && voice) {
        const avatarArg = removeAvatar ? '__REMOVE__' : avatar;
        result = await invoke<Voice>('tts_update_voice', {
          modelsDir,
          id: voice.id,
          name,
          refText,
          avatar: avatarArg,
          srcAudio: audio,
        });
        status = 'голос обновлён';
      } else {
        result = await invoke<Voice>('tts_add_voice', {
          modelsDir,
          name,
          srcAudio: audio,
          refText,
          avatar,
        });
        status = 'голос добавлен';
      }
      onSaved(result);
    } catch (e) {
      status = 'ошибка: ' + String(e);
    } finally {
      busy = false;
    }
  }

  function baseName(p: string): string {
    return p.split('\\').pop()?.split('/').pop() || p;
  }
</script>

<div
  class="ve-overlay"
  role="presentation"
  onclick={(e) => { if (e.target === e.currentTarget) onClose(); }}
  onkeydown={(e) => { if (e.key === 'Escape') onClose(); }}
>
  <div class="ve-modal" role="dialog" aria-modal="true">
    <h3>{editing ? 'Изменить голос' : 'Новый голос'}</h3>

    <label for="ve_name">Имя голоса:</label>
    <input id="ve_name" bind:value={name} placeholder="напр. Morgan Freeman" />

    <label for="ve_audio">Референсное аудио {editing ? '(опц. — чтобы заменить)' : ''}:</label>
    <div class="row">
      <button onclick={pickAudio}>выбрать аудио</button>
      <span class="status">{audio ? baseName(audio) : editing ? 'без изменений' : 'не выбрано'}</span>
    </div>

    <label for="ve_text">Референсный текст (опц., улучшает качество):</label>
    <textarea id="ve_text" bind:value={refText} placeholder="что говорится в аудио"></textarea>

    <label for="ve_avatar">Аватар (опц.):</label>
    <div class="row">
      <button onclick={pickAvatar}>выбрать картинку</button>
      <span class="status">
        {avatar ? baseName(avatar) : editing ? (removeAvatar ? 'будет удалён' : 'без изменений') : 'не выбрана'}
      </span>
      {#if editing && voice?.has_avatar}
        <button class="small" onclick={() => { removeAvatar = !removeAvatar; avatar = ''; }}>
          {removeAvatar ? 'не удалять' : 'сбросить аватар'}
        </button>
      {/if}
    </div>

    <div class="row">
      <button class="primary" onclick={submit} disabled={busy}>сохранить</button>
      <button onclick={onClose}>отмена</button>
      <span class="status">{status}</span>
    </div>
  </div>
</div>

<style>
  .ve-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 50;
    padding: 16px;
  }
  .ve-modal {
    background: #181825;
    border: 1px solid #45475a;
    border-radius: 12px;
    padding: 18px 20px;
    width: 100%;
    max-width: 440px;
    max-height: 90vh;
    overflow-y: auto;
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.5);
  }
  .ve-modal h3 {
    margin: 0 0 12px;
    font-size: 17px;
    color: #cdd6f4;
  }
  label {
    display: block;
    margin: 10px 0 6px;
    opacity: 0.8;
    color: #cdd6f4;
  }
  input,
  textarea {
    width: 100%;
    box-sizing: border-box;
    padding: 8px;
    border-radius: 6px;
    border: 1px solid #45475a;
    background: #313244;
    color: #cdd6f4;
  }
  textarea {
    min-height: 64px;
    resize: vertical;
    font-family: system-ui, sans-serif;
  }
  input::placeholder,
  textarea::placeholder {
    color: #7f849c;
  }
  .row {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
    margin-bottom: 4px;
  }
  .status {
    opacity: 0.8;
    font-size: 13px;
    color: #cdd6f4;
  }
  button {
    padding: 8px 14px;
    border-radius: 6px;
    border: none;
    background: #45475a;
    color: #cdd6f4;
    cursor: pointer;
  }
  button:hover:not(:disabled) {
    background: #585b70;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button.primary {
    background: #89b4fa;
    color: #1e1e2e;
    font-weight: 600;
  }
  button.primary:hover:not(:disabled) {
    background: #74a0f0;
  }
  button.small {
    padding: 4px 10px;
    font-size: 12px;
  }
</style>
