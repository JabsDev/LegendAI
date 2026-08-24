use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::LegendaiError;
use crate::translate::TranslationOptions;

pub const SCHEMA_VERSION: u32 = 1;

/// Teto da lista de arquivos recentes (tarefa 4.10) — nunca cresce além disso.
pub const MAX_RECENT_FILES: usize = 10;

/// Config persistente do app, serializada em TOML em
/// `dirs::config_dir()/legendai/config.toml`. Campos novos devem ter
/// `#[serde(default)]` para não quebrar arquivos antigos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    /// Idioma de origem da fala ("auto" = detectar no Whisper).
    pub source_lang: String,
    /// Idioma de destino da tradução.
    pub target_lang: String,
    /// Modelos ativos por tipo (preenchido na tarefa 2.10).
    pub active_models: ActiveModels,
    /// Diretório de cache de modelos; `None` = default da plataforma.
    pub model_cache: Option<PathBuf>,
    /// Nº de threads para inferência; `None` = detectar automaticamente.
    pub threads: Option<u32>,
    /// Engine de tradução ativa ("nllb" | "llm").
    pub translation_engine: String,
    /// Opções avançadas de tradução (tarefa 5.4): formalidade, instruções
    /// livres e nível de contexto — injetadas no template de prompt (3.7).
    pub translation_options: TranslationOptions,
    /// Últimos arquivos abertos/processados (tarefa 4.10), topo = mais recente.
    pub recent_files: Vec<String>,
    pub ui: UiPrefs,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ActiveModels {
    pub stt: String,
    pub translation: String,
}

/// Preferências de UI persistidas (tarefa 4.10). Campos novos precisam de
/// `#[serde(default)]` (via derive) para não quebrar configs antigas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiPrefs {
    /// "light" | "dark" (default claro — `system` de versões antigas é tratado
    /// como claro pelo frontend).
    pub theme: String,
    /// Idioma da interface ("pt" | "en").
    pub ui_language: String,
    /// Modo do preview duplo ("original" | "translated" | "both").
    pub preview_mode: String,
    /// Último diretório onde um SRT foi salvo.
    pub last_output_dir: Option<String>,
    /// Último par de idiomas usado (espelha `source_lang`/`target_lang`).
    pub last_language_pair: Option<LanguagePairPref>,
}

/// Par de idiomas origem→destino persistido nas preferências (4.10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguagePairPref {
    pub source: String,
    pub target: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            source_lang: "auto".into(),
            target_lang: "pt".into(),
            active_models: ActiveModels::default(),
            model_cache: None,
            threads: None,
            translation_engine: "nllb".into(),
            translation_options: TranslationOptions::default(),
            recent_files: Vec::new(),
            ui: UiPrefs::default(),
        }
    }
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: "light".into(),
            ui_language: "pt".into(),
            preview_mode: "translated".into(),
            last_output_dir: None,
            last_language_pair: None,
        }
    }
}

impl AppConfig {
    /// Caminho da config por plataforma (ex: `~/.config/legendai/config.toml`).
    pub fn config_path() -> Result<PathBuf, LegendaiError> {
        dirs::config_dir()
            .map(|d| d.join("legendai").join("config.toml"))
            .ok_or_else(|| {
                LegendaiError::ConfigDirMissing("dirs::config_dir() retornou None".into())
            })
    }

    /// Caminho do arquivo de glossário do usuário (tarefa 5.6), ao lado da
    /// config — arquivos auxiliares seguem o mesmo diretório raiz (ADR-004).
    pub fn glossary_path() -> Result<PathBuf, LegendaiError> {
        dirs::config_dir()
            .map(|d| d.join("legendai").join("glossary.toml"))
            .ok_or_else(|| {
                LegendaiError::ConfigDirMissing("dirs::config_dir() retornou None".into())
            })
    }

    /// Carrega a config do diretório padrão. Arquivo ausente/corrompido → erro tipado.
    #[allow(dead_code)] // API pública: versão estrita de `load_or_default` (futuro uso)
    pub fn load() -> Result<AppConfig, LegendaiError> {
        Self::load_from(&Self::config_path()?)
    }

    /// Carrega a config do diretório padrão com fallback: primeira execução ou
    /// arquivo inválido → defaults (com log de erro no segundo caso).
    pub fn load_or_default() -> AppConfig {
        match Self::config_path() {
            Ok(path) => Self::load_or_default_from(&path),
            Err(e) => {
                tracing::error!("falha ao localizar diretório de config: {e}");
                Self::default()
            }
        }
    }

    /// Salva no diretório padrão com escrita atômica (temp + rename).
    pub fn save(&self) -> Result<(), LegendaiError> {
        self.save_to(&Self::config_path()?)
    }

    /// Escrita atômica: grava em `path.toml.tmp` e renomeia — nunca deixa um
    /// arquivo meio-escrito em caso de crash no meio do save.
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

    fn load_from(path: &Path) -> Result<AppConfig, LegendaiError> {
        let raw = std::fs::read_to_string(path).map_err(|e| LegendaiError::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        let mut cfg: AppConfig = toml::from_str(&raw).map_err(|e| LegendaiError::Parse {
            path: path.display().to_string(),
            source: e,
        })?;
        cfg.migrate();
        Ok(cfg)
    }

    fn load_or_default_from(path: &Path) -> AppConfig {
        if !path.exists() {
            return Self::default(); // primeira execução — config é criada no primeiro save()
        }
        match Self::load_from(path) {
            Ok(cfg) => cfg,
            Err(e) => {
                tracing::error!(
                    "config inválida em `{}`, usando defaults: {e}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Migra uma config antiga para o schema atual. Hoje só bump de versão;
    /// migrações reais (0.x → 1.x) entram aqui quando o formato mudar.
    fn migrate(&mut self) {
        if self.schema_version < SCHEMA_VERSION {
            self.schema_version = SCHEMA_VERSION;
        }
    }

    /// Registra um job concluído nas "últimas escolhas" (4.10): adiciona o vídeo
    /// aos recentes e lembra o diretório do SRT de saída. Chamado no sucesso do
    /// pipeline (não falha o job se o save falhar).
    pub fn record_recent(&mut self, input_path: &str, output_path: &Path) {
        self.push_recent(input_path);
        if let Some(dir) = output_path.parent() {
            self.ui.last_output_dir = Some(dir.display().to_string());
        }
    }

    /// Adiciona um caminho à lista de recentes: move para o topo, sem
    /// duplicatas e com teto em [`MAX_RECENT_FILES`] (nunca cresce sem limite).
    pub fn push_recent(&mut self, path: &str) {
        self.recent_files.retain(|p| p != path);
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(MAX_RECENT_FILES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "legendai-config-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trip_default() {
        let dir = temp_dir("round-trip");
        let path = dir.join("config.toml");

        AppConfig::default().save_to(&path).unwrap();
        assert_eq!(AppConfig::load_from(&path).unwrap(), AppConfig::default());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trip_modified() {
        let dir = temp_dir("round-trip-modified");
        let path = dir.join("config.toml");

        let cfg = AppConfig {
            target_lang: "en".into(),
            threads: Some(8),
            active_models: ActiveModels {
                stt: "whisper-small-q5".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        cfg.save_to(&path).unwrap();

        let loaded = AppConfig::load_from(&path).unwrap();
        assert_eq!(loaded.target_lang, "en");
        assert_eq!(loaded.threads, Some(8));
        assert_eq!(loaded.active_models.stt, "whisper-small-q5");
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn active_models_round_trip_persiste_entre_execucoes() {
        let dir = temp_dir("active-models");
        let path = dir.join("config.toml");

        let cfg = AppConfig {
            active_models: ActiveModels {
                stt: "whisper-small-q5".into(),
                translation: "nllb-200-distilled-600m-q4".into(),
            },
            ..Default::default()
        };
        cfg.save_to(&path).unwrap(); // "execução 1"

        let loaded = AppConfig::load_from(&path).unwrap(); // "execução 2" (restart)
        assert_eq!(loaded.active_models.stt, "whisper-small-q5");
        assert_eq!(
            loaded.active_models.translation,
            "nllb-200-distilled-600m-q4"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arquivo_corrompido_usa_defaults() {
        let dir = temp_dir("corrupted");
        let path = dir.join("config.toml");
        std::fs::write(&path, "[[[ não é toml válido").unwrap();

        assert!(AppConfig::load_from(&path).is_err());
        assert_eq!(AppConfig::load_or_default_from(&path), AppConfig::default());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn arquivo_ausente_usa_defaults() {
        let dir = temp_dir("missing");
        let path = dir.join("config.toml");

        assert_eq!(AppConfig::load_or_default_from(&path), AppConfig::default());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn campos_ausentes_nao_quebram_arquivo_antigo() {
        let dir = temp_dir("missing-fields");
        let path = dir.join("config.toml");
        std::fs::write(&path, "target_lang = \"en\"\n").unwrap();

        let cfg = AppConfig::load_from(&path).unwrap();
        assert_eq!(cfg.target_lang, "en");
        assert_eq!(cfg.source_lang, "auto"); // default aplicado
        assert_eq!(cfg.translation_options, TranslationOptions::default()); // default aplicado

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn translation_options_round_trip_persistem_entre_execucoes() {
        let dir = temp_dir("translation-options");
        let path = dir.join("config.toml");

        let cfg = AppConfig {
            translation_options: TranslationOptions {
                formality: crate::translate::Formality::Formal,
                custom_instructions: "preservar apelidos".into(),
                context_size: 1,
            },
            ..Default::default()
        };
        cfg.save_to(&path).unwrap(); // "execução 1"

        let loaded = AppConfig::load_from(&path).unwrap(); // "execução 2" (restart)
        assert_eq!(
            loaded.translation_options.formality,
            crate::translate::Formality::Formal
        );
        assert_eq!(
            loaded.translation_options.custom_instructions,
            "preservar apelidos"
        );
        assert_eq!(loaded.translation_options.context_size, 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn migrate_bump_schema_version() {
        let dir = temp_dir("migrate");
        let path = dir.join("config.toml");
        std::fs::write(&path, "schema_version = 0\n").unwrap();

        let cfg = AppConfig::load_from(&path).unwrap();
        assert_eq!(cfg.schema_version, SCHEMA_VERSION);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn config_path_usa_dirs_config_dir() {
        if let Some(expected) = dirs::config_dir() {
            assert_eq!(
                AppConfig::config_path().unwrap(),
                expected.join("legendai").join("config.toml")
            );
        }
    }

    #[test]
    fn push_recent_dedup_e_cap_dez() {
        let mut cfg = AppConfig::default();
        for i in 0..15 {
            cfg.push_recent(&format!("/videos/f{i}.mp4"));
        }
        assert_eq!(cfg.recent_files.len(), MAX_RECENT_FILES);
        assert_eq!(cfg.recent_files[0], "/videos/f14.mp4");
        assert!(!cfg.recent_files.contains(&"/videos/f4.mp4".to_string()));

        // Reinserir o topo não duplica; reinserir um antigo move para o topo.
        cfg.push_recent("/videos/f14.mp4");
        assert_eq!(cfg.recent_files.len(), MAX_RECENT_FILES);
        cfg.push_recent("/videos/f9.mp4");
        assert_eq!(cfg.recent_files.len(), MAX_RECENT_FILES);
        assert_eq!(cfg.recent_files[0], "/videos/f9.mp4");
    }

    #[test]
    fn preferencias_round_trip_persistem_entre_execucoes() {
        let dir = temp_dir("prefs");
        let path = dir.join("config.toml");

        let cfg = AppConfig {
            source_lang: "es".into(),
            target_lang: "de".into(),
            ui: UiPrefs {
                theme: "dark".into(),
                ui_language: "en".into(),
                preview_mode: "both".into(),
                last_output_dir: Some("/tmp/out".into()),
                last_language_pair: Some(LanguagePairPref {
                    source: "es".into(),
                    target: "de".into(),
                }),
            },
            ..Default::default()
        };
        cfg.save_to(&path).unwrap(); // "execução 1"

        let loaded = AppConfig::load_from(&path).unwrap(); // "execução 2" (restart)
        assert_eq!(loaded.ui.theme, "dark");
        assert_eq!(loaded.ui.ui_language, "en");
        assert_eq!(loaded.ui.preview_mode, "both");
        assert_eq!(loaded.ui.last_output_dir.as_deref(), Some("/tmp/out"));
        assert_eq!(loaded.source_lang, "es");
        assert_eq!(loaded.target_lang, "de");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn record_recent_grava_arquivo_e_diretorio_de_saida() {
        let mut cfg = AppConfig::default();
        cfg.record_recent("/videos/a.mp4", Path::new("/saida/out/a.srt"));
        assert_eq!(cfg.recent_files, vec!["/videos/a.mp4"]);
        assert_eq!(cfg.ui.last_output_dir.as_deref(), Some("/saida/out"));

        // Segunda gravação mantém o teto e atualiza o diretório.
        cfg.record_recent("/videos/b.mp4", Path::new("/out2/b.srt"));
        assert_eq!(cfg.recent_files, vec!["/videos/b.mp4", "/videos/a.mp4"]);
        assert_eq!(cfg.ui.last_output_dir.as_deref(), Some("/out2"));
    }
}
