//! Detecção de hardware em runtime (tarefa 2.5).
//!
//! Coleta RAM total, threads de CPU, nome do processador e presença de GPU
//! (CUDA/ROCm/Metal) sem nenhuma dependência de rede. Consumido pela 2.6
//! (tier) e pelo onboarding (6.4).
//!
//! Abordagem pragmática para GPU (nota da tarefa): **não** inicializa backend
//! GPU (seria lento e exigiria build com feature GPU) — detecta a presença dos
//! binários de driver no `PATH` (`nvidia-smi` → CUDA, `rocm-smi`/`rocminfo` →
//! ROCm). Metal é sempre reportado em macOS (todo hardware Apple suporta).
//! Backend CPU é priorizado no Tier 1 por design (ADR-005); a GPU detectada
//! aqui é informativa e usada só para o tier/recomendação (2.6).

use serde::{Deserialize, Serialize};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Backend de GPU detectado no sistema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuKind {
    /// NVIDIA CUDA (driver `nvidia-smi` presente).
    Cuda,
    /// AMD ROCm (driver `rocminfo`/`rocm-smi` presente).
    Rocm,
    /// Apple Metal (todo hardware macOS).
    Metal,
}

/// Resumo do hardware da máquina, coletado em runtime sem rede.
///
/// `recommended_threads` é a heurística inicial (passo 3 da tarefa):
/// `min(cpu_threads, ram_gb / 2)`, no mínimo 1 — o Tier 1 (4GB) prefere
/// threads reduzidas a estourar RAM; ajustável em 2.6 conforme feedback real.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareInfo {
    /// RAM total em GiB (inteiro truncado; 0 se indisponível).
    pub ram_gb: u32,
    /// Threads de CPU disponíveis (0 se indisponível).
    pub cpu_threads: u32,
    /// GPU detectada (`None` → CPU-only).
    pub gpu: Option<GpuKind>,
    /// Nome/modelo do processador (ex: `Intel(R) Core(TM) i7-1165G7`).
    pub cpu_name: String,
    /// Heurística de threads recomendadas para o backend.
    pub recommended_threads: u32,
}

/// Heurística inicial de threads: `min(cpu_threads, ram_gb / 2)`, no mínimo 1.
/// RAM desconhecida (0) cai no piso de 1 thread — seguro e não bloqueia boot.
pub fn recommended_threads(ram_gb: u32, cpu_threads: u32) -> u32 {
    cpu_threads.min(ram_gb.saturating_div(2)).max(1)
}

/// Coleta as informações de hardware. Sem rede e sem panics: qualquer falha
/// individual degrada para um default seguro (RAM/CPU 0, GPU `None`).
/// Rápido o suficiente para o boot (<1s): só RAM + lista de CPUs.
pub fn detect() -> HardwareInfo {
    let sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_memory(MemoryRefreshKind::nothing().with_ram())
            .with_cpu(CpuRefreshKind::nothing()),
    );
    let ram_gb = (sys.total_memory() >> 30) as u32;
    let cpu_threads = sys.cpus().len() as u32;
    let cpu_name = sys
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_string())
        .unwrap_or_default();
    let gpu = detect_gpu();
    HardwareInfo {
        ram_gb,
        cpu_threads,
        gpu,
        cpu_name,
        recommended_threads: recommended_threads(ram_gb, cpu_threads),
    }
}

/// Detecta GPU pela presença dos binários de driver no `PATH`.
/// Em macOS, Metal é sempre reportado (todo hardware Apple suporta).
pub fn detect_gpu() -> Option<GpuKind> {
    #[cfg(target_os = "macos")]
    {
        return Some(GpuKind::Metal);
    }
    #[cfg(not(target_os = "macos"))]
    {
        if binary_in_path("nvidia-smi") {
            return Some(GpuKind::Cuda);
        }
        if binary_in_path("rocm-smi") || binary_in_path("rocminfo") {
            return Some(GpuKind::Rocm);
        }
        None
    }
}

/// Busca um executável nas entradas do `PATH` sem spawn — só `stat` em cada
/// diretório (rápido e não executa o binário do driver).
#[cfg(not(target_os = "macos"))]
fn binary_in_path(name: &str) -> bool {
    binary_in_path_in(std::env::var_os("PATH").as_deref(), name)
}

#[cfg(not(target_os = "macos"))]
fn binary_in_path_in(path: Option<&std::ffi::OsStr>, name: &str) -> bool {
    let exe = if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    path.is_some_and(|path| std::env::split_paths(path).any(|dir| dir.join(&exe).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("legendai-hw-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn detect_retorna_valores_plausiveis() {
        let hw = detect();
        assert!(hw.ram_gb >= 1, "RAM plausível, veio {}", hw.ram_gb);
        assert!(
            hw.cpu_threads >= 1,
            "CPU plausível, veio {}",
            hw.cpu_threads
        );
        assert_eq!(
            hw.recommended_threads,
            recommended_threads(hw.ram_gb, hw.cpu_threads)
        );
        assert!(
            (1..=hw.cpu_threads).contains(&hw.recommended_threads),
            "threads recomendadas dentro de 1..=cpu_threads"
        );
        // Critério de aceitação: detecção leva <1s (não é o gargalo do boot).
        let start = std::time::Instant::now();
        let _ = detect();
        assert!(
            start.elapsed().as_millis() < 1000,
            "detect() levou {:?} (limite 1s)",
            start.elapsed()
        );
    }

    #[test]
    fn gpu_detect_nao_crasha_com_e_sem_gpu() {
        // Deve sempre retornar (Some ou None), nunca panic — fallback CPU.
        let gpu = detect_gpu();
        if let Some(kind) = gpu {
            assert!(matches!(
                kind,
                GpuKind::Cuda | GpuKind::Rocm | GpuKind::Metal
            ));
        }
        // Estrutura consumida pela 2.6 nunca depende de GPU presente.
        let hw = HardwareInfo {
            ram_gb: 8,
            cpu_threads: 4,
            gpu: None,
            cpu_name: "test".into(),
            recommended_threads: recommended_threads(8, 4),
        };
        assert_eq!(hw.gpu, None);
    }

    #[test]
    fn recommended_threads_respeita_min_de_1_e_a_heuristica() {
        // Heurística: min(cpu_threads, ram_gb/2).
        assert_eq!(recommended_threads(8, 16), 4); // cap por RAM (8/2)
        assert_eq!(recommended_threads(32, 16), 16); // cap por CPU
        assert_eq!(recommended_threads(4, 8), 2); // tier 1: RAM pequena limita
                                                  // Piso de 1 (nunca 0 threads), mesmo com RAM desconhecida (0).
        assert_eq!(recommended_threads(2, 8), 1);
        assert_eq!(recommended_threads(1, 8), 1);
        assert_eq!(recommended_threads(0, 8), 1);
        assert_eq!(recommended_threads(0, 0), 1);
    }

    #[test]
    fn hardware_info_round_trip_serde() {
        let hw = HardwareInfo {
            ram_gb: 16,
            cpu_threads: 8,
            gpu: Some(GpuKind::Cuda),
            cpu_name: "AMD Ryzen".into(),
            recommended_threads: 8,
        };
        let back: HardwareInfo =
            serde_json::from_value(serde_json::to_value(&hw).unwrap()).unwrap();
        assert_eq!(hw, back);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn binary_in_path_detecta_presenca_e_ausencia() {
        let dir = temp_dir("bin");
        let name = if cfg!(target_os = "windows") {
            "nvidia-smi.exe"
        } else {
            "nvidia-smi"
        };
        std::fs::write(dir.join(name), b"#!/bin/sh\n").unwrap();
        assert!(binary_in_path_in(Some(dir.as_os_str()), "nvidia-smi"));
        assert!(!binary_in_path_in(Some(dir.as_os_str()), "rocm-smi"));
        assert!(!binary_in_path_in(None, "nvidia-smi"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
