<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import VoiceCard from './VoiceCard.svelte';
  import VoiceEditor from './VoiceEditor.svelte';
  import type { Voice } from './voiceTypes';

  let {
    voices,
    modelsDir,
    onChanged,
    onUseInTts,
  }: {
    voices: Voice[];
    modelsDir: string;
    onChanged: () => void;
    onUseInTts: (id: string) => void;
  } = $props();

  let editorOpen = $state(false);
  let editingVoice = $state<Voice | null>(null);
  let status = $state('');

  function openAdd() {
    editingVoice = null;
    editorOpen = true;
  }
  function openEdit(v: Voice) {
    editingVoice = v;
    editorOpen = true;
  }
  async function doDelete(id: string) {
    if (!confirm('Удалить голос безвозвратно?')) return;
    try {
      await invoke('tts_delete_voice', { modelsDir, id });
      status = 'голос удалён';
      onChanged();
    } catch (e) {
      status = 'ошибка удаления: ' + String(e);
    }
  }
  function doSave(v: Voice) {
    editorOpen = false;
    onChanged();
    onUseInTts(v.id);
    status = 'голос сохранён';
  }
</script>

<section class="voice-storage">
  <div class="vs-head">
    <h3>Хранилище голосов</h3>
    <button class="primary" onclick={openAdd}>добавить голос</button>
  </div>
  <p class="hint">
    Здесь хранятся ваши голоса для клонирования. Нажмите «ТТС» на карточке, чтобы сразу
    применить голос в синтезе речи.
  </p>

  {#if voices.length === 0}
    <p class="hint warn">Пока нет сохранённых голосов — нажмите «добавить голос».</p>
  {:else}
    <div class="voice-grid">
      {#each voices as v (v.id)}
        <VoiceCard voice={v} {modelsDir} onEdit={openEdit} onDelete={doDelete} onUse={onUseInTts} />
      {/each}
    </div>
  {/if}

  <span class="status">{status}</span>

  {#if editorOpen}
    <VoiceEditor {modelsDir} voice={editingVoice} onClose={() => (editorOpen = false)} onSaved={doSave} />
  {/if}
</section>

<style>
  .voice-storage {
    margin-top: 8px;
  }
  .vs-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .vs-head h3 {
    font-size: 15px;
    margin: 14px 0 8px;
    opacity: 0.85;
    color: #cdd6f4;
  }
  .voice-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 10px;
    margin-top: 6px;
  }
  .hint {
    opacity: 0.65;
    font-size: 13px;
    margin: 4px 0 10px;
    color: #cdd6f4;
  }
  .hint.warn {
    color: #f9e2af;
  }
  .status {
    opacity: 0.8;
    font-size: 13px;
    margin-left: 8px;
    color: #cdd6f4;
  }
  button.primary {
    background: #89b4fa;
    color: #1e1e2e;
    font-weight: 600;
    padding: 8px 14px;
    border-radius: 6px;
    border: none;
    cursor: pointer;
  }
  button.primary:hover {
    background: #74a0f0;
  }
</style>
