//! Mapeamento de hardware → tier (tarefa 2.6).
//!
//! Tier é determinístico (fórmula), não magia de ML (nota da tarefa): limites
//! de RAM + presença de GPU. Consumido pela recomendação (2.6), pelo
//! onboarding (6.4) e pela UI de modelos via comando IPC (2.8).
//!
//! Limiares (README/tarefa): Tier 1 = <6GB RAM ou CPU-only fraco; Tier 2 =
//! ~8GB; Tier 3 = 16GB+ ou GPU. `max_model_ram_gb` deixa folga para o SO+app
//! além do `min_ram_gb` declarado pelo modelo. Ajustar as constantes conforme
//! feedback real — a fórmula em si não muda.

use serde::{Deserialize, Serialize};

use super::detect::HardwareInfo;

/// RAM máxima (inclusive) para a máquina ser Tier 1 — abaixo de 6GB.
pub const TIER_1_MAX_RAM_GB: u32 = 5;
/// RAM mínima para a máquina ser Tier 2 (~8GB).
pub const TIER_2_MIN_RAM_GB: u32 = 6;
/// RAM mínima para a máquina ser Tier 3 (16GB+ ou GPU presente).
pub const TIER_3_MIN_RAM_GB: u32 = 16;

/// Tier de hardware do usuário, derivado de [`HardwareInfo`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Tier1,
    Tier2,
    Tier3,
}

impl Tier {
    /// Teto de `min_ram_gb` de um modelo compatível com o tier.
    ///
    /// Deixa folga para o SO + app além do que o modelo declara precisar:
    /// Tier 1 (≈4GB) ≈ 2GB livres; Tier 2 (≈8GB) ≈ 5GB; Tier 3 (16GB+) ≈ 16GB.
    pub fn max_model_ram_gb(self) -> u32 {
        match self {
            Tier::Tier1 => 2,
            Tier::Tier2 => 5,
            Tier::Tier3 => 16,
        }
    }
}

/// Mapeia o hardware para um tier: GPU presente ou RAM >= 16GB → Tier 3;
/// RAM >= 6GB → Tier 2; senão (RAM < 6GB, CPU-only fraco) → Tier 1.
pub fn tier_for(hw: &HardwareInfo) -> Tier {
    if hw.gpu.is_some() || hw.ram_gb >= TIER_3_MIN_RAM_GB {
        Tier::Tier3
    } else if hw.ram_gb >= TIER_2_MIN_RAM_GB {
        Tier::Tier2
    } else {
        Tier::Tier1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hardware::detect::GpuKind;

    fn hw(ram_gb: u32, gpu: Option<GpuKind>) -> HardwareInfo {
        HardwareInfo {
            ram_gb,
            cpu_threads: 8,
            gpu,
            cpu_name: "test".into(),
            recommended_threads: 4,
        }
    }

    #[test]
    fn tier_for_segue_limiares_de_ram_sem_gpu() {
        assert_eq!(tier_for(&hw(2, None)), Tier::Tier1);
        assert_eq!(tier_for(&hw(4, None)), Tier::Tier1);
        assert_eq!(tier_for(&hw(5, None)), Tier::Tier1);
        assert_eq!(tier_for(&hw(6, None)), Tier::Tier2);
        assert_eq!(tier_for(&hw(8, None)), Tier::Tier2);
        assert_eq!(tier_for(&hw(15, None)), Tier::Tier2);
        assert_eq!(tier_for(&hw(16, None)), Tier::Tier3);
        assert_eq!(tier_for(&hw(32, None)), Tier::Tier3);
    }

    #[test]
    fn gpu_presente_eleva_para_tier3() {
        assert_eq!(tier_for(&hw(2, Some(GpuKind::Cuda))), Tier::Tier3);
        assert_eq!(tier_for(&hw(4, Some(GpuKind::Rocm))), Tier::Tier3);
        assert_eq!(tier_for(&hw(8, Some(GpuKind::Metal))), Tier::Tier3);
    }

    #[test]
    fn max_model_ram_gb_cresce_com_o_tier() {
        assert_eq!(Tier::Tier1.max_model_ram_gb(), 2);
        assert_eq!(Tier::Tier2.max_model_ram_gb(), 5);
        assert_eq!(Tier::Tier3.max_model_ram_gb(), 16);
    }

    #[test]
    fn tier_round_trip_serde() {
        for t in [Tier::Tier1, Tier::Tier2, Tier::Tier3] {
            let back: Tier = serde_json::from_value(serde_json::to_value(t).unwrap()).unwrap();
            assert_eq!(t, back);
        }
    }
}
