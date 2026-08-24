//! Recomendação de modelos por tier (tarefa 2.6).
//!
//! Para cada `kind` (STT/tradução), [`recommend`] retorna os modelos do
//! catálogo compatíveis com o tier — `min_ram_gb` dentro do teto do tier
//! ([`Tier::max_model_ram_gb`]) — ordenados do mais alto `quality` primeiro
//! (desempate por `speed`). A UI consome via comando IPC (2.8/6.4).
//!
//! O `backend` não filtra separadamente: todos os modelos do catálogo rodam
//! em CPU (qualquer máquina tem) e a curadoria por tier já codifica a escolha
//! de backend (NLLB/ort no Tier 1, Qwen/llama no Tier 2/3, ADR-001). A
//! disponibilidade real do backend (ex: build sem feature GPU) é tratada pela
//! factory de engines (3.4), não pela recomendação.

use crate::hardware::tier::Tier;
use crate::model_manager::catalog::{Catalog, ModelInfo, ModelKind};

/// Recomenda modelos do catálogo embutido para o `kind`, filtrados por
/// compatibilidade de RAM com o tier e ordenados por `quality` (desc) e
/// depois `speed` (desc). O primeiro item é o modelo recomendado por padrão.
pub fn recommend(tier: Tier, kind: ModelKind) -> Vec<ModelInfo> {
    let mut models: Vec<ModelInfo> = Catalog::embedded()
        .expect("catálogo embutido deve validar (invariante de build-time)")
        .models
        .into_iter()
        .filter(|m| m.kind == kind && m.min_ram_gb <= tier.max_model_ram_gb())
        .collect();
    models.sort_by(|a, b| b.quality.cmp(&a.quality).then(b.speed.cmp(&a.speed)));
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recomendacao_respeita_min_ram_gb_por_tier() {
        for (tier, max) in [
            (Tier::Tier1, Tier::Tier1.max_model_ram_gb()),
            (Tier::Tier2, Tier::Tier2.max_model_ram_gb()),
            (Tier::Tier3, Tier::Tier3.max_model_ram_gb()),
        ] {
            for kind in [ModelKind::Stt, ModelKind::Translation] {
                let rec = recommend(tier, kind);
                assert!(!rec.is_empty(), "{tier:?}/{kind:?} deve recomendar algo");
                assert!(
                    rec.iter().all(|m| m.min_ram_gb <= max),
                    "{tier:?}/{kind:?} respeitou min_ram_gb <= {max}"
                );
            }
        }
    }

    #[test]
    fn recomendacao_so_retorna_kind_do_pedido() {
        let stt = recommend(Tier::Tier3, ModelKind::Stt);
        let tr = recommend(Tier::Tier3, ModelKind::Translation);
        assert!(stt.iter().all(|m| m.kind == ModelKind::Stt));
        assert!(tr.iter().all(|m| m.kind == ModelKind::Translation));
    }

    #[test]
    fn recomendacao_ordenada_por_qualidade_depois_velocidade() {
        for tier in [Tier::Tier1, Tier::Tier2, Tier::Tier3] {
            for kind in [ModelKind::Stt, ModelKind::Translation] {
                let rec = recommend(tier, kind);
                for w in rec.windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    assert!(
                        a.quality > b.quality || (a.quality == b.quality && a.speed >= b.speed),
                        "{tier:?}/{kind:?}: `{}` (q{} v{}) antes de `{}` (q{} v{})",
                        a.id,
                        a.quality,
                        a.speed,
                        b.id,
                        b.quality,
                        b.speed
                    );
                }
            }
        }
    }

    #[test]
    fn tier1_exclui_modelos_pesados() {
        let stt_ids: Vec<String> = recommend(Tier::Tier1, ModelKind::Stt)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert!(stt_ids.contains(&"whisper-tiny".to_string()));
        assert!(stt_ids.contains(&"whisper-small-q5".to_string()));
        assert!(!stt_ids.contains(&"whisper-medium-q5".to_string()));
        assert!(!stt_ids.contains(&"whisper-large-v3-q5".to_string()));

        let tr_ids: Vec<String> = recommend(Tier::Tier1, ModelKind::Translation)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert!(tr_ids.contains(&"nllb-200-distilled-600m-q4".to_string()));
        assert!(!tr_ids.contains(&"nllb-200-distilled-600m".to_string()));
        assert!(!tr_ids.contains(&"towerinstruct-7b-q4_k_m".to_string()));
    }

    #[test]
    fn tier3_recomenda_todos_os_modelos_do_catalogo() {
        let cat = Catalog::embedded().unwrap();
        for kind in [ModelKind::Stt, ModelKind::Translation] {
            let expected = cat.models.iter().filter(|m| m.kind == kind).count();
            let rec = recommend(Tier::Tier3, kind);
            assert_eq!(rec.len(), expected, "{kind:?}: tier 3 inclui todos");
            assert!(rec.iter().all(|m| m.kind == kind));
        }
    }

    #[test]
    fn recomendacao_padrao_por_tier_bate_com_readme() {
        let top = |tier, kind| recommend(tier, kind).first().unwrap().clone();
        // Com Hy-MT2 1.8B (quality 5 speed 5) no catálogo, ele vira topo para Tier1/2/3 (supera NLLB/Tower em quality)
        let t1 = top(Tier::Tier1, ModelKind::Translation);
        assert!(
            t1.quality == 5 && t1.min_ram_gb <= 2,
            "Tier1 top deve ser Hy-MT2 1.8B q4 (q5 speed5), veio {:?}",
            t1.id
        );
        let t2 = top(Tier::Tier2, ModelKind::Translation);
        assert!(
            t2.quality == 5 && t2.min_ram_gb <= 5,
            "Tier2 top deve ser quality 5 (Hy-MT2), veio {:?}",
            t2.id
        );
        let t3 = top(Tier::Tier3, ModelKind::Translation);
        assert!(
            t3.quality == 5,
            "Tier3 top deve ser quality 5, veio {:?}",
            t3.id
        );
        // STT: Tier1 → small-q5; Tier2/3 → melhor quality 5 que cabe (Parakeet 0.6B speed 5 vence turbo)
        assert_eq!(top(Tier::Tier1, ModelKind::Stt).id, "whisper-small-q5");
        let t2stt = top(Tier::Tier2, ModelKind::Stt);
        assert_eq!(
            t2stt.quality, 5,
            "Tier2 STT top deve ser quality 5, veio {:?}",
            t2stt.id
        );
        assert!(
            t2stt.min_ram_gb <= 5,
            "Tier2 STT top deve caber em 5GB, veio {:?}",
            t2stt.id
        );
    }
}
