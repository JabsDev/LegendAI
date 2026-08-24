//! Etapas do pipeline de processamento (tarefa 4.3).
//!
//! Define o enum [`PipelineStep`] que identifica cada etapa do fluxo
//! (extrair → transcrever → traduzir → formatar → exportar) e os payloads dos
//! eventos Tauri emitidos pelo backend para a tela de processamento:
//! - `pipeline-progress` → avanço real de uma etapa ([`PipelineProgress`]);
//! - `pipeline-finished` → término do job (sucesso/erro/cancelado) com resumo
//!   ([`PipelineFinished`] / [`PipelineSummary`]).
//!
//! O frontend (`src/components/pipeline/PipelineView.svelte`) consome os
//! eventos filtrados por `job_id` para montar o stepper e a barra de progresso.

use serde::{Deserialize, Serialize};

/// Etapa do pipeline exibida no stepper da UI. A ordem de enumeração é a ordem
/// de execução; o frontend escolhe quais etapas exibir conforme a origem
/// (legenda embutida pula [`PipelineStep::Transcribe`]) e a opção de tradução.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStep {
    /// Extração do áudio (trilha escolhida) ou da legenda embutida.
    Extract,
    /// Transcrição com o Whisper (origem áudio).
    Transcribe,
    /// Tradução com a engine ativa (NLLB/LLM).
    Translate,
    /// Regras profissionais de formatação (1.8).
    Format,
    /// Gravação do SRT de saída.
    Export,
    /// Processamento concluído.
    Done,
}

/// Payload do evento `pipeline-progress` (emitido a cada avanço real de etapa).
#[derive(Debug, Clone, Serialize)]
pub struct PipelineProgress {
    pub job_id: String,
    pub step: PipelineStep,
    /// Porcentagem da etapa atual (0-100).
    pub pct: u8,
    /// Detalhe textual opcional (ex: `3/10 lotes`).
    pub detail: Option<String>,
}

/// Resumo exibido pela UI ao final do processamento (critério "conclusão mostra
/// resumo: duração, nº segmentos, idiomas").
#[derive(Debug, Clone, Serialize)]
pub struct PipelineSummary {
    /// Caminho do SRT gravado (link para preview/exportação, tarefas 4.4+).
    pub output_path: String,
    /// Duração do vídeo em segundos (0 se o container não expuser).
    pub duration_secs: f64,
    /// Nº de legendas formatadas no SRT final.
    pub segments: usize,
    pub source_lang: String,
    pub target_lang: String,
    /// Segmentos que mantiveram o texto original (fallback por linha da 3.6).
    pub kept_original: usize,
    /// Estatísticas do processamento (5.5): tempo, CPS médio, cobertura de
    /// fala, taxa de tradução e comparativo com a meta do tier.
    pub stats: crate::stats::JobStats,
}

/// Payload do evento `pipeline-finished`: término do job. `ok` = concluído com
/// sucesso; `cancelled` = cancelado pelo usuário (estado limpo, não erro — ver
/// critério "cancelar interrompe etapa atual com estado limpo"); caso contrário
/// `error` traz o [`crate::errors::ErrorDetail`] `{ code, message, hint }` —
/// o frontend (4.8) mapeia `code` para i18n + ação, e usa `message` como
/// fallback. `error` é `None` quando cancelado (a UI usa o flag `cancelled`).
#[derive(Debug, Clone, Serialize)]
pub struct PipelineFinished {
    pub job_id: String,
    pub ok: bool,
    pub cancelled: bool,
    pub error: Option<crate::errors::ErrorDetail>,
    pub summary: Option<PipelineSummary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_serializa_com_nome_estavel_para_ipc() {
        for (step, name) in [
            (PipelineStep::Extract, "extract"),
            (PipelineStep::Transcribe, "transcribe"),
            (PipelineStep::Translate, "translate"),
            (PipelineStep::Format, "format"),
            (PipelineStep::Export, "export"),
            (PipelineStep::Done, "done"),
        ] {
            assert_eq!(
                serde_json::to_value(step).unwrap(),
                serde_json::json!(name),
                "etapa {step:?} deve serializar como `{name}`"
            );
            assert_eq!(
                serde_json::from_value::<PipelineStep>(serde_json::json!(name)).unwrap(),
                step
            );
        }
    }

    #[test]
    fn progress_payload_serializa_para_ipc() {
        let value = serde_json::to_value(PipelineProgress {
            job_id: "abc".into(),
            step: PipelineStep::Translate,
            pct: 42,
            detail: Some("3/10 lotes".into()),
        })
        .unwrap();
        assert_eq!(value["job_id"], "abc");
        assert_eq!(value["step"], "translate");
        assert_eq!(value["pct"], 42);
        assert_eq!(value["detail"], "3/10 lotes");
    }

    #[test]
    fn summary_e_finished_serializam_para_ipc() {
        let summary = PipelineSummary {
            output_path: "/tmp/out.srt".into(),
            duration_secs: 90.5,
            segments: 42,
            source_lang: "pt".into(),
            target_lang: "en".into(),
            kept_original: 1,
            stats: Default::default(),
        };
        let value = serde_json::to_value(PipelineFinished {
            job_id: "j1".into(),
            ok: true,
            cancelled: false,
            error: None,
            summary: Some(summary.clone()),
        })
        .unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["cancelled"], false);
        assert_eq!(value["summary"]["segments"], 42);
        assert_eq!(value["summary"]["source_lang"], "pt");
        // Erro/cancelado sem resumo também serializa (UI trata cada caso).
        let err = crate::errors::ErrorDetail {
            code: "no_speech",
            message: "nenhuma fala detectada no áudio".into(),
            hint: None,
        };
        let value = serde_json::to_value(PipelineFinished {
            job_id: "j2".into(),
            ok: false,
            cancelled: false,
            error: Some(err),
            summary: None,
        })
        .unwrap();
        assert_eq!(value["error"]["code"], "no_speech");
        assert_eq!(value["error"]["message"], "nenhuma fala detectada no áudio");
        assert!(value["summary"].is_null());
        // Cancelado: `error` é `None` (a UI usa o flag `cancelled`).
        let value = serde_json::to_value(PipelineFinished {
            job_id: "j3".into(),
            ok: false,
            cancelled: true,
            error: None,
            summary: None,
        })
        .unwrap();
        assert_eq!(value["cancelled"], true);
        assert!(value["error"].is_null());
    }
}
