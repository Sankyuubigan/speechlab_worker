//! VAD (Voice Activity Detection).
//!
//! Вместо жесткого обрезания ровно по 25.0 секундам (что может разрезать слово
//! пополам и сильно ухудшить качество CTC/RNNT декодинга), здесь реализовано
//! простое разбиение на основе минимальной энергии (тишины).

/// Разбивает моно f32 (16kHz) на чанки не превышающие `max_chunk_sec` секунд.
/// Ищет самую тихую паузу в диапазоне от 10 секунд до `max_chunk_sec`.
pub fn split_by_length(samples: &[f32], sample_rate: u32, max_chunk_sec: f32) -> Vec<Vec<f32>> {
    let max_chunk_size = (sample_rate as f32 * max_chunk_sec) as usize;
    if samples.len() <= max_chunk_size || max_chunk_size == 0 {
        return vec![samples.to_vec()];
    }

    let mut result = Vec::new();
    let mut start = 0;

    // Окно для поиска тишины — 300 мс
    let window_size = (sample_rate as f32 * 0.3) as usize; 
    let mut min_chunk_size = (sample_rate as f32 * 10.0) as usize;
    
    // Защита: если передадут слишком маленький max_chunk_sec
    if min_chunk_size + window_size >= max_chunk_size {
        min_chunk_size = max_chunk_size.saturating_sub(window_size * 2);
    }

    while start < samples.len() {
        let remaining = samples.len() - start;
        if remaining <= max_chunk_size {
            result.push(samples[start..].to_vec());
            break;
        }

        let search_start = start + min_chunk_size;
        let search_end = start + max_chunk_size;

        let mut best_split_point = search_end;

        if search_end > search_start + window_size {
            // Вычисляем энергию первого окна в f64 для предотвращения накопления погрешности
            let mut current_energy: f64 = samples[search_start..search_start + window_size]
                .iter()
                .map(|&s| (s as f64) * (s as f64))
                .sum();
            
            let mut min_energy = current_energy;
            best_split_point = search_start + window_size / 2;

            // Скользящее окно (быстрый O(N) алгоритм)
            for i in search_start + 1..=search_end - window_size {
                let old_s = samples[i - 1] as f64;
                let new_s = samples[i + window_size - 1] as f64;
                current_energy = current_energy - old_s * old_s + new_s * new_s;
                
                if current_energy < min_energy {
                    min_energy = current_energy;
                    best_split_point = i + window_size / 2;
                }
            }
        }

        result.push(samples[start..best_split_point].to_vec());
        start = best_split_point;
    }

    result
}