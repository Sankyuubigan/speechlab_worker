//! VAD (Voice Activity Detection).
//!
//! В onnx_asr silero VAD подгружается автоматически. У пользователя отдельного
//! silero.onnx в папке модели нет, поэтому реализуем два режима:
//! 1. Если указан путь к silero.onnx — грузим через ort и сегментируем (TODO).
//! 2. Иначе (по умолчанию) — нарезаем аудио на куски по `chunk_sec` секунд,
//!    чтобы не превышать лимит модели (~25с) и иметь примерную сегментацию.
//!
//! Этот модуль пока предоставляет только разбиение по длине. Тонкая VAD-сегментация
//! через silero будет добавлена отдельным этапом.

/// Разбивает моно f32 (16kHz) на чанки по `chunk_sec` секунд.
pub fn split_by_length(samples: &[f32], sample_rate: u32, chunk_sec: f32) -> Vec<Vec<f32>> {
    let chunk_size = (sample_rate as f32 * chunk_sec) as usize;
    if chunk_size == 0 {
        return vec![samples.to_vec()];
    }
    samples
        .chunks(chunk_size)
        .map(|c| c.to_vec())
        .collect()
}
