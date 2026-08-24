use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::Language;

/// Erros do pipeline de tradução. Engines concretas (3.2/3.3) podem adicionar
/// variantes tipadas; a integração com `LegendaiError` (código estável p/ UI)
/// entra quando o pipeline expõe a tradução (3.10).
#[derive(Debug, Error)]
pub enum TranslateError {
    #[error("falha no backend de tradução: {0}")]
    Backend(String),
}

/// Status de uma linha traduzida — base do fallback por linha (3.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranslationStatus {
    /// Traduzida com sucesso.
    Ok,
    /// Falhou — re-tentar no próximo lote (até N tentativas).
    Retry,
    /// Falhou persistentemente — manter o texto original.
    KeptOriginal,
}

/// Um segmento a traduzir, com contexto anterior para coerência (3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchSegment {
    pub id: u32,
    pub text: String,
    /// Segmentos anteriores (originais e já traduzidos) — nunca futuros.
    pub context: Vec<String>,
}

/// Opções de geração por lote. `None` = default do engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BatchOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// Requisição de tradução de um lote de segmentos (IPC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchRequest {
    pub source_lang: Language,
    pub target_lang: Language,
    pub segments: Vec<BatchSegment>,
    #[serde(default)]
    pub options: BatchOptions,
}

/// Uma linha traduzida com seu status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslatedSegment {
    pub id: u32,
    pub text: String,
    pub status: TranslationStatus,
}

/// Resultado de um lote — ordem reconstruída via `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchResult {
    pub translations: Vec<TranslatedSegment>,
}

/// Contrato plugável da engine de tradução (núcleo do ADR-001).
pub trait TranslationEngine {
    fn translate_batch(&mut self, req: &BatchRequest) -> Result<BatchResult, TranslateError>;
    fn supported_pair(&self, source: &Language, target: &Language) -> bool;
}

/// Engine mock para testes/contrato: traduz prefixando o texto.
#[derive(Debug, Clone)]
pub struct MockEngine {
    pub prefix: String,
}

impl Default for MockEngine {
    fn default() -> Self {
        Self {
            prefix: "TR".into(),
        }
    }
}

impl TranslationEngine for MockEngine {
    fn translate_batch(&mut self, req: &BatchRequest) -> Result<BatchResult, TranslateError> {
        let translations = req
            .segments
            .iter()
            .map(|s| TranslatedSegment {
                id: s.id,
                text: format!("{} {}", self.prefix, s.text),
                status: TranslationStatus::Ok,
            })
            .collect();
        Ok(BatchResult { translations })
    }

    fn supported_pair(&self, _source: &Language, _target: &Language) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_segments() -> Vec<BatchSegment> {
        vec![
            BatchSegment {
                id: 1,
                text: "Olá mundo".into(),
                context: vec![],
            },
            BatchSegment {
                id: 2,
                text: "Como vai?".into(),
                context: vec!["Olá mundo".into()],
            },
            BatchSegment {
                id: 3,
                text: "Tudo bem.".into(),
                context: vec!["Olá mundo".into(), "Como vai?".into()],
            },
        ]
    }

    fn sample_request() -> BatchRequest {
        BatchRequest {
            source_lang: Language::Pt,
            target_lang: Language::En,
            segments: sample_segments(),
            options: BatchOptions::default(),
        }
    }

    #[test]
    fn mock_engine_traduz_lote_preservando_ids_e_ordem() {
        let mut engine = MockEngine::default();
        let result = engine.translate_batch(&sample_request()).unwrap();

        assert_eq!(result.translations.len(), 3);
        let ids: Vec<u32> = result.translations.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![1, 2, 3]); // ordem preservada
        assert!(result
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::Ok));
        assert_eq!(result.translations[0].text, "TR Olá mundo");
    }

    #[test]
    fn mock_engine_suporta_todos_os_pares() {
        let engine = MockEngine::default();
        assert!(engine.supported_pair(&Language::Pt, &Language::En));
        assert!(engine.supported_pair(&Language::En, &Language::Ja));
    }

    #[test]
    fn batch_request_round_trip_serde() {
        let req = sample_request();
        let json = serde_json::to_string(&req).unwrap();
        let back: BatchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source_lang, Language::Pt);
        assert_eq!(back.target_lang, Language::En);
        assert_eq!(back.segments, req.segments);
    }

    #[test]
    fn result_serializa_status_ok_retry_kept_original() {
        let result = BatchResult {
            translations: vec![
                TranslatedSegment {
                    id: 1,
                    text: "x".into(),
                    status: TranslationStatus::Ok,
                },
                TranslatedSegment {
                    id: 2,
                    text: "y".into(),
                    status: TranslationStatus::Retry,
                },
                TranslatedSegment {
                    id: 3,
                    text: "z".into(),
                    status: TranslationStatus::KeptOriginal,
                },
            ],
        };
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({
                "translations": [
                    { "id": 1, "text": "x", "status": "ok" },
                    { "id": 2, "text": "y", "status": "retry" },
                    { "id": 3, "text": "z", "status": "kept_original" }
                ]
            })
        );
    }

    #[test]
    fn request_options_ausentes_default_para_vazio() {
        let req: BatchRequest =
            serde_json::from_str(r#"{"source_lang":"pt","target_lang":"en","segments":[]}"#)
                .unwrap();
        assert_eq!(req.options, BatchOptions::default());
        assert!(req.segments.is_empty());
    }
}
