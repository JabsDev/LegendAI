//! Medição de RSS e pico de memória por etapa (tarefa 3.8).
//!
//! No Tier 1 (4GB) o requisito é que o modelo STT e a engine de tradução NUNCA
//! fiquem carregados juntos. Este módulo dá as ferramentas para medir o RSS do
//! processo atual (em bytes) e registrar o pico a cada etapa do pipeline, além
//! de avisar quando o uso ultrapassar um limite configurado (ex: 3.2GB no Tier
//! 1) com a sugestão de usar um modelo menor.
//!
//! O RSS é medido via `sysinfo` (já uma dependência do projeto, usada na 2.5):
//! internamente ele lê `/proc/self/status` no Linux e `GetProcessMemoryInfo` no
//! Windows — exatamente as fontes citadas na tarefa — mas de forma cross-platform
//! e sem depender de crate por-OS.

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

/// Limite de RSS do Tier 1 (3.2GB) acima do qual o pipeline avisa o usuário
/// com sugestão de modelo menor. Outros tiers usam valores maiores (3.8).
pub const TIER1_RSS_LIMIT_BYTES: u64 = 3_200_000_000;

/// Mede o RSS (resident set size) do processo atual, em bytes. Retorna 0 se a
/// medição falhar (ex: plataforma sem suporte) — o tracker degrada para "sem
/// limite efetivo", sem crash.
pub fn rss_bytes() -> u64 {
    let mut sys = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing().with_memory()),
    );
    let pid = Pid::from_u32(std::process::id());
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), false);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}

/// Snapshot de RSS ao fim de uma etapa do pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryStage {
    pub stage: String,
    pub rss_bytes: u64,
}

/// Rastreador de pico de memória por etapa. Cada [`MemoryTracker::mark`] mede o
/// RSS atual, loga a etapa e o pico acumulado, e sinaliza se o limite foi
/// estourado. [`MemoryTracker::warn_if_over`] emite o aviso final (sugestão de
/// modelo menor) — usado pelo pipeline ao término.
pub struct MemoryTracker {
    limit_bytes: u64,
    peak_bytes: u64,
    peak_stage: String,
    stages: Vec<MemoryStage>,
}

impl MemoryTracker {
    /// Cria um tracker com `limit_bytes` como teto de aviso.
    pub fn new(limit_bytes: u64) -> Self {
        Self {
            limit_bytes,
            peak_bytes: 0,
            peak_stage: String::new(),
            stages: Vec::new(),
        }
    }

    /// Marca o fim de `stage`: mede o RSS, loga a etapa com o pico acumulado e
    /// atualiza o pico. Retorna o RSS medido nesta etapa.
    pub fn mark(&mut self, stage: &str) -> u64 {
        let rss = rss_bytes();
        let mb = rss as f64 / (1024.0 * 1024.0);
        if rss > self.peak_bytes {
            self.peak_bytes = rss;
            self.peak_stage = stage.to_string();
        }
        self.stages.push(MemoryStage {
            stage: stage.to_string(),
            rss_bytes: rss,
        });
        tracing::info!(
            "memória [{stage}]: RSS {mb:.1} MiB (pico {:.1} MiB)",
            self.peak_bytes as f64 / (1024.0 * 1024.0)
        );
        rss
    }

    /// Pico de RSS observado até agora, em bytes.
    pub fn peak_bytes(&self) -> u64 {
        self.peak_bytes
    }

    /// Etapa em que o pico foi observado.
    pub fn peak_stage(&self) -> &str {
        &self.peak_stage
    }

    /// `true` se o pico ultrapassou o limite configurado.
    pub fn over_limit(&self) -> bool {
        self.limit_bytes != 0 && self.peak_bytes > self.limit_bytes
    }

    /// Loga aviso com sugestão de modelo menor se o pico passou do limite.
    pub fn warn_if_over(&self) {
        if self.over_limit() {
            let lim_mb = self.limit_bytes as f64 / (1024.0 * 1024.0);
            let peak_mb = self.peak_bytes as f64 / (1024.0 * 1024.0);
            tracing::warn!(
                "pico de memória de {peak_mb:.0} MiB em `{}` ultrapassou o limite de {lim_mb:.0} \
                 MiB — considere usar um modelo menor (ex: whisper-tiny + nllb-q4)",
                self.peak_stage
            );
        }
    }

    /// Snapshot imutável das etapas para relatório.
    pub fn stages(&self) -> &[MemoryStage] {
        &self.stages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss_bytes_retorna_valor_plausivel_na_mesa_de_dev() {
        // Critério de aceitação 1/3: medição funciona (pico < limite numa etapa).
        // Não exige modelo — só o processo atual. >0 no Linux/Windows/macOS.
        let rss = rss_bytes();
        assert!(
            rss > 0,
            "RSS do processo deve ser > 0 em plataforma suportada (veio {rss})"
        );
        assert!(
            rss < TIER1_RSS_LIMIT_BYTES,
            "processo já acima do teto do Tier 1"
        );
    }

    #[test]
    fn mark_loga_e_acumula_pico_por_etapa() {
        let mut t = MemoryTracker::new(TIER1_RSS_LIMIT_BYTES);
        let a = t.mark("stt");
        let b = t.mark("tradução");
        assert!(a > 0 && b > 0);
        // O pico é o maior dos dois (RSS real do processo; ambos plausíveis).
        assert_eq!(t.peak_bytes(), a.max(b));
        assert!(
            t.peak_stage() == "stt" || t.peak_stage() == "tradução",
            "pico deve apontar para uma etapa real, veio {}",
            t.peak_stage()
        );
        assert_eq!(t.stages().len(), 2);
        assert!(!t.over_limit(), "processo de teste não deve estourar 3.2GB");
    }

    #[test]
    fn limite_zero_significa_sem_aviso() {
        let mut t = MemoryTracker::new(0);
        t.mark("x");
        assert!(!t.over_limit(), "limite 0 = sem teto efetivo (não avisa)");
    }

    #[test]
    fn over_limit_detecta_limite_menor_que_o_pico_real() {
        let mut t = MemoryTracker::new(1); // 1 byte < RSS real
        t.mark("x");
        assert!(t.over_limit(), "limite de 1 byte deve ser estourado");
    }
}
