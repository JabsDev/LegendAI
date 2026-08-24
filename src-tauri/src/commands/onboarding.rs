//! Comando de onboarding / primeira execução (tarefa 6.4).
//!
//! Expoõe à UI os dados de "primeiro boot": se a config ainda não existe
//! (`first_run`), o hardware detectado (2.5), o tier derivado (2.6) e os
//! modelos recomendados por tipo (2.6) — a tela de boas-vindas baixa direto os
//! recomendados via `download_model` (2.9).

use serde::Serialize;

use crate::config::AppConfig;
use crate::hardware::detect::{detect, HardwareInfo};
use crate::hardware::tier::{tier_for, Tier};
use crate::model_manager::catalog::ModelInfo;
use crate::model_manager::recommend::recommend;

/// Modelos recomendados por tipo para a tela de boas-vindas.
#[derive(Debug, Clone, Serialize)]
pub struct RecommendedModels {
    pub stt: Vec<ModelInfo>,
    pub translation: Vec<ModelInfo>,
}

/// Dados completos do onboarding (serializados para a UI).
#[derive(Debug, Clone, Serialize)]
pub struct OnboardingInfo {
    /// `true` quando a config ainda não existe (primeira execução).
    pub first_run: bool,
    pub hardware: HardwareInfo,
    pub tier: Tier,
    pub recommendations: RecommendedModels,
}

/// Devolve o estado de primeira execução, o hardware e as recomendações por
/// tier. Não faz rede e não bloqueia o boot (<1s, ver 2.5).
#[tauri::command(rename_all = "snake_case")]
pub fn get_onboarding() -> Result<OnboardingInfo, String> {
    // Config ausente → primeira execução. Se o diretório não for resolvível
    // (raro), não bloqueia o app com onboarding.
    let first_run = match AppConfig::config_path() {
        Ok(p) => !p.exists(),
        Err(_) => false,
    };
    let hw = detect();
    let tier = tier_for(&hw);
    Ok(OnboardingInfo {
        first_run,
        hardware: hw,
        tier,
        recommendations: RecommendedModels {
            stt: recommend(tier, crate::model_manager::catalog::ModelKind::Stt),
            translation: recommend(tier, crate::model_manager::catalog::ModelKind::Translation),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onboarding_recomenda_stt_e_traducao() {
        // Com a config real do ambiente não garantimos `first_run`; o que
        // interessa é que o comando sempre devolva recomendações válidas.
        let info = get_onboarding().unwrap();
        assert!(!info.recommendations.stt.is_empty());
        assert!(!info.recommendations.translation.is_empty());
        // Recomendações respeitam o teto de RAM do tier.
        for m in info
            .recommendations
            .stt
            .iter()
            .chain(info.recommendations.translation.iter())
        {
            assert!(m.min_ram_gb <= info.tier.max_model_ram_gb());
        }
    }
}
