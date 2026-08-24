mod audio;
mod commands;
pub mod config;
#[allow(dead_code)] // tipos de domínio consumidos por 1.4/1.7/1.8/1.9 e Fase 3
pub mod domain;
mod errors;
mod ffmpeg;
#[allow(dead_code)] // format_subtitles consumido por 1.9 e Fase 3
pub mod format;
#[allow(dead_code)] // detect() consumido pela 2.6 (tier) e onboarding 6.4
pub mod hardware;
pub mod logging;
#[allow(dead_code)] // download_file/whisper_dir consumidos por 1.9 e Fase 2
mod model_manager;
#[cfg(feature = "stt")]
pub mod pipeline;
pub mod smoke;
#[allow(dead_code)] // JobStats consumido pelo PipelineSummary (4.3) e StatsPanel (5.5)
pub mod stats;
#[cfg(feature = "stt")]
pub mod stt;
#[allow(dead_code)] // to_srt/parse_srt consumidos por 1.8/1.9 e Fase 3
pub mod subtitles;
#[allow(dead_code)] // trait/engine consumidos pela Fase 3 (3.2-3.10)
pub mod translate;

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Retorna a primeira linha de `ffmpeg -version` (ex: `ffmpeg version n9.0 ...`).
#[tauri::command]
async fn ffmpeg_version(app: tauri::AppHandle) -> Result<String, String> {
    ffmpeg::run(&app, ffmpeg::FFMPEG, &["-version"])
        .await
        .map(|out| out.lines().next().unwrap_or_default().to_string())
        .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            match model_manager::catalog::Catalog::embedded() {
                Ok(cat) => {
                    tracing::info!(
                        "catálogo de modelos ok: {} entradas (schema v{})",
                        cat.models.len(),
                        cat.version
                    );
                    // Limpeza de locks órfãos após crash (status `downloading` sem processo vivo)
                    for m in &cat.models {
                        if let Ok(None) = model_manager::cache::effective_status(m) {
                            // effective_status já remove .lock stale; log apenas se havia estado preso
                            // (não loga para não poluir boot; download_model logará ao retomar)
                        }
                    }
                }
                Err(e) => tracing::error!("catálogo de modelos inválido: {e}"),
            }
            let cfg = config::AppConfig::load_or_default();
            tracing::info!(
                "config carregada (schema {}, {}→{}, engine {}) em {:?}",
                cfg.schema_version,
                cfg.source_lang,
                cfg.target_lang,
                cfg.translation_engine,
                config::AppConfig::config_path()
            );
            let hw = hardware::detect::detect();
            tracing::info!(
                "hardware: {} RAM, {} threads CPU ({} recomendadas), GPU {:?}, CPU {}",
                hw.ram_gb,
                hw.cpu_threads,
                hw.recommended_threads,
                hw.gpu,
                hw.cpu_name
            );
            let tier = hardware::tier::tier_for(&hw);
            tracing::info!("tier de hardware: {:?}", tier);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                for name in [ffmpeg::FFMPEG, ffmpeg::FFPROBE] {
                    match ffmpeg::binary_path(name) {
                        Ok(path) => {
                            tracing::info!("{name} sidecar resolvido em: {}", path.display())
                        }
                        Err(e) => tracing::warn!("{name} sidecar não encontrado: {e}"),
                    }
                }
                match ffmpeg::run(&handle, ffmpeg::FFMPEG, &["-version"]).await {
                    Ok(out) => tracing::info!(
                        "ffmpeg sidecar ok: {}",
                        out.lines().next().unwrap_or_default()
                    ),
                    Err(e) => tracing::warn!("ffmpeg sidecar indisponível: {e}"),
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            ffmpeg_version,
            commands::app::get_app_info,
            commands::config::get_prefs,
            commands::config::set_prefs,
            commands::config::get_glossary,
            commands::config::set_glossary,
            commands::export::export_subtitle,
            commands::import::inspect_video,
            commands::models::list_catalog,
            commands::models::list_cache_status,
            commands::models::download_model,
            commands::models::cancel_download,
            commands::models::delete_model,
            commands::models::set_active_model,
            commands::models::get_active_models,
            commands::onboarding::get_onboarding,
            commands::preview::load_preview,
            commands::subtitles::save_subtitles,
            #[cfg(feature = "stt")]
            commands::pipeline::translate_subtitle,
            #[cfg(feature = "stt")]
            commands::pipeline::run_pipeline,
            #[cfg(feature = "stt")]
            commands::pipeline::cancel_pipeline,
            #[cfg(feature = "stt")]
            pipeline::queue::queue_list,
            #[cfg(feature = "stt")]
            pipeline::queue::queue_enqueue,
            #[cfg(feature = "stt")]
            pipeline::queue::queue_cancel,
            #[cfg(feature = "stt")]
            pipeline::queue::queue_remove
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
