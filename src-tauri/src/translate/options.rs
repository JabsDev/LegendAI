//! Opções avançadas de tradução (tarefa 5.4).
//!
//! Formalidade, instruções livres do usuário e nível de contexto. Persistidas
//! na config (0.7, via campo `AppConfig::translation_options`) e injetadas no
//! template de prompt (3.7, `prompt::build_prompt`).
//!
//! Nota do plano: **formalidade só faz sentido em engine LLM** — a engine NLLB
//! (3.2) ignora estas opções (não há prompt). Documentado no campo e nas
//! strings da UI.

use serde::{Deserialize, Serialize};

use super::batcher::DEFAULT_CONTEXT_SIZE;

/// Formalidade da tradução. `ponytail:` sem variante "automático" — o default
/// [`Formality::Colloquial`] preserva exatamente o comportamento atual do
/// prompt 3.7 (basta não trocar a enum); mudar para `Formal` só reforça o tom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Formality {
    /// Registro formal e cuidado.
    Formal,
    /// Registro coloquial e natural (default atual do prompt).
    #[default]
    Colloquial,
}

/// Opções por tradução: entram no template de prompt (3.7) e são persistidas
/// na config. Campos novos precisam de `#[serde(default)]` (via derive) para
/// não quebrar configs antigas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslationOptions {
    /// Tom da tradução. Só afeta engines com prompt (LLM); NLLB ignora.
    pub formality: Formality,
    /// Instruções livres do usuário (ex: "preservar apelidos"). Vazia → o
    /// bloco de instruções é omitido do prompt.
    pub custom_instructions: String,
    /// Quantos segmentos anteriores incluir como contexto no prompt; `0` =
    /// nenhum. `ponytail:` é um teto aplicado em `build_prompt` (`take(n)`) —
    /// o batcher (3.5) já limita o contexto ao default dele; valores acima do
    /// default só entram em vigor se o pipeline repassar o `context_size` ao
    /// batcher (ver nota 5.4 no PLANNING).
    pub context_size: usize,
}

impl Default for TranslationOptions {
    fn default() -> Self {
        Self {
            formality: Formality::Colloquial,
            custom_instructions: String::new(),
            context_size: DEFAULT_CONTEXT_SIZE,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_e_coloquial_sem_instrucoes_e_contexto_padrao() {
        let o = TranslationOptions::default();
        assert_eq!(o.formality, Formality::Colloquial);
        assert_eq!(o.custom_instructions, "");
        assert_eq!(o.context_size, DEFAULT_CONTEXT_SIZE);
    }

    #[test]
    fn round_trip_serde_ipc() {
        let o = TranslationOptions {
            formality: Formality::Formal,
            custom_instructions: "preservar apelidos".into(),
            context_size: 2,
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"formal\""), "{json}");
        let back: TranslationOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(back, o);
    }

    #[test]
    fn campos_ausentes_default_para_padrao() {
        let o: TranslationOptions = serde_json::from_str(r#"{"formality":"colloquial"}"#).unwrap();
        assert_eq!(o.custom_instructions, "");
        assert_eq!(o.context_size, DEFAULT_CONTEXT_SIZE);
    }
}
