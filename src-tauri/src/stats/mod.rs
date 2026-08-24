//! Estatísticas de processamento (tarefa 5.5).
//!
//! Coleta, ao fim de cada job do pipeline (4.3/4.9), as métricas exibidas no
//! painel `src/components/stats/StatsPanel.svelte`: tempo total de
//! processamento, duração do vídeo, nº de segmentos, CPS médio, cobertura de
//! fala, taxa de tradução (segundos de legenda por segundo de processamento) e
//! o comparativo com a meta do tier.
//!
//! A meta por tier é a única fonte de verdade vinda da 2.6 (nota da tarefa):
//! [`tier_goal_realtime`] mapeia [`Tier`] para o fator *realtime* alvo
//! (duração do vídeo / tempo de processamento) — "1h → 30/10/3min" para
//! Tier 1/2/3 — e [`compute_stats`] deriva a meta em segundos da duração real
//! do vídeo. O [`JobStats`] é serializado dentro do
//! [`crate::pipeline::steps::PipelineSummary`] (IPC) para a UI exibir.

use serde::Serialize;

use crate::format::FormattedSubtitle;
use crate::hardware::tier::Tier;

/// Fator *realtime* alvo do Tier 1: 1h de vídeo processada em ≤ 30min.
pub const TIER1_GOAL_REALTIME: f64 = 2.0;
/// Fator *realtime* alvo do Tier 2: 1h → ≤ 10min.
pub const TIER2_GOAL_REALTIME: f64 = 6.0;
/// Fator *realtime* alvo do Tier 3: 1h → ≤ 3min.
pub const TIER3_GOAL_REALTIME: f64 = 20.0;

/// Fator *realtime* alvo do tier (segundos de legenda por segundo de
/// processamento). A meta de um vídeo em segundos = `duração / fator`
/// (ex: 1h → Tier 1 = 1800s, Tier 2 = 600s, Tier 3 = 180s).
pub fn tier_goal_realtime(tier: Tier) -> f64 {
    match tier {
        Tier::Tier1 => TIER1_GOAL_REALTIME,
        Tier::Tier2 => TIER2_GOAL_REALTIME,
        Tier::Tier3 => TIER3_GOAL_REALTIME,
    }
}

/// Métricas de um job preenchidas ao término (serializadas no
/// [`crate::pipeline::steps::PipelineSummary`] para a UI).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct JobStats {
    /// Tempo total de processamento, em segundos.
    pub processing_secs: f64,
    /// Duração do vídeo em segundos (0 se o container não expuser).
    pub duration_secs: f64,
    /// Nº de legendas formatadas no SRT final.
    pub segments: usize,
    /// Velocidade de leitura média (CPS), ponderada pelo tempo de tela:
    /// total de caracteres ÷ tempo total de fala.
    pub avg_cps: f64,
    /// Percentual de cobertura de fala (tempo de fala / duração do vídeo, 0-100).
    pub speech_coverage_pct: f64,
    /// Taxa de tradução: segundos de legenda por segundo de processamento
    /// (fator *realtime* real — ex: `6.0` = 1h de vídeo em 10min).
    pub translation_ratio: f64,
    /// Tier de hardware (2.6) que definiu a meta.
    pub tier: Tier,
    /// Meta de processamento do tier para esta duração, em segundos.
    pub goal_processing_secs: f64,
}

impl Default for JobStats {
    /// Estatísticas "vazias" (zeros + Tier 1) — usadas em testes e como valor
    /// inicial antes de um job ser concluído.
    fn default() -> Self {
        Self {
            processing_secs: 0.0,
            duration_secs: 0.0,
            segments: 0,
            avg_cps: 0.0,
            speech_coverage_pct: 0.0,
            translation_ratio: 0.0,
            tier: Tier::Tier1,
            goal_processing_secs: 0.0,
        }
    }
}

/// Calcula as estatísticas do job a partir das legendas formatadas finais (1.8).
///
/// `processing_secs` é o tempo decorrido do job (medido pelo chamador — o
/// pipeline 4.3/4.9). `formatted` é o vetor pós-formatação que alimenta o SRT
/// final — CPS e cobertura medem o que o espectador realmente lê. O tempo de
/// fala é a soma das durações das legendas (overlap zero por construção do
/// formatter 1.8, então a soma mede o intervalo coberto).
pub fn compute_stats(
    processing_secs: f64,
    duration_secs: f64,
    formatted: &[FormattedSubtitle],
    tier: Tier,
) -> JobStats {
    let (total_chars, total_speech_ms) = formatted.iter().fold((0usize, 0u64), |(c, d), f| {
        (c + f.text().chars().count(), d + f.duration_ms())
    });
    let speech_secs = total_speech_ms as f64 / 1000.0;
    let avg_cps = if total_speech_ms > 0 {
        total_chars as f64 / speech_secs
    } else {
        0.0
    };
    // Legendas podem ter o `end` estendido além do vídeo por regras 1.8 — a
    // cobertura é capada em 100% (nunca "mais fala que o vídeo").
    let speech_coverage_pct = if duration_secs > 0.0 {
        (speech_secs / duration_secs * 100.0).min(100.0)
    } else {
        0.0
    };
    let translation_ratio = if processing_secs > 0.0 {
        duration_secs / processing_secs
    } else {
        0.0
    };
    let goal_processing_secs = duration_secs / tier_goal_realtime(tier);

    JobStats {
        processing_secs,
        duration_secs,
        segments: formatted.len(),
        avg_cps,
        speech_coverage_pct,
        translation_ratio,
        tier,
        goal_processing_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Language, Timestamp};

    fn f(lines: Vec<&str>, start_ms: u64, end_ms: u64) -> FormattedSubtitle {
        FormattedSubtitle {
            index: 1,
            lines: lines.into_iter().map(String::from).collect(),
            start_ms: Timestamp::from_ms(start_ms),
            end_ms: Timestamp::from_ms(end_ms),
            language: Language::Pt,
        }
    }

    #[test]
    fn metas_por_tier_seguem_1h_30_10_3min() {
        let hour = 3600.0;
        assert_eq!(tier_goal_realtime(Tier::Tier1), 2.0);
        assert_eq!(tier_goal_realtime(Tier::Tier2), 6.0);
        assert_eq!(tier_goal_realtime(Tier::Tier3), 20.0);
        for (tier, goal_secs) in [
            (Tier::Tier1, 1800.0), // 1h em 30min
            (Tier::Tier2, 600.0),  // 1h em 10min
            (Tier::Tier3, 180.0),  // 1h em 3min
        ] {
            let stats = compute_stats(0.0, hour, &[], tier);
            assert!(
                (stats.goal_processing_secs - goal_secs).abs() < 1e-9,
                "tier {tier:?}: meta {} != {goal_secs}s",
                stats.goal_processing_secs
            );
        }
    }

    #[test]
    fn cps_medio_e_ponderado_pelo_tempo_de_tela() {
        // 5 chars / 1s + 5 chars / 10s → 10 chars / 11s ≈ 0.9091 cps.
        let formatted = vec![f(vec!["abcde"], 0, 1000), f(vec!["abcde"], 2000, 12000)];
        let stats = compute_stats(100.0, 30.0, &formatted, Tier::Tier2);
        assert!(
            (stats.avg_cps - 10.0 / 11.0).abs() < 1e-6,
            "avg_cps = {}",
            stats.avg_cps
        );
    }

    #[test]
    fn cobertura_de_fala_e_quociente_da_duracao() {
        // 11s de fala em 22s de vídeo → 50%.
        let formatted = vec![f(vec!["abcde"], 0, 1000), f(vec!["abcde"], 2000, 12000)];
        let stats = compute_stats(60.0, 22.0, &formatted, Tier::Tier1);
        assert!((stats.speech_coverage_pct - 50.0).abs() < 1e-6);
        // Legendas estendidas além do vídeo nunca passam de 100%.
        let over = compute_stats(60.0, 1.0, &formatted, Tier::Tier1);
        assert!(over.speech_coverage_pct <= 100.0);
    }

    #[test]
    fn taxa_de_traducao_mede_segundos_de_legenda_por_segundo_de_processamento() {
        // 3600s de vídeo em 600s de processamento → 6× (1h em 10min, Tier 2).
        let stats = compute_stats(600.0, 3600.0, &[], Tier::Tier2);
        assert!((stats.translation_ratio - 6.0).abs() < 1e-9);
        // Sem tempo de processamento → 0 (nunca divide por zero/NaN).
        let zero = compute_stats(0.0, 3600.0, &[], Tier::Tier2);
        assert_eq!(zero.translation_ratio, 0.0);
        assert!(zero.avg_cps.is_finite());
    }

    #[test]
    fn entrada_vazia_produz_zeros_sem_panic() {
        let stats = compute_stats(0.0, 0.0, &[], Tier::Tier1);
        assert_eq!(stats.segments, 0);
        assert_eq!(stats.avg_cps, 0.0);
        assert_eq!(stats.speech_coverage_pct, 0.0);
        assert_eq!(stats.translation_ratio, 0.0);
        assert_eq!(stats.goal_processing_secs, 0.0);
    }

    #[test]
    fn stats_serializa_para_ipc_com_tier_snake_case() {
        let stats = compute_stats(10.0, 60.0, &[], Tier::Tier3);
        let value = serde_json::to_value(stats).unwrap();
        assert_eq!(value["tier"], "tier3");
        assert_eq!(value["segments"], 0);
        assert_eq!(value["processing_secs"], 10.0);
        assert_eq!(value["goal_processing_secs"], 3.0);
    }
}
