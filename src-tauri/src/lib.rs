mod modules;

use std::sync::Arc;
use tokio::sync::Mutex;
use tauri::{AppHandle, Emitter, State};

use modules::asr::gigaam::ModelRunner;

pub struct AppState {
    pub model: Mutex<Option<Arc<ModelRunner>>>,
    pub model_dir: Mutex<String>,
}

fn emit_log(app: &AppHandle, msg: &str) {
    println!("{}", msg);
    let _ = app.emit("app-log", msg);
}

#[tauri::command]
async fn set_model_dir(state: State<'_, AppState>, dir: String) -> Result<(), String> {
    let mut current_dir = state.model_dir.lock().await;
    if *current_dir != dir {
        *current_dir = dir;
        // Сбрасываем модель только если путь изменился
        *state.model.lock().await = None;
    }
    Ok(())
}

#[tauri::command]
async fn load_model(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let dir = state.model_dir.lock().await.clone();
    if dir.is_empty() {
        emit_log(&app, "ОШИБКА: путь к модели не указан");
        return Err("путь к модели не указан".into());
    }

    emit_log(&app, &format!("Начинаю загрузку модели из {}...", dir));
    
    // Клонируем данные для передачи в spawn_blocking
    let dir_clone = dir.clone();
    let app_clone = app.clone();

    // Загрузка модели - тяжелая операция, выполняем в отдельном потоке
    let model = tokio::task::spawn_blocking(move || {
        ModelRunner::load(&dir_clone)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| {
        emit_log(&app_clone, &format!("ОШИБКА загрузки модели: {}", e));
        e.to_string()
    })?;

    *state.model.lock().await = Some(Arc::new(model));
    emit_log(&app, "Модель успешно загружена в память.");

    Ok(format!("модель успешно загружена из {dir}"))
}

#[tauri::command]
async fn recognize(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> Result<Vec<String>, String> {
    // 1. Получаем или загружаем модель
    let arc_model = {
        let mut guard = state.model.lock().await;
        if guard.is_none() {
            emit_log(&app, "Модель не загружена. Загружаю автоматически перед распознаванием...");
            let dir = state.model_dir.lock().await.clone();
            
            let dir_clone = dir.clone();
            let app_clone = app.clone();
            
            let model = tokio::task::spawn_blocking(move || {
                ModelRunner::load(&dir_clone)
            })
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| {
                emit_log(&app_clone, &format!("ОШИБКА автозагрузки модели: {}", e));
                e.to_string()
            })?;
            
            *guard = Some(Arc::new(model));
            emit_log(&app, "Автозагрузка модели завершена.");
        }
        guard.as_ref().unwrap().clone()
    };

    let mut results = Vec::with_capacity(paths.len());

    // 2. Распознаем файлы (можно запускать параллельно, но пока делаем последовательно)
    for p in paths {
        emit_log(&app, &format!("--- Распознаю файл: {} ---", p));
        
        let p_clone = p.clone();
        let model = arc_model.clone();
        
        // Само распознавание — это долгий синхронный процесс (ONNX inference)
        let res = tokio::task::spawn_blocking(move || {
            model.recognize_file(&p_clone)
        })
        .await
        .map_err(|e| e.to_string())?;

        match res {
            Ok(text) => {
                emit_log(&app, &format!("УСПЕХ ({}): {}", p, text));
                results.push(text);
            }
            Err(e) => {
                emit_log(&app, &format!("ОШИБКА ({}): {}", p, e));
                results.push(format!("[ОШИБКА: {}]", e));
            }
        }
    }

    emit_log(&app, "✅ Все файлы обработаны.");
    Ok(results)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            model: Mutex::new(None),
            model_dir: Mutex::new(String::new()),
        })
        .invoke_handler(tauri::generate_handler![
            set_model_dir,
            load_model,
            recognize
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}