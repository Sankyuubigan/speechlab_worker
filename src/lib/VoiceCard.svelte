<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { Voice } from './voiceTypes';

  let {
    voice,
    modelsDir,
    onEdit,
    onDelete,
    onUse,
  }: {
    voice: Voice;
    modelsDir: string;
    onEdit: (v: Voice) => void;
    onDelete: (id: string) => void;
    onUse: (id: string) => void;
  } = $props();

  let avatarUrl = $state<string | null>(null);
  let playing = $state(false);
  let audioEl: HTMLAudioElement | undefined = $state(undefined);

  $effect(() => {
    let url: string | null = null;
    if (voice.has_avatar) {
      invoke<number[]>('tts_voice_avatar', { modelsDir, id: voice.id })
        .then((b) => {
          if (b && b.length) {
            const blob = new Blob([new Uint8Array(b)], { type: 'image/jpeg' });
            url = URL.createObjectURL(blob);
            avatarUrl = url;
          }
        })
        .catch(() => {});
    }
    return () => {
      if (url) URL.revokeObjectURL(url);
    };
  });

  async function play() {
    try {
      const b = await invoke<number[]>('tts_voice_audio', { modelsDir, id: voice.id });
      const blob = new Blob([new Uint8Array(b)], { type: 'audio/wav' });
      const url = URL.createObjectURL(blob);
      if (audioEl) {
        audioEl.src = url;
        audioEl.onended = () => {
          playing = false;
          URL.revokeObjectURL(url);
        };
        playing = true;
        await audioEl.play();
      }
    } catch (e) {
      console.error(e);
    }
  }

  function fmtDate(s: string): string {
    if (!s) return '';
    return s.replace('T', ' ').replace('Z', '').slice(0, 16);
  }
</script>

<div class="voice-card">
  <div class="vc-avatar">
    {#if avatarUrl}
      <img src={avatarUrl} alt={voice.name} />
    {:else}
      <div class="vc-avatar-ph">{voice.name.slice(0, 1).toUpperCase()}</div>
    {/if}
  </div>

  <div class="vc-body">
    <div class="vc-name">{voice.name}</div>
    <div class="vc-ref">{voice.ref_text || '— нет референсного текста —'}</div>
    {#if voice.created_at}<div class="vc-date">{fmtDate(voice.created_at)}</div>{/if}
  </div>

  <div class="vc-actions">
    <button class="vc-play" onclick={play} disabled={playing} title="прослушать референс">▶</button>
    <button class="vc-use" onclick={() => onUse(voice.id)} title="использовать в ТТС">ТТС</button>
    <button class="vc-edit" onclick={() => onEdit(voice)} title="изменить">✎</button>
    <button class="vc-del" onclick={() => onDelete(voice.id)} title="удалить">✕</button>
  </div>

  <audio bind:this={audioEl}></audio>
</div>

<style>
  .voice-card {
    display: flex;
    gap: 12px;
    align-items: center;
    background: #313244;
    border: 1px solid #45475a;
    border-radius: 10px;
    padding: 10px 12px;
  }
  .vc-avatar {
    flex: 0 0 auto;
    width: 52px;
    height: 52px;
    border-radius: 50%;
    overflow: hidden;
    background: #45475a;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .vc-avatar img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .vc-avatar-ph {
    font-size: 22px;
    font-weight: 700;
    color: #cdd6f4;
  }
  .vc-body {
    flex: 1 1 auto;
    min-width: 0;
  }
  .vc-name {
    font-size: 14px;
    font-weight: 600;
    color: #cdd6f4;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .vc-ref {
    font-size: 12px;
    opacity: 0.7;
    color: #cdd6f4;
    margin-top: 2px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .vc-date {
    font-size: 11px;
    opacity: 0.5;
    margin-top: 3px;
  }
  .vc-actions {
    flex: 0 0 auto;
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .vc-actions button {
    padding: 4px 9px;
    font-size: 12px;
  }
  .vc-play {
    background: #89b4fa;
    color: #1e1e2e;
    border: none;
    border-radius: 6px;
    font-weight: 700;
  }
  .vc-play:hover:not(:disabled) {
    background: #74a0f0;
  }
  .vc-use {
    background: #45475a;
    color: #cdd6f4;
  }
  .vc-edit {
    background: transparent;
    color: #cdd6f4;
    opacity: 0.7;
  }
  .vc-edit:hover {
    opacity: 1;
    background: #45475a;
  }
  .vc-del {
    background: transparent;
    color: #f38ba8;
    opacity: 0.8;
  }
  .vc-del:hover {
    opacity: 1;
    background: #45475a;
  }
</style>
