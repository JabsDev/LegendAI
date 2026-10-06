//! Rotas axum do servidor HTTP embutido `/v1` (Fase 2).
//!
//! O estado da API é abstraído por [`QueueApi`] para permitir testes de rota
//! sem um `AppHandle` real: em produção [`AppQueue`] delega para a fila
//! (`pipeline::queue`); nos testes um `FakeQueue` em memória responde.
//!
//! Erros sempre JSON `{ code, message, hint }` com HTTP coerente
//! (400/404/409/500) e `202` para job aceito mas ainda sem SRT.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path as UrlPath, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};

use crate::commands::pipeline::{PipelineOptions, PipelineSource};
use crate::config::AppConfig;
use crate::errors::ErrorDetail;
use crate::hardware::detect::detect;
use crate::hardware::tier::{tier_for, Tier};
use crate::model_manager::catalog::{Backend, Catalog, ModelKind};
use crate::pipeline::queue::{self, JobOrigin, QueueItem, QueueState};

use super::dto::*;

/// Teto do corpo de `POST /v1/uploads` (Fase 5): PCM cru 16 kHz mono de um
/// episódio de ~50 min fica em ~96 MB; 512 MiB dá folga sem permitir encher o
/// disco do PC (LAN sem auth — ver §13 do plano).
pub const MAX_UPLOAD_BYTES: usize = 512 * 1024 * 1024;

/// Ids sequenciais dos uploads (arquivo-safe).
static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Diretório dos áudios enviados pelo app (`config_dir/legendai/uploads/`).
pub fn uploads_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .map(|d| d.join("legendai").join("uploads"))
        .unwrap_or_else(|| std::env::temp_dir().join("legendai-uploads"))
}

/// Gera o id de um upload (sem separadores de caminho).
fn next_upload_id() -> String {
    let n = UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("upload-{ms}-{n}")
}

/// Resolve o caminho de um upload a partir do id, rejeitando ids com
/// separadores (`../`) — o id vem do cliente, nunca confiar no formato.
fn upload_path(upload_id: &str) -> Result<std::path::PathBuf, ApiError> {
    let id = upload_id.trim();
    if id.is_empty() || id.contains(['/', '\\', '.']) {
        return Err(ApiError::bad_request("upload_id inválido"));
    }
    Ok(uploads_dir().join(id))
}

/// Remove uploads órfãos com mais de 24 h (best-effort no boot). Um job que
/// ficou na fila por mais tempo que isso perde o áudio — o app reenvia.
pub fn sweep_uploads() {
    let dir = uploads_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 3600);
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() && meta.modified().map(|m| m < cutoff).unwrap_or(false) {
            if let Err(e) = std::fs::remove_file(entry.path()) {
                tracing::debug!("upload órfão não removido: {e}");
            }
        }
    }
}

/// Identidade do servidor exibida no pareamento (`/info`) e no `/health`.
#[derive(Debug, Clone)]
pub struct ServerIdentity {
    pub name: String,
    pub host: String,
    pub port: u16,
    /// IP do Tailscale (`100.64.0.0/10`), quando ativo — segunda rota de
    /// pareamento para redes com isolamento Ethernet/Wi-Fi.
    pub tailscale_host: Option<String>,
}

impl ServerIdentity {
    fn url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    fn tailscale_url(&self) -> Option<String> {
        self.tailscale_host
            .as_ref()
            .map(|h| format!("http://{h}:{}", self.port))
    }
}

/// Pedido de enfileiramento já normalizado (vindo do protocolo remoto).
pub struct EnqueueRequest {
    /// Origem já resolvida (URL, upload localizado ou SRT inline).
    pub source: PipelineSource,
    pub options: PipelineOptions,
    pub client_job_id: Option<String>,
    pub anime_key: Option<String>,
    pub episode: Option<i64>,
}

impl EnqueueRequest {
    /// Rótulo do item da fila: URL, caminho do upload ou chave do SRT. Não é
    /// usado pela execução (o `source` manda), mas identifica o item na UI/log.
    fn input_path(&self) -> String {
        match &self.source {
            PipelineSource::Url { url, .. } => url.clone(),
            PipelineSource::Upload { path, .. } => path.clone(),
            PipelineSource::InlineSrt { .. } => {
                format!("inline-srt:{}", self.client_job_id.as_deref().unwrap_or(""))
            }
            _ => String::new(),
        }
    }
}

/// Operações de fila que as rotas consomem. Abstraído para teste.
pub trait QueueApi: Send + Sync + 'static {
    fn list(&self) -> Vec<QueueItem>;
    fn get(&self, id: &str) -> Option<QueueItem>;
    fn enqueue(&self, req: EnqueueRequest) -> Result<QueueItem, ApiError>;
    fn cancel(&self, id: &str) -> Result<(), ApiError>;
    fn remove(&self, id: &str) -> Result<(), ApiError>;
}

/// Implementação de produção: fila real + `AppHandle` para emitir eventos.
struct AppQueue {
    app: tauri::AppHandle,
}

impl QueueApi for AppQueue {
    fn list(&self) -> Vec<QueueItem> {
        queue::queue_list()
    }

    fn get(&self, id: &str) -> Option<QueueItem> {
        queue::queue_get(id)
    }

    fn enqueue(&self, req: EnqueueRequest) -> Result<QueueItem, ApiError> {
        let input_path = req.input_path();
        queue::enqueue_internal(
            &self.app,
            input_path,
            req.source,
            Some(req.options),
            JobOrigin::Remote,
            req.client_job_id,
            req.anime_key,
            req.episode,
        )
        .map_err(ApiError::from_queue_enqueue)
    }

    fn cancel(&self, id: &str) -> Result<(), ApiError> {
        queue::queue_cancel(self.app.clone(), id.to_string()).map_err(ApiError::from_queue_cancel)
    }

    fn remove(&self, id: &str) -> Result<(), ApiError> {
        queue::queue_remove(self.app.clone(), id.to_string()).map_err(ApiError::from_queue_remove)
    }
}

/// Estado compartilhado das rotas.
#[derive(Clone)]
pub struct ApiState {
    identity: ServerIdentity,
    queue: Arc<dyn QueueApi>,
}

impl ApiState {
    /// Estado de produção (app Tauri + fila real).
    pub fn new(app: tauri::AppHandle, identity: ServerIdentity) -> Self {
        Self {
            identity,
            queue: Arc::new(AppQueue { app }),
        }
    }

    /// Estado com uma fila injetada (testes).
    #[cfg(test)]
    fn with_queue(identity: ServerIdentity, queue: Arc<dyn QueueApi>) -> Self {
        Self { identity, queue }
    }
}

/// Monta o `Router` com todas as rotas `/v1`.
pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/info", get(info))
        .route("/v1/models", get(models))
        .route("/v1/jobs", get(list_jobs).post(create_job))
        .route("/v1/jobs/{id}", get(get_job).delete(delete_job))
        .route("/v1/jobs/{id}/srt", get(get_srt))
        .route("/v1/jobs/{id}/cancel", post(cancel_job))
        // Fase 5 — fallback de upload de áudio: corpo binário cru (o app manda
        // PCM 16 kHz mono). O limite de corpo é elevado só nesta rota.
        .route(
            "/v1/uploads",
            post(upload_audio).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .with_state(state)
}

/// `Tier` → rótulo do protocolo (`"Tier2"`).
fn tier_label(tier: Tier) -> String {
    match tier {
        Tier::Tier1 => "Tier1",
        Tier::Tier2 => "Tier2",
        Tier::Tier3 => "Tier3",
    }
    .to_string()
}

fn kind_str(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Stt => "stt",
        ModelKind::Translation => "translation",
    }
}

fn backend_str(backend: Backend) -> &'static str {
    match backend {
        Backend::Whisper => "whisper",
        Backend::Llama => "llama",
        Backend::Ort => "ort",
        Backend::Parakeet => "parakeet",
        Backend::Canary => "canary",
        Backend::Nemotron => "nemotron",
    }
}

async fn health(State(state): State<ApiState>) -> Json<HealthView> {
    let hw = detect();
    let items = state.queue.list();
    let busy = items
        .iter()
        .filter(|i| i.state == QueueState::Running)
        .count();
    let pending = items
        .iter()
        .filter(|i| i.state == QueueState::Pending)
        .count();
    let cfg = AppConfig::load_or_default();
    Json(HealthView {
        app: "legendai",
        version: env!("CARGO_PKG_VERSION").into(),
        protocol: PROTOCOL,
        name: state.identity.name.clone(),
        tier: tier_label(tier_for(&hw)),
        gpu: hw.gpu.is_some(),
        busy,
        queue: pending,
        models: ModelsActiveView {
            stt: cfg.active_models.stt,
            translation: cfg.active_models.translation,
        },
    })
}

async fn info(State(state): State<ApiState>) -> Json<InfoView> {
    Json(InfoView {
        name: state.identity.name.clone(),
        host: state.identity.host.clone(),
        port: state.identity.port,
        protocol: PROTOCOL,
        version: env!("CARGO_PKG_VERSION").into(),
        url: state.identity.url(),
        tailscale_host: state.identity.tailscale_host.clone(),
        tailscale_url: state.identity.tailscale_url(),
    })
}

async fn models(State(_state): State<ApiState>) -> Result<Json<ModelsView>, ApiError> {
    let cat = Catalog::embedded().map_err(|e| ApiError::internal(e.to_string()))?;
    let cfg = AppConfig::load_or_default();
    let catalog = cat
        .models
        .iter()
        .map(|m| ModelView {
            id: m.id.clone(),
            kind: kind_str(m.kind).into(),
            backend: backend_str(m.backend).into(),
            size_mb: m.size_mb,
            min_ram_gb: m.min_ram_gb,
            quality: m.quality,
            speed: m.speed,
            downloaded: crate::model_manager::cache::resolve_model_path(m).is_ok(),
        })
        .collect();
    Ok(Json(ModelsView {
        active: ModelsActiveView {
            stt: cfg.active_models.stt,
            translation: cfg.active_models.translation,
        },
        catalog,
    }))
}

async fn list_jobs(
    State(state): State<ApiState>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<Vec<JobView>> {
    let since = params.get("since").and_then(|s| s.parse::<u64>().ok());
    let mut items = state.queue.list();
    if let Some(since) = since {
        items.retain(|i| i.updated_ms > since);
    }
    Json(items.iter().map(JobView::from_item).collect())
}

async fn get_job(State(state): State<ApiState>, UrlPath(id): UrlPath<String>) -> Response {
    match state.queue.get(&id) {
        Some(item) => Json(JobView::from_item(&item)).into_response(),
        None => ApiError::not_found(format!("job `{id}` não encontrado")).into_response(),
    }
}

async fn create_job(
    State(state): State<ApiState>,
    payload: Result<Json<CreateJobRequest>, JsonRejection>,
) -> Response {
    let Json(req) = match payload {
        Ok(json) => json,
        Err(rejection) => {
            return ApiError::bad_request(format!(
                "corpo JSON inválido: {}",
                rejection.body_text()
            ))
            .with_hint("Envie `{ source: { type: \"url\", url: \"...\", headers: {...} }, ... }`")
            .into_response();
        }
    };
    // Campos aceitos no contrato mas ainda sem efeito no MVP: registra para
    // diagnóstico (os modelos ativos da config do PC prevalecem).
    if req.preferred_stt.is_some() || req.preferred_translation.is_some() {
        tracing::info!(
            "job remoto pediu preferências (stt={:?}, mt={:?}) — usando os modelos ativos",
            req.preferred_stt,
            req.preferred_translation
        );
    }
    if let Some(p) = req.priority {
        tracing::debug!("prioridade informada pelo cliente: {p}");
    }

    // Resolve a origem (Fase 2/5): URL, upload já gravado no PC ou SRT inline.
    let source = match req.source {
        SourceRequest::Url { url, headers } => {
            if url.trim().is_empty() {
                return ApiError::bad_request("a URL da fonte está vazia").into_response();
            }
            PipelineSource::Url { url, headers }
        }
        SourceRequest::Upload { upload_id, format } => {
            let path = match upload_path(&upload_id) {
                Ok(p) => p,
                Err(e) => return e.into_response(),
            };
            if !path.exists() {
                return ApiError::not_found("upload não encontrado (expirou?)")
                    .with_hint("Envie o áudio do aparelho novamente.")
                    .into_response();
            }
            PipelineSource::Upload {
                path: path.to_string_lossy().into_owned(),
                format,
            }
        }
        SourceRequest::Srt { srt, source_lang } => {
            if srt.trim().is_empty() {
                return ApiError::bad_request("a legenda enviada está vazia").into_response();
            }
            PipelineSource::InlineSrt { srt, source_lang }
        }
    };
    let options = PipelineOptions {
        translate: req.translate.unwrap_or(true),
        out_path: None,
        target_lang: req.target_lang,
        source_lang: req.source_lang,
    };
    let request = EnqueueRequest {
        source,
        options,
        client_job_id: req.client_job_id,
        anime_key: req.anime_key,
        episode: req.episode,
    };
    match state.queue.enqueue(request) {
        Ok(item) => (StatusCode::ACCEPTED, Json(JobView::from_item(&item))).into_response(),
        Err(e) => e.into_response(),
    }
}

/// `POST /v1/uploads` (Fase 5): recebe o áudio extraído pelo app (corpo binário
/// cru) e grava em `uploads/<id>`. Devolve `201 { upload_id, bytes }`, usado no
/// `POST /v1/jobs` com `source.type = "upload"`.
async fn upload_audio(body: Bytes) -> Response {
    if body.is_empty() {
        return ApiError::bad_request("o upload está vazio").into_response();
    }
    let dir = uploads_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return ApiError::internal(format!("não foi possível preparar os uploads: {e}"))
            .into_response();
    }
    let upload_id = next_upload_id();
    let path = dir.join(&upload_id);
    if let Err(e) = std::fs::write(&path, &body) {
        return ApiError::internal(format!("não foi possível gravar o upload: {e}"))
            .into_response();
    }
    let bytes = body.len() as u64;
    tracing::info!("upload de áudio recebido: {upload_id} ({bytes} bytes)");
    (StatusCode::CREATED, Json(UploadView { upload_id, bytes })).into_response()
}

async fn cancel_job(State(state): State<ApiState>, UrlPath(id): UrlPath<String>) -> Response {
    match state.queue.cancel(&id) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn delete_job(State(state): State<ApiState>, UrlPath(id): UrlPath<String>) -> Response {
    match state.queue.remove(&id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => e.into_response(),
    }
}

async fn get_srt(State(state): State<ApiState>, UrlPath(id): UrlPath<String>) -> Response {
    let Some(item) = state.queue.get(&id) else {
        return ApiError::not_found(format!("job `{id}` não encontrado")).into_response();
    };
    match item.state {
        QueueState::Done => {
            let Some(summary) = item.summary.as_ref() else {
                return ApiError::internal("job concluído sem SRT associado").into_response();
            };
            match std::fs::read(&summary.output_path) {
                Ok(bytes) => (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    bytes,
                )
                    .into_response(),
                Err(e) => ApiError::internal(format!("SRT indisponível: {e}")).into_response(),
            }
        }
        QueueState::Error => {
            let error = item.error.clone().unwrap_or(ErrorDetail {
                code: "pipeline_failed",
                message: "o job falhou".into(),
                hint: None,
            });
            (StatusCode::CONFLICT, Json(error)).into_response()
        }
        QueueState::Cancelled => {
            ApiError::conflict("job_cancelled", "o job foi cancelado").into_response()
        }
        // pending/running: ainda não há SRT — o app deve continuar o polling.
        _ => (StatusCode::ACCEPTED, Json(JobView::from_item(&item))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::pipeline::steps::PipelineSummary;

    #[derive(Default)]
    struct FakeQueue {
        items: Mutex<Vec<QueueItem>>,
    }

    impl QueueApi for FakeQueue {
        fn list(&self) -> Vec<QueueItem> {
            self.items.lock().unwrap().clone()
        }

        fn get(&self, id: &str) -> Option<QueueItem> {
            self.items
                .lock()
                .unwrap()
                .iter()
                .find(|i| i.id == id)
                .cloned()
        }

        fn enqueue(&self, req: EnqueueRequest) -> Result<QueueItem, ApiError> {
            let input_path = req.input_path();
            let mut item = QueueItem::new(
                format!("job-fake-{}", self.items.lock().unwrap().len() + 1),
                input_path,
                req.source,
                req.options,
            );
            item.origin = JobOrigin::Remote;
            item.client_job_id = req.client_job_id;
            item.anime_key = req.anime_key;
            item.episode = req.episode;
            self.items.lock().unwrap().push(item.clone());
            Ok(item)
        }

        fn cancel(&self, id: &str) -> Result<(), ApiError> {
            let mut items = self.items.lock().unwrap();
            match items.iter_mut().find(|i| i.id == id) {
                Some(i) if i.state == QueueState::Running => Ok(()),
                Some(_) => Err(ApiError::conflict(
                    "not_running",
                    "nenhum processamento em andamento",
                )),
                None => Err(ApiError::conflict(
                    "not_running",
                    "nenhum processamento em andamento",
                )),
            }
        }

        fn remove(&self, id: &str) -> Result<(), ApiError> {
            let mut items = self.items.lock().unwrap();
            match items.iter().position(|i| i.id == id) {
                Some(idx) => {
                    items.remove(idx);
                    Ok(())
                }
                None => Err(ApiError::not_found(format!("item `{id}` não está na fila"))),
            }
        }
    }

    fn identity() -> ServerIdentity {
        ServerIdentity {
            name: "PC-Jabs".into(),
            host: "192.168.2.109".into(),
            port: 8765,
            tailscale_host: Some("100.125.210.81".into()),
        }
    }

    fn app(items: Vec<QueueItem>) -> Router {
        router(ApiState::with_queue(
            identity(),
            Arc::new(FakeQueue {
                items: Mutex::new(items),
            }),
        ))
    }

    fn item(id: &str, state: QueueState) -> QueueItem {
        let mut it = QueueItem::new(
            id.into(),
            "https://cdn/a.m3u8".into(),
            PipelineSource::Url {
                url: "https://cdn/a.m3u8".into(),
                headers: HashMap::new(),
            },
            PipelineOptions::default(),
        );
        it.state = state;
        it.origin = JobOrigin::Remote;
        it
    }

    /// Executa uma requisição no router e devolve (status, body, content-type).
    async fn send(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, Vec<u8>, Option<String>) {
        let mut builder = Request::builder().method(method).uri(uri);
        let request = match body {
            Some(value) => {
                builder = builder.header(header::CONTENT_TYPE, "application/json");
                builder.body(Body::from(value.to_string())).unwrap()
            }
            None => builder.body(Body::empty()).unwrap(),
        };
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let ctype = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        (status, bytes, ctype)
    }

    fn json(body: &[u8]) -> serde_json::Value {
        serde_json::from_slice(body).unwrap()
    }

    #[tokio::test]
    async fn health_responde_200_com_protocolo_e_nome() {
        let (status, body, _) = send(app(vec![]), "GET", "/v1/health", None).await;
        assert_eq!(status, StatusCode::OK);
        let v = json(&body);
        assert_eq!(v["app"], "legendai");
        assert_eq!(v["protocol"], PROTOCOL);
        assert_eq!(v["name"], "PC-Jabs");
        assert!(v["tier"].as_str().unwrap().starts_with("Tier"));
        assert_eq!(v["queue"], 0);
    }

    #[tokio::test]
    async fn info_responde_url_de_pareamento() {
        let (status, body, _) = send(app(vec![]), "GET", "/v1/info", None).await;
        assert_eq!(status, StatusCode::OK);
        let v = json(&body);
        assert_eq!(v["url"], "http://192.168.2.109:8765");
        assert_eq!(v["protocol"], PROTOCOL);
        // Rota Tailscale exposta para parear com isolamento de rede.
        assert_eq!(v["tailscale_host"], "100.125.210.81");
        assert_eq!(v["tailscale_url"], "http://100.125.210.81:8765");
    }

    #[tokio::test]
    async fn create_job_responde_202_e_item_pendente() {
        let payload = serde_json::json!({
            "client_job_id": "goanime:Bocchi:1:ja",
            "anime_key": "Bocchi",
            "episode": 1,
            "source": {
                "type": "url",
                "url": "https://cdn/ep1.m3u8",
                "headers": { "Referer": "https://animegg.org/" }
            },
            "translate": true,
            "target_lang": "pt"
        });
        let (status, body, _) = send(app(vec![]), "POST", "/v1/jobs", Some(payload)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let v = json(&body);
        assert_eq!(v["state"], "pending");
        assert_eq!(v["origin"], "remote");
        assert_eq!(v["client_job_id"], "goanime:Bocchi:1:ja");
        assert_eq!(v["episode"], 1);
    }

    #[tokio::test]
    async fn create_job_rejeita_json_invalido_com_400() {
        let mut builder = Request::builder().method("POST").uri("/v1/jobs");
        builder = builder.header(header::CONTENT_TYPE, "application/json");
        let request = builder.body(Body::from("{ não é json")).unwrap();
        let response = app(vec![]).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn upload_audio_grava_e_devolve_id() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/uploads")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(vec![0u8, 1, 2, 3, 4]))
            .unwrap();
        let response = app(vec![]).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let v = json(&bytes);
        let id = v["upload_id"].as_str().unwrap().to_string();
        assert_eq!(v["bytes"], 5);
        let path = uploads_dir().join(&id);
        assert!(path.exists(), "arquivo do upload deve existir");
        // Limpeza: não poluir o diretório de config real.
        std::fs::remove_file(&path).ok();
    }

    #[tokio::test]
    async fn upload_vazio_responde_400() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/uploads")
            .body(Body::empty())
            .unwrap();
        let response = app(vec![]).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_job_upload_e_srt_aceitam_as_novas_origens() {
        // Upload: grava o áudio e enfileira apontando para o id.
        let request = Request::builder()
            .method("POST")
            .uri("/v1/uploads")
            .body(Body::from(vec![1u8, 2, 3]))
            .unwrap();
        let response = app(vec![]).oneshot(request).await.unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let id = json(&bytes)["upload_id"].as_str().unwrap().to_string();

        let payload = serde_json::json!({
            "client_job_id": "goanime:X:1:upload",
            "source": { "type": "upload", "upload_id": id, "format": "s16le" },
            "translate": true,
            "target_lang": "pt"
        });
        let (status, body, _) = send(app(vec![]), "POST", "/v1/jobs", Some(payload)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(json(&body)["state"], "pending");
        std::fs::remove_file(uploads_dir().join(&id)).ok();

        // SRT inline: rota S remota — só traduz.
        let payload = serde_json::json!({
            "client_job_id": "goanime:X:1:srt-en",
            "source": {
                "type": "srt",
                "srt": "1\n00:00:01,000 --> 00:00:02,000\nHello\n",
                "source_lang": "en"
            },
            "target_lang": "pt"
        });
        let (status, body, _) = send(app(vec![]), "POST", "/v1/jobs", Some(payload)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(json(&body)["client_job_id"], "goanime:X:1:srt-en");

        // SRT vazio é rejeitado.
        let payload = serde_json::json!({
            "source": { "type": "srt", "srt": "   " }
        });
        let (status, _, _) = send(app(vec![]), "POST", "/v1/jobs", Some(payload)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Upload inexistente → 404.
        let payload = serde_json::json!({
            "source": { "type": "upload", "upload_id": "upload-inexistente" }
        });
        let (status, _, _) = send(app(vec![]), "POST", "/v1/jobs", Some(payload)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn upload_path_rejeita_traversal() {
        assert!(upload_path("upload-1-2").is_ok());
        assert!(upload_path("../../etc/passwd").is_err());
        assert!(upload_path("a/b").is_err());
        assert!(upload_path("").is_err());
    }

    #[tokio::test]
    async fn list_jobs_respeita_since() {
        let mut old = item("j-old", QueueState::Done);
        old.updated_ms = 100;
        let mut new = item("j-new", QueueState::Pending);
        new.updated_ms = 500;
        let (status, body, _) = send(app(vec![old, new]), "GET", "/v1/jobs?since=200", None).await;
        assert_eq!(status, StatusCode::OK);
        let v = json(&body);
        assert_eq!(v.as_array().unwrap().len(), 1);
        assert_eq!(v[0]["job_id"], "j-new");
    }

    #[tokio::test]
    async fn get_job_desconhecido_responde_404() {
        let (status, body, _) = send(app(vec![]), "GET", "/v1/jobs/nao-existe", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json(&body)["code"], "not_found");
    }

    #[tokio::test]
    async fn srt_de_job_pendente_responde_202() {
        let (status, _, _) = send(
            app(vec![item("j1", QueueState::Pending)]),
            "GET",
            "/v1/jobs/j1/srt",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn srt_de_job_concluido_responde_text_plain() {
        let dir = std::env::temp_dir().join(format!("legendai-net-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let srt = dir.join("j1.srt");
        std::fs::write(&srt, "1\n00:00:01,000 --> 00:00:02,000\nOlá\n").unwrap();

        let mut done = item("j1", QueueState::Done);
        done.summary = Some(PipelineSummary {
            output_path: srt.to_string_lossy().into_owned(),
            duration_secs: 120.0,
            segments: 1,
            source_lang: "ja".into(),
            target_lang: "pt".into(),
            kept_original: 0,
            stats: Default::default(),
        });
        let (status, body, ctype) = send(app(vec![done]), "GET", "/v1/jobs/j1/srt", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(ctype.unwrap().starts_with("text/plain"));
        assert!(String::from_utf8_lossy(&body).contains("Olá"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn srt_de_job_com_erro_responde_409_com_error() {
        let mut failed = item("j1", QueueState::Error);
        failed.error = Some(ErrorDetail {
            code: "no_speech",
            message: "nenhuma fala detectada".into(),
            hint: None,
        });
        let (status, body, _) = send(app(vec![failed]), "GET", "/v1/jobs/j1/srt", None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json(&body)["code"], "no_speech");
    }

    #[tokio::test]
    async fn cancel_sem_execucao_responde_409() {
        let (status, _, _) = send(
            app(vec![item("j1", QueueState::Pending)]),
            "POST",
            "/v1/jobs/j1/cancel",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn delete_remove_item_e_responde_204() {
        let state_items = vec![item("j1", QueueState::Done)];
        let router = app(state_items);
        let (status, _, _) = send(router.clone(), "DELETE", "/v1/jobs/j1", None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        // Removido: get subsequente é 404.
        let (status, _, _) = send(router, "GET", "/v1/jobs/j1", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn models_responde_catalogo_e_ativos() {
        let (status, body, _) = send(app(vec![]), "GET", "/v1/models", None).await;
        assert_eq!(status, StatusCode::OK);
        let v = json(&body);
        assert!(v["catalog"].as_array().unwrap().len() > 10);
        assert!(v["active"].is_object());
    }

    /// Sobe o servidor de verdade num socket efêmero e conversa por HTTP real
    /// (o equivalente aos `curl` do entregável da Fase 2).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn serve_real_cria_job_lista_e_consulta_srt() {
        let state = ApiState::with_queue(identity(), Arc::new(FakeQueue::default()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router(state)).await.unwrap();
        });

        let client = reqwest::Client::new();
        let base = format!("http://{addr}");

        let health: serde_json::Value = client
            .get(format!("{base}/v1/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["protocol"], PROTOCOL);
        assert_eq!(health["name"], "PC-Jabs");

        let created = client
            .post(format!("{base}/v1/jobs"))
            .json(&serde_json::json!({
                "client_job_id": "goanime:Bocchi:1:ja",
                "anime_key": "Bocchi",
                "episode": 1,
                "source": { "type": "url", "url": "https://cdn/ep1.m3u8" },
                "target_lang": "pt"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::ACCEPTED);
        let created: serde_json::Value = created.json().await.unwrap();
        let job_id = created["job_id"].as_str().unwrap().to_string();

        let list: Vec<serde_json::Value> = client
            .get(format!("{base}/v1/jobs"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["job_id"], job_id);
        assert_eq!(list[0]["origin"], "remote");

        // Ainda não há SRT → 202 (o app continua o polling).
        let srt = client
            .get(format!("{base}/v1/jobs/{job_id}/srt"))
            .send()
            .await
            .unwrap();
        assert_eq!(srt.status(), StatusCode::ACCEPTED);
    }
}
