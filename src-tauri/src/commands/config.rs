//! Comandos IPC de preferências do usuário (tarefa 4.10).
//!
//! `get_prefs` devolve as preferências persistidas (tema, idioma, modo de
//! preview, último diretório de saída, último par de idiomas e arquivos
//! recentes). `set_prefs` aplica um *patch* parcial — só os campos `Some` são
//! gravados — e salva na config com escrita atômica (0.7). O frontend grava com
//! debounce de 500ms para não escrever disco a cada mudança.

use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, LanguagePairPref};
use crate::translate::glossary::{Glossary, GlossaryEntry};
use crate::translate::TranslationOptions;

/// Preferências completas do usuário para a UI (serde/IPC).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Prefs {
    pub theme: String,
    pub ui_language: String,
    pub preview_mode: String,
    pub last_output_dir: Option<String>,
    /// Par origem→destino (espelhado de `AppConfig.source_lang/target_lang`).
    pub last_language_pair: Option<LanguagePairPref>,
    /// Arquivos recentes, do mais recente para o mais antigo (máx. 10).
    pub recent_files: Vec<String>,
    /// Opções avançadas de tradução (tarefa 5.4).
    pub translation_options: TranslationOptions,
}

/// Patch parcial de preferências: só os campos presentes (`Some`) são aplicados.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PrefsPatch {
    pub theme: Option<String>,
    pub ui_language: Option<String>,
    pub preview_mode: Option<String>,
    pub last_output_dir: Option<String>,
    pub last_language_pair: Option<LanguagePairPref>,
    /// Adiciona um arquivo à lista de recentes (dedup + topo + cap 10).
    pub recent_file: Option<String>,
    /// Substitui as opções avançadas de tradução por inteiro (5.4).
    pub translation_options: Option<TranslationOptions>,
}

impl From<AppConfig> for Prefs {
    fn from(cfg: AppConfig) -> Self {
        Self {
            theme: cfg.ui.theme,
            ui_language: cfg.ui.ui_language,
            preview_mode: cfg.ui.preview_mode,
            last_output_dir: cfg.ui.last_output_dir,
            last_language_pair: Some(LanguagePairPref {
                source: cfg.source_lang.clone(),
                target: cfg.target_lang.clone(),
            }),
            recent_files: cfg.recent_files,
            translation_options: cfg.translation_options,
        }
    }
}

/// Aplica um patch parcial em `cfg` (núcleo testável, sem tocar disco).
fn apply_patch(cfg: &mut AppConfig, patch: &PrefsPatch) {
    if let Some(t) = &patch.theme {
        cfg.ui.theme = t.clone();
    }
    if let Some(l) = &patch.ui_language {
        cfg.ui.ui_language = l.clone();
    }
    if let Some(m) = &patch.preview_mode {
        cfg.ui.preview_mode = m.clone();
    }
    if let Some(d) = &patch.last_output_dir {
        cfg.ui.last_output_dir = Some(d.clone());
    }
    if let Some(pair) = &patch.last_language_pair {
        if !pair.source.is_empty() {
            cfg.source_lang = pair.source.clone();
        }
        if !pair.target.is_empty() {
            cfg.target_lang = pair.target.clone();
        }
    }
    if let Some(path) = &patch.recent_file {
        if !path.is_empty() {
            cfg.push_recent(path);
        }
    }
    if let Some(o) = &patch.translation_options {
        cfg.translation_options = o.clone();
    }
}

/// Devolve as preferências persistidas para a UI restaurar no boot (4.10).
#[tauri::command(rename_all = "snake_case")]
pub fn get_prefs() -> Result<Prefs, String> {
    Ok(Prefs::from(AppConfig::load_or_default()))
}

/// Aplica um patch parcial de preferências e salva na config (merge). Retorna
/// as preferências atualizadas.
#[tauri::command(rename_all = "snake_case")]
pub fn set_prefs(patch: PrefsPatch) -> Result<Prefs, String> {
    let mut cfg = AppConfig::load_or_default();
    apply_patch(&mut cfg, &patch);
    cfg.save().map_err(|e| e.to_string())?;
    Ok(Prefs::from(cfg))
}

/// Devolve as entradas do glossário do usuário (tarefa 5.6). Ausente/vazio →
/// lista vazia (sem erro).
#[tauri::command(rename_all = "snake_case")]
pub fn get_glossary() -> Result<Vec<GlossaryEntry>, String> {
    Ok(Glossary::load().entries)
}

/// Substitui as entradas do glossário por inteiro e persiste em `glossary.toml`
/// (escrita atômica). Termos duplicados (case-insensitive) são deduplicados via
/// `upsert` — o prompt não deve repetir um termo. Retorna a lista persistida.
#[tauri::command(rename_all = "snake_case")]
pub fn set_glossary(entries: Vec<GlossaryEntry>) -> Result<Vec<GlossaryEntry>, String> {
    let mut g = Glossary::default();
    for e in entries {
        g.upsert(e);
    }
    g.save().map_err(|e| e.to_string())?;
    Ok(g.entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_patch_so_toca_campos_presentes() {
        let mut cfg = AppConfig::default();
        apply_patch(
            &mut cfg,
            &PrefsPatch {
                preview_mode: Some("both".into()),
                ..Default::default()
            },
        );
        assert_eq!(cfg.ui.preview_mode, "both");
        assert_eq!(cfg.ui.theme, "light"); // não tocado
        assert_eq!(cfg.ui.ui_language, "pt"); // não tocado
        assert_eq!(cfg.source_lang, "auto"); // não tocado
    }

    #[test]
    fn apply_patch_par_de_idiomas_escreve_source_e_target() {
        let mut cfg = AppConfig::default();
        apply_patch(
            &mut cfg,
            &PrefsPatch {
                last_language_pair: Some(LanguagePairPref {
                    source: "es".into(),
                    target: "pt".into(),
                }),
                ..Default::default()
            },
        );
        assert_eq!(cfg.source_lang, "es");
        assert_eq!(cfg.target_lang, "pt");
    }

    #[test]
    fn apply_patch_recent_file_so_adiciona_quando_nao_vazio() {
        let mut cfg = AppConfig::default();
        apply_patch(
            &mut cfg,
            &PrefsPatch {
                recent_file: Some("".into()),
                ..Default::default()
            },
        );
        assert!(cfg.recent_files.is_empty());

        apply_patch(
            &mut cfg,
            &PrefsPatch {
                recent_file: Some("/v.mp4".into()),
                ..Default::default()
            },
        );
        assert_eq!(cfg.recent_files, vec!["/v.mp4"]);
    }

    #[test]
    fn prefs_deriva_par_de_idiomas_da_config() {
        let cfg = AppConfig {
            source_lang: "en".into(),
            target_lang: "fr".into(),
            ..Default::default()
        };
        let p = Prefs::from(cfg);
        assert_eq!(p.theme, "light");
        assert_eq!(p.ui_language, "pt");
        let pair = p.last_language_pair.unwrap();
        assert_eq!(pair.source, "en");
        assert_eq!(pair.target, "fr");
        // Opções avançadas de tradução derivam da config (5.4).
        assert_eq!(p.translation_options, TranslationOptions::default());
    }

    #[test]
    fn apply_patch_aplica_translation_options() {
        let mut cfg = AppConfig::default();
        apply_patch(
            &mut cfg,
            &PrefsPatch {
                translation_options: Some(TranslationOptions {
                    formality: crate::translate::Formality::Formal,
                    custom_instructions: "preservar apelidos".into(),
                    context_size: 1,
                }),
                ..Default::default()
            },
        );
        assert_eq!(
            cfg.translation_options.formality,
            crate::translate::Formality::Formal
        );
        assert_eq!(
            cfg.translation_options.custom_instructions,
            "preservar apelidos"
        );
        assert_eq!(cfg.translation_options.context_size, 1);
        assert_eq!(cfg.ui.theme, "light"); // não tocado
    }

    #[test]
    fn apply_patch_nao_toca_translation_options_quando_ausente() {
        let mut cfg = AppConfig {
            translation_options: TranslationOptions {
                formality: crate::translate::Formality::Formal,
                custom_instructions: "x".into(),
                context_size: 5,
            },
            ..Default::default()
        };
        apply_patch(&mut cfg, &PrefsPatch::default());
        assert_eq!(
            cfg.translation_options.formality,
            crate::translate::Formality::Formal
        );
        assert_eq!(cfg.translation_options.context_size, 5);
    }
}
