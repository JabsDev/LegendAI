//! DTOs do protocolo HTTP `/v1` (Fase 2 — "LegendAI na rede local").
//!
//! Espelha o contrato documentado no plano (`POST /v1/jobs`, `GET /v1/jobs`,
//! `/health`, `/info`, `/models`) e o reusa o [`ErrorDetail`] de código estável
//! para os erros do pipeline. Tempos em ms; `episode` inteiro; idiomas ISO 639-1.

use std::collections::HashMap;

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::errors::ErrorDetail;
use crate::pipeline::queue::{JobOrigin, QueueItem, QueueState};
use crate::pipeline::steps::PipelineStep;

/// Versão do protocolo de rede. O app recusa servidor com `protocol` maior.
pub const PROTOCOL: u32 = 1;

/// Payload de `POST /v1/jobs`. Campos desconhecidos são ignorados; ausentes
/// ganham defaults. `preferred_stt`/`preferred_translation`/`priority` são
/// aceitos no contrato mas ainda não alteram a execução (os modelos ativos da
/// config do PC prevalecem) — ficam registrados no log.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateJobRequest {
    #[serde(default)]
    pub client_job_id: Option<String>,
    #[serde(default)]
    pub anime_key: Option<String>,
    #[serde(default)]
    pub episode: Option<i64>,
    pub source: SourceRequest,
    #[serde(default)]
    pub source_lang: Option<String>,
    #[serde(default)]
    pub target_lang: Option<String>,
    #[serde(default)]
    pub translate: Option<bool>,
    #[serde(default)]
    pub preferred_stt: Option<String>,
    #[serde(default)]
    pub preferred_translation: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
}

/// Fonte de mídia do job remoto. `url` (Fase 2), `upload` (Fase 5 — áudio já
/// extraído pelo app, para DASH/tokens que o PC não abre) e `srt` (Fase 5 —
/// rota S remota: o PC só traduz uma legenda EN/ES já existente).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceRequest {
    Url {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Upload {
        /// Id devolvido por `POST /v1/uploads`.
        upload_id: String,
        /// Demuxer de entrada do ffmpeg (`s16le` = PCM cru 16 kHz mono do app).
        #[serde(default)]
        format: Option<String>,
    },
    #[serde(rename = "srt")]
    Srt {
        srt: String,
        #[serde(default)]
        source_lang: Option<String>,
    },
}

/// Resposta de `POST /v1/uploads` (Fase 5): id do arquivo recebido no PC.
#[derive(Debug, Clone, Serialize)]
pub struct UploadView {
    pub upload_id: String,
    pub bytes: u64,
}

/// Resumo de um item concluído no protocolo (subconjunto do `PipelineSummary`).
#[derive(Debug, Clone, Serialize)]
pub struct JobSummaryView {
    pub duration_secs: f64,
    pub segments: usize,
    pub source_lang: String,
    pub target_lang: String,
    /// Tamanho do SRT pronto em bytes (0 se o arquivo sumiu).
    pub srt_bytes: u64,
    /// ETA estimado (não calculado no MVP — `null`).
    pub eta_secs: Option<u64>,
}

/// Item da fila como visto pelo app remoto.
#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    pub job_id: String,
    pub client_job_id: Option<String>,
    pub anime_key: Option<String>,
    pub episode: Option<i64>,
    pub state: QueueState,
    pub step: Option<PipelineStep>,
    pub pct: u8,
    pub detail: Option<String>,
    pub summary: Option<JobSummaryView>,
    pub error: Option<ErrorDetail>,
    pub origin: JobOrigin,
    pub created_ms: u64,
    pub updated_ms: u64,
}

impl JobView {
    /// Projeta um [`QueueItem`] da fila no DTO do protocolo, lendo o tamanho do
    /// SRT do disco quando o job terminou.
    pub fn from_item(item: &QueueItem) -> Self {
        let summary = item.summary.as_ref().map(|s| JobSummaryView {
            duration_secs: s.duration_secs,
            segments: s.segments,
            source_lang: s.source_lang.clone(),
            target_lang: s.target_lang.clone(),
            srt_bytes: std::fs::metadata(&s.output_path)
                .map(|m| m.len())
                .unwrap_or(0),
            eta_secs: None,
        });
        Self {
            job_id: item.id.clone(),
            client_job_id: item.client_job_id.clone(),
            anime_key: item.anime_key.clone(),
            episode: item.episode,
            state: item.state,
            step: item.step,
            pct: item.pct,
            detail: item.detail.clone(),
            summary,
            error: item.error.clone(),
            origin: item.origin,
            created_ms: item.created_ms,
            updated_ms: item.updated_ms,
        }
    }
}

/// Modelos ativos retornados por `/health` e `/models`.
#[derive(Debug, Clone, Serialize)]
pub struct ModelsActiveView {
    pub stt: String,
    pub translation: String,
}

/// `GET /v1/health`.
#[derive(Debug, Clone, Serialize)]
pub struct HealthView {
    pub app: &'static str,
    pub version: String,
    pub protocol: u32,
    pub name: String,
    pub tier: String,
    pub gpu: bool,
    /// Jobs em execução.
    pub busy: usize,
    /// Jobs aguardando na fila.
    pub queue: usize,
    pub models: ModelsActiveView,
}

/// `GET /v1/info` — dados de pareamento (QR).
///
/// `tailscale_host`/`tailscale_url` aparecem quando há uma interface Tailscale
/// ativa (`100.64.0.0/10`). É a rota para parear quando o roteador isola as
/// redes Ethernet/Wi-Fi: o app aceita o IP `100.x` e fala com o PC pela VPN.
#[derive(Debug, Clone, Serialize)]
pub struct InfoView {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub protocol: u32,
    pub version: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailscale_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailscale_url: Option<String>,
}

/// Entrada do catálogo em `GET /v1/models` (diagnóstico).
#[derive(Debug, Clone, Serialize)]
pub struct ModelView {
    pub id: String,
    pub kind: String,
    pub backend: String,
    pub size_mb: u64,
    pub min_ram_gb: u32,
    pub quality: u8,
    pub speed: u8,
    pub downloaded: bool,
}

/// `GET /v1/models`.
#[derive(Debug, Clone, Serialize)]
pub struct ModelsView {
    pub active: ModelsActiveView,
    pub catalog: Vec<ModelView>,
}

/// Erro de API com código estável + HTTP coerente. `status` não cruza o JSON.
#[derive(Debug, Clone, Serialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(skip)]
    pub status: StatusCode,
}

impl ApiError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, status: StatusCode) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            hint: None,
            status,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message, StatusCode::BAD_REQUEST)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message, StatusCode::NOT_FOUND)
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, message, StatusCode::CONFLICT)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message, StatusCode::INTERNAL_SERVER_ERROR)
    }

    /// Erro de `enqueue_internal`: no fluxo remoto só falha por condição
    /// inesperada (URL remota não valida caminho local) → 500.
    pub fn from_queue_enqueue(message: String) -> Self {
        Self::internal(message)
    }

    /// `queue_cancel`: job não está rodando → 409.
    pub fn from_queue_cancel(message: String) -> Self {
        if message.contains("nenhum processamento") {
            Self::conflict("not_running", message)
        } else {
            Self::internal(message)
        }
    }

    /// `queue_remove`: item inexistente → 404; em execução → 409.
    pub fn from_queue_remove(message: String) -> Self {
        if message.contains("não está na fila") {
            Self::not_found(message)
        } else if message.contains("cancele antes") {
            Self::conflict("running", message)
        } else {
            Self::internal(message)
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self)).into_response()
    }
}
