//! Comandos IPC da UI de gerenciamento de modelos (tarefas 2.8, 2.9 e 2.10).
//!
//! 2.8: catálogo curado (2.1) e status de download de cada modelo via cache
//! (2.4). 2.9: ações de download/remoção com progresso e cancelamento via
//! eventos Tauri (sem polling). 2.10: seleção de modelo ativo (um STT e um de
//! tradução) persistida em `AppConfig.active_models`. Consumido por
//! `src/components/models/`.
//!
//! Fluxo do download: `download_model` valida, adquire o lock do cache (2.4),
//! grava `status=downloading` e dispara a tarefa em background. A tarefa baixa
//! todos os arquivos do `ModelInfo::files` (ou só o principal), verifica o
//! checksum (2.3), grava o status final e emite `model-download-finished`.
//! Progresso em tempo real via evento `model-download-progress { id, bytes,
//! total }`. Cancelamento é cooperativo (CancellationToken da 2.2) e deixa o
//! `.part` para retomada futura.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::model_manager::{cache, catalog, checksum, download};

use cache::{CacheStatus, ModelStatus};
use catalog::{Catalog, ModelInfo, ModelKind};

/// Status de download de um modelo para a UI. `None` = nunca baixado.
#[derive(Debug, Clone, Serialize)]
pub struct ModelCacheStatus {
    pub model_id: String,
    pub status: Option<CacheStatus>,
}

/// Payload do evento `model-download-progress` (emitido a cada chunk baixado).
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub model_id: String,
    pub file: String,
    pub bytes: u64,
    pub total: u64,
}

/// Payload do evento `model-download-finished` (término: sucesso/erro/cancelado).
#[derive(Debug, Clone, Serialize)]
pub struct DownloadFinished {
    pub model_id: String,
    pub ok: bool,
}

/// Tokens de cancelamento dos downloads em andamento (um por modelo). A tarefa
/// remove a entrada ao encerrar; `cancel_download` cancela o token cooperativo.
static ACTIVE_DOWNLOADS: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();

fn active_downloads() -> &'static Mutex<HashMap<String, CancellationToken>> {
    ACTIVE_DOWNLOADS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Busca um modelo no catálogo embutido pelo `id`.
fn find_model(id: &str) -> Result<ModelInfo, String> {
    Catalog::embedded()
        .map_err(|e| e.to_string())?
        .models
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("modelo `{id}` não encontrado no catálogo"))
}

/// Lista o catálogo curado completo (modelos + metadados de exibição).
#[tauri::command(rename_all = "snake_case")]
pub fn list_catalog() -> Result<Catalog, String> {
    Catalog::embedded().map_err(|e| e.to_string())
}

/// Status de download de cada modelo do catálogo (lido do `status.json` da 2.4,
/// com correção de `downloading` stale após crash — ver `effective_status`).
#[tauri::command(rename_all = "snake_case")]
pub fn list_cache_status() -> Result<Vec<ModelCacheStatus>, String> {
    let cat = Catalog::embedded().map_err(|e| e.to_string())?;
    cat.models
        .iter()
        .map(|m| {
            cache::effective_status(m)
                .map(|status| ModelCacheStatus {
                    model_id: m.id.clone(),
                    status: status.map(|s| s.status),
                })
                .map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, _>>()
}

/// Inicia o download do modelo `id` em background. Progresso via evento
/// `model-download-progress`; término via `model-download-finished`. O comando
/// retorna assim que a tarefa é disparada (a UI não é bloqueada).
#[tauri::command(rename_all = "snake_case")]
pub fn download_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let model = find_model(&id)?;
    if active_downloads().lock().unwrap().contains_key(&id) {
        return Err(format!("download do modelo `{id}` já está em andamento"));
    }
    let lock = cache::acquire_download_lock(&model).map_err(|e| e.to_string())?;
    let dir = cache::model_dir(&model).map_err(|e| e.to_string())?;
    cache::write_status(
        &model,
        &ModelStatus {
            status: CacheStatus::Downloading,
            size_bytes: 0,
            sha256: None,
        },
    )
    .map_err(|e| e.to_string())?;

    let token = CancellationToken::new();
    active_downloads()
        .lock()
        .unwrap()
        .insert(id.clone(), token.clone());

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_download(&model, &dir, &token, &handle).await;
        active_downloads().lock().unwrap().remove(&id);
        drop(lock);

        if token.is_cancelled() {
            // Estado volta a "não baixado": sem status.json. O `.part` fica
            // para retomada futura (2.2).
            let _ = std::fs::remove_file(dir.join("status.json"));
            tracing::info!("download de `{}` cancelado pelo usuário", model.id);
        } else {
            match &result {
                Ok(()) => {
                    let size = std::fs::metadata(dir.join(&model.file))
                        .map(|m| m.len())
                        .unwrap_or(0);
                    if let Err(e) = cache::write_status(
                        &model,
                        &ModelStatus {
                            status: CacheStatus::Downloaded,
                            size_bytes: size,
                            sha256: model.sha256.clone(),
                        },
                    ) {
                        tracing::error!("falha ao gravar status de `{}`: {e}", model.id);
                    }
                    tracing::info!("modelo `{}` baixado e verificado", model.id);
                }
                Err(e) => {
                    if let Err(se) = cache::write_status(
                        &model,
                        &ModelStatus {
                            status: CacheStatus::Error,
                            size_bytes: 0,
                            sha256: None,
                        },
                    ) {
                        tracing::error!("falha ao gravar status de `{}`: {se}", model.id);
                    }
                    tracing::error!("download de `{}` falhou: {e}", model.id);
                }
            }
        }
        let _ = handle.emit(
            "model-download-finished",
            DownloadFinished {
                model_id: model.id.clone(),
                ok: result.is_ok(),
            },
        );
    });
    Ok(())
}

/// Baixa todos os arquivos do modelo (sequencial) verificando o checksum no fim.
async fn run_download(
    model: &ModelInfo,
    dir: &Path,
    token: &CancellationToken,
    app: &tauri::AppHandle,
) -> Result<(), String> {
    let files = if model.files.is_empty() {
        vec![model.file.clone()]
    } else {
        model.files.clone()
    };
    for file in &files {
        let id = model.id.clone();
        let f = file.clone();
        let handle = app.clone();
        download::download_model(&model.repo_id, file, dir, token, move |bytes, total| {
            let _ = handle.emit(
                "model-download-progress",
                DownloadProgress {
                    model_id: id.clone(),
                    file: f.clone(),
                    bytes,
                    total,
                },
            );
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    checksum::verify_model(model, dir).map_err(|e| e.to_string())?;
    Ok(())
}

/// Cancela o download do modelo `id` (cooperativo: para entre chunks e mantém
/// o `.part` consistente para retomada).
#[tauri::command(rename_all = "snake_case")]
pub fn cancel_download(id: String) -> Result<(), String> {
    let token = active_downloads()
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("nenhum download em andamento para o modelo `{id}`"))?;
    token.cancel();
    Ok(())
}

/// Remove o modelo `id` do cache (diretório inteiro: arquivos + status).
/// Falha se houver download em andamento.
#[tauri::command(rename_all = "snake_case")]
pub fn delete_model(id: String) -> Result<(), String> {
    let model = find_model(&id)?;
    if active_downloads().lock().unwrap().contains_key(&id) {
        return Err(format!(
            "não é possível remover o modelo `{id}` durante o download"
        ));
    }
    let dir = cache::model_dir(&model).map_err(|e| e.to_string())?;
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => {
            tracing::info!("modelo `{id}` removido do cache ({:?})", dir);
            Ok(())
        }
        // Modelo nunca baixado: já está "removido".
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(format!("falha ao remover o modelo `{id}`: {source}")),
    }
}

/// Valida a string de tipo de modelo recebida do frontend.
fn parse_kind(kind: &str) -> Result<ModelKind, String> {
    match kind {
        "stt" => Ok(ModelKind::Stt),
        "translation" => Ok(ModelKind::Translation),
        other => Err(format!(
            "tipo de modelo inválido: `{other}` (use `stt` ou `translation`)"
        )),
    }
}

fn kind_label(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Stt => "transcrição",
        ModelKind::Translation => "tradução",
    }
}

/// Marca o modelo na config e retorna um aviso (não erro) se ele ainda não
/// estiver baixado — o usuário pode ativá-lo antes de baixar, mas é avisado.
fn apply_active(cfg: &mut AppConfig, model: &ModelInfo) -> Option<String> {
    let warning = cache::resolve_model_path(model).err().map(|e| {
        format!(
            "O modelo ativo `{}` ainda não está baixado ({e}). Baixe-o antes de processar.",
            model.id
        )
    });
    match model.kind {
        ModelKind::Stt => cfg.active_models.stt = model.id.clone(),
        ModelKind::Translation => cfg.active_models.translation = model.id.clone(),
    }
    tracing::info!(
        "modelo ativo de {} = `{}`",
        kind_label(model.kind),
        model.id
    );
    warning
}

/// Define o modelo ativo de um tipo (um STT e um de tradução). Persistido em
/// `AppConfig.active_models` com o save atômico da 0.7. Retorna `Ok(Some(aviso))`
/// se o modelo ainda não estiver baixado (aviso, não erro silencioso).
#[tauri::command(rename_all = "snake_case")]
pub fn set_active_model(kind: String, id: String) -> Result<Option<String>, String> {
    let kind = parse_kind(&kind)?;
    let model = find_model(&id)?;
    if model.kind != kind {
        return Err(format!(
            "o modelo `{id}` é de {} e não pode ser ativo como {}",
            kind_label(model.kind),
            kind_label(kind)
        ));
    }
    let mut cfg = AppConfig::load_or_default();
    let warning = apply_active(&mut cfg, &model);
    cfg.save().map_err(|e| e.to_string())?;
    Ok(warning)
}

/// Modelos ativos persistidos na config (lidos para destacar a seleção na UI).
#[tauri::command(rename_all = "snake_case")]
pub fn get_active_models() -> Result<crate::config::ActiveModels, String> {
    Ok(AppConfig::load_or_default().active_models)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_model_acha_por_id_no_catalogo() {
        let m = find_model("whisper-tiny").expect("whisper-tiny deve existir");
        assert_eq!(m.kind, catalog::ModelKind::Stt);
        assert_eq!(m.repo_id, "ggerganov/whisper.cpp");
    }

    #[test]
    fn find_model_erro_claro_para_id_desconhecido() {
        let err = find_model("modelo-inexistente").unwrap_err();
        assert!(err.contains("não encontrado no catálogo"), "{err}");
    }

    #[test]
    fn parse_kind_aceita_stt_e_translation() {
        assert_eq!(parse_kind("stt").unwrap(), ModelKind::Stt);
        assert_eq!(parse_kind("translation").unwrap(), ModelKind::Translation);
    }

    #[test]
    fn parse_kind_rejeita_tipo_desconhecido() {
        let err = parse_kind("llm").unwrap_err();
        assert!(err.contains("tipo de modelo inválido"), "{err}");
    }

    #[test]
    fn set_active_model_rejeita_tipo_invalido_antes_de_tocar_config() {
        let err = set_active_model("llm".into(), "whisper-tiny".into()).unwrap_err();
        assert!(err.contains("tipo de modelo inválido"), "{err}");
    }

    #[test]
    fn set_active_model_rejeita_kind_incompativel_com_o_modelo() {
        let err = set_active_model("translation".into(), "whisper-tiny".into()).unwrap_err();
        assert!(err.contains("não pode ser ativo como tradução"), "{err}");
        let err = set_active_model("stt".into(), "towerinstruct-7b-q4_k_m".into()).unwrap_err();
        assert!(err.contains("não pode ser ativo como transcrição"), "{err}");
    }

    #[test]
    fn apply_active_marca_o_campo_correto_da_config() {
        let mut cfg = AppConfig::default();
        let m = find_model("whisper-small-q5").unwrap();
        apply_active(&mut cfg, &m);
        assert_eq!(cfg.active_models.stt, "whisper-small-q5");
        assert_eq!(cfg.active_models.translation, "");

        let m = find_model("nllb-200-distilled-600m-q4").unwrap();
        apply_active(&mut cfg, &m);
        assert_eq!(cfg.active_models.translation, "nllb-200-distilled-600m-q4");
        assert_eq!(cfg.active_models.stt, "whisper-small-q5"); // STT preservado
    }
}
