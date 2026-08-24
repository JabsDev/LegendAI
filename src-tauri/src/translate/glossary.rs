//! Glossário persistente do usuário (tarefa 5.6).
//!
//! Termos fixos (termo no idioma de origem → tradução fixa no idioma de
//! destino, com observação de contexto opcional) aplicados por padrão em todas
//! as traduções. Persistido em `glossary.toml` ao lado da config (ADR-004:
//! arquivos auxiliares seguem o mesmo diretório raiz). Sem fuzzy matching —
//! match exato (case-insensitive) no MVP.
//!
//! O glossário entra no prompt via `to_prompt_entries` (mesmo formato do bloco
//! de glossário do template 3.7). A observação de contexto é armazenada e
//! editável na UI, mas ainda não é renderizada no prompt (`ponytail:` o
//! caminho runtime do LLM ainda não consome o template 3.7 com opções/glossário
//! — débito herdado da 5.4; renderizar a nota quando esse fio for ligado).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::AppConfig;
use crate::errors::LegendaiError;

use super::prompt::GlossaryEntry as PromptGlossaryEntry;

/// Entrada do glossário: termo → tradução fixa, com observação de contexto
/// opcional (ex: "usar ao se referir à protagonista"). Serde para IPC (UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlossaryEntry {
    pub term: String,
    pub translation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Glossário usuário-mantido. Persistido como tabela `entries = [...]` (TOML
/// exige uma tabela no topo; array puro não serializa — `toml::ser`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Glossary {
    pub entries: Vec<GlossaryEntry>,
}

impl Glossary {
    /// Caminho do arquivo de glossário, ao lado da config (mesmo dir raiz).
    pub fn path() -> Result<std::path::PathBuf, LegendaiError> {
        AppConfig::glossary_path()
    }

    /// Carrega do diretório padrão. Ausente/corrompido → glossário vazio (com
    /// log de erro no segundo caso), seguindo o padrão da config (0.7).
    pub fn load() -> Self {
        match Self::path() {
            Ok(path) => Self::load_from(&path),
            Err(e) => {
                tracing::error!("falha ao localizar diretório de config: {e}");
                Self::default()
            }
        }
    }

    /// Carrega de um caminho específico (testável): arquivo ausente → vazio,
    /// arquivo inválido → vazio + log (nunca crasha).
    pub fn load_from(path: &Path) -> Self {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(_) => return Self::default(), // primeira execução
        };
        match toml::from_str::<Glossary>(&raw) {
            Ok(g) => g,
            Err(e) => {
                tracing::error!(
                    "glossário inválido em `{}`, usando vazio: {e}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Salva no diretório padrão com escrita atômica (temp + rename).
    pub fn save(&self) -> Result<(), LegendaiError> {
        self.save_to(&Self::path()?)
    }

    /// Escrita atômica: grava em `path.toml.tmp` e renomeia (mesmo padrão da
    /// config 0.7) — nunca deixa arquivo meio-escrito em caso de crash.
    pub fn save_to(&self, path: &Path) -> Result<(), LegendaiError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| LegendaiError::Io {
                path: dir.display().to_string(),
                source: e,
            })?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?).map_err(|e| LegendaiError::Io {
            path: tmp.display().to_string(),
            source: e,
        })?;
        std::fs::rename(&tmp, path).map_err(|e| LegendaiError::Io {
            path: path.display().to_string(),
            source: e,
        })
    }

    /// Busca exata (case-insensitive) por termo — match exato, sem fuzzy (nota
    /// da tarefa). `None` se não houver entrada correspondente.
    pub fn find(&self, term: &str) -> Option<&GlossaryEntry> {
        self.entries
            .iter()
            .find(|e| e.term.eq_ignore_ascii_case(term))
    }

    /// Adiciona uma entrada ou substitui a existente com o mesmo termo
    /// (case-insensitive) — garante termos únicos no glossário.
    pub fn upsert(&mut self, entry: GlossaryEntry) {
        match self
            .entries
            .iter_mut()
            .find(|e| e.term.eq_ignore_ascii_case(&entry.term))
        {
            Some(existing) => *existing = entry,
            None => self.entries.push(entry),
        }
    }

    /// Remove a entrada com o termo dado (case-insensitive). Retorna `true` se
    /// algo foi removido.
    pub fn remove(&mut self, term: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| !e.term.eq_ignore_ascii_case(term));
        self.entries.len() != before
    }

    /// Converte para as entradas do template de prompt (3.7) — mesmo tipo que o
    /// bloco de glossário de `build_prompt` consome.
    pub fn to_prompt_entries(&self) -> Vec<PromptGlossaryEntry> {
        self.entries
            .iter()
            .map(|e| PromptGlossaryEntry {
                term: e.term.clone(),
                translation: e.translation.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Language;
    use crate::translate::batcher::Batch;
    use crate::translate::engine::BatchSegment;
    use crate::translate::options::TranslationOptions;
    use crate::translate::prompt::{build_prompt, LanguagePair};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "legendai-glossary-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(term: &str, translation: &str) -> GlossaryEntry {
        GlossaryEntry {
            term: term.into(),
            translation: translation.into(),
            note: None,
        }
    }

    #[test]
    fn path_usa_dirs_config_dir_ao_lado_da_config() {
        if let Some(base) = dirs::config_dir() {
            assert_eq!(
                Glossary::path().unwrap(),
                base.join("legendai").join("glossary.toml")
            );
        }
    }

    #[test]
    fn round_trip_salva_e_carrega_entradas() {
        let dir = temp_dir("round-trip");
        let path = dir.join("glossary.toml");

        let mut g = Glossary::default();
        g.upsert(entry("Dragon", "Dragão"));
        g.upsert(GlossaryEntry {
            term: "Lannister".into(),
            translation: "Lannister".into(),
            note: Some("casa nobre".into()),
        });
        g.save_to(&path).unwrap();

        let loaded = Glossary::load_from(&path);
        assert_eq!(loaded, g);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arquivo_ausente_usa_vazio() {
        let dir = temp_dir("missing");
        assert_eq!(
            Glossary::load_from(&dir.join("glossary.toml")),
            Glossary::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arquivo_corrompido_usa_vazio_sem_crash() {
        let dir = temp_dir("corrupted");
        let path = dir.join("glossary.toml");
        std::fs::write(&path, "[[[ não é toml válido").unwrap();
        assert_eq!(Glossary::load_from(&path), Glossary::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn entrada_sem_nota_nao_quebra_carregamento() {
        let dir = temp_dir("no-note");
        let path = dir.join("glossary.toml");
        std::fs::write(
            &path,
            "[[entries]]\nterm = \"Dragon\"\ntranslation = \"Dragão\"\n",
        )
        .unwrap();
        let g = Glossary::load_from(&path);
        assert_eq!(
            g.entries,
            vec![GlossaryEntry {
                term: "Dragon".into(),
                translation: "Dragão".into(),
                note: None,
            }]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upsert_adiciona_e_substitui_por_termo_case_insensitive() {
        let mut g = Glossary::default();
        g.upsert(entry("Dragon", "Dragão"));
        g.upsert(entry("DRAGON", "Dragão (mítico)"));
        assert_eq!(g.entries.len(), 1);
        assert_eq!(g.entries[0].term, "DRAGON"); // substitui mantendo o termo novo
        assert_eq!(g.entries[0].translation, "Dragão (mítico)");
    }

    #[test]
    fn find_e_remove_sao_case_insensitive() {
        let mut g = Glossary::default();
        g.upsert(entry("dragon", "Dragão"));

        assert_eq!(g.find("Dragon").unwrap().translation, "Dragão");
        assert_eq!(g.find("DRAGON").unwrap().translation, "Dragão");
        assert!(g.find("wolf").is_none());

        assert!(g.remove("DRAGON"));
        assert!(g.find("dragon").is_none());
        assert!(!g.remove("dragon"));
        assert!(g.entries.is_empty());
    }

    /// Critério 1 da tarefa: entradas do glossário aparecem no prompt (snapshot
    /// do bloco de glossário do template 3.7).
    #[test]
    fn entradas_do_glossario_aparecem_no_prompt() {
        let mut g = Glossary::default();
        g.upsert(entry("Dragon", "Dragão"));
        g.upsert(GlossaryEntry {
            term: "Lannister".into(),
            translation: "Lannister".into(),
            note: Some("casa nobre".into()),
        });

        let batch = Batch {
            segments: vec![BatchSegment {
                id: 1,
                text: "Olá.".into(),
                context: vec![],
            }],
        };
        let prompt = build_prompt(
            &batch,
            &g.to_prompt_entries(),
            &LanguagePair {
                source: &Language::Pt,
                target: &Language::En,
            },
            &TranslationOptions::default(),
        );

        assert!(prompt.contains(
            "Glossário (termo → tradução):\n- Dragon → Dragão\n- Lannister → Lannister\n"
        ));
        // Nota é armazenada mas não renderizada no prompt (ver doc-comment do módulo).
        assert!(!prompt.contains("casa nobre"));
    }
}
