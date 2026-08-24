//! Catálogo curado de modelos (tarefa 2.1).
//!
//! O manifesto vive em `catalog/models.json`, é embutido no binário em
//! build-time via [`include_str!`] e validado em runtime por [`Catalog::embedded`].
//! Consumido pelas Fases 2-3: recomendação por tier (2.6), download (2.2),
//! seleção de modelo ativo (2.10) e factory de engines de tradução (3.4).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Tipo de modelo: transcrição (STT) ou tradução.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Stt,
    Translation,
}

/// Backend que executa o modelo (ver ADR-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Whisper,
    Llama,
    Ort,
    Parakeet,
    Canary,
    Nemotron,
}

/// Um modelo do catálogo curado.
///
/// `file` é o arquivo principal (identidade e checksum na 2.3). `files`, quando
/// presente, é a lista ordenada de TODOS os arquivos do download — cobre GGUF
/// split (ex: Qwen 7B em 2 partes) e modelos multi-arquivo (ex: NLLB ONNX com
/// encoder + decoder + tokenizer). `languages` só se aplica a tradução (pares
/// suportados entre os códigos ISO 639-1 listados).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub kind: ModelKind,
    pub name: String,
    pub repo_id: String,
    pub file: String,
    #[serde(default)]
    pub files: Vec<String>,
    pub backend: Backend,
    pub quantization: String,
    pub size_mb: u64,
    pub min_ram_gb: u32,
    pub quality: u8,
    pub speed: u8,
    pub threads_supported: bool,
    /// Idiomas suportados (somente para `kind == Translation`; ausente no STT).
    pub languages: Option<Vec<String>>,
    /// SHA256 hex (64 chars) do arquivo principal [`Self::file`], quando
    /// conhecido. `None` (ou `null` no JSON) → verificação de integridade é
    /// pulada com aviso (2.3) — hashes mudam se o repo HF atualizar o arquivo.
    pub sha256: Option<String>,
}

/// Catálogo completo (top-level do `models.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub version: u32,
    pub models: Vec<ModelInfo>,
}

/// Erros de carregamento/validação do catálogo.
#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("catálogo embutido não parseia: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("catálogo inválido: {0}")]
    Invalid(String),
}

impl Catalog {
    /// Carrega e valida o catálogo embutido em build-time (`catalog/models.json`).
    pub fn embedded() -> Result<Self, CatalogError> {
        let catalog: Catalog = serde_json::from_str(include_str!("../../../catalog/models.json"))?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Valida invariantes do catálogo: IDs únicos, campos obrigatórios,
    /// ranges de `quality`/`speed`, consistência de `files`/`languages` e
    /// que `file` conste em `files` quando multi-arquivo.
    pub fn validate(&self) -> Result<(), CatalogError> {
        let mut errs: Vec<String> = Vec::new();
        if self.version == 0 {
            errs.push("version deve ser > 0".into());
        }
        if self.models.is_empty() {
            errs.push("catálogo sem modelos".into());
        }
        let mut ids = HashSet::new();
        for m in &self.models {
            let id = m.id.as_str();
            for (field, value) in [
                ("id", id),
                ("name", m.name.as_str()),
                ("repo_id", m.repo_id.as_str()),
                ("file", m.file.as_str()),
            ] {
                if value.trim().is_empty() {
                    errs.push(format!("modelo `{id}`: `{field}` vazio"));
                }
            }
            if !ids.insert(id.to_string()) {
                errs.push(format!("id duplicado: `{id}`"));
            }
            if !(1..=5).contains(&m.quality) {
                errs.push(format!(
                    "modelo `{id}`: quality {} fora de 1..=5",
                    m.quality
                ));
            }
            if !(1..=5).contains(&m.speed) {
                errs.push(format!("modelo `{id}`: speed {} fora de 1..=5", m.speed));
            }
            if m.min_ram_gb == 0 {
                errs.push(format!("modelo `{id}`: min_ram_gb deve ser > 0"));
            }
            if m.size_mb == 0 {
                errs.push(format!("modelo `{id}`: size_mb deve ser > 0"));
            }
            if let Some(h) = &m.sha256 {
                if !(h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())) {
                    errs.push(format!("modelo `{id}`: sha256 malformado `{h}`"));
                }
            }
            if !m.files.is_empty() && !m.files.contains(&m.file) {
                errs.push(format!("modelo `{id}`: `file` não consta em `files`"));
            }
            match (&m.kind, &m.languages) {
                (ModelKind::Stt, Some(_)) => {
                    errs.push(format!("modelo `{id}`: stt não deve declarar `languages`"));
                }
                (ModelKind::Translation, None) => {
                    errs.push(format!("modelo `{id}`: tradução exige `languages`"));
                }
                _ => {}
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(CatalogError::Invalid(errs.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::*;

    fn model(id: &str) -> Value {
        json!({
            "id": id,
            "kind": "stt",
            "name": "Teste",
            "repo_id": "owner/repo",
            "file": "model.bin",
            "backend": "whisper",
            "quantization": "q5",
            "size_mb": 100,
            "min_ram_gb": 2,
            "quality": 3,
            "speed": 3,
            "threads_supported": true
        })
    }

    fn catalog(models: Vec<Value>) -> Catalog {
        serde_json::from_value(json!({ "version": 1, "models": models })).unwrap()
    }

    #[test]
    fn catalogo_embutido_parseia_e_valida() {
        let cat = Catalog::embedded().expect("catálogo embutido deve validar");
        assert!(cat.version > 0);
        assert!(
            (15..=30).contains(&cat.models.len()),
            "esperado 15-30 modelos (multilíngues Handy + HF), tem {}",
            cat.models.len()
        );
        assert!(cat.models.iter().any(|m| m.kind == ModelKind::Stt));
        assert!(cat.models.iter().any(|m| m.kind == ModelKind::Translation));
        for m in &cat.models {
            assert!(!m.id.is_empty() && !m.repo_id.is_empty() && !m.file.is_empty());
            assert!(m.size_mb > 0 && m.min_ram_gb > 0);
            assert!((1..=5).contains(&m.quality) && (1..=5).contains(&m.speed));
        }
    }

    #[test]
    fn catalogo_inclui_whisper_q5_e_engines_do_adr001() {
        let cat = Catalog::embedded().unwrap();
        let ids: Vec<&str> = cat.models.iter().map(|m| m.id.as_str()).collect();
        for expected in [
            "whisper-tiny",
            "whisper-small-q5",
            "whisper-medium-q5",
            "whisper-large-v3-q5",
            "nllb-200-distilled-600m",
            "nllb-200-distilled-600m-q4",
            "towerinstruct-7b-q4_k_m",
            "towerinstruct-7b-q6_k",
        ] {
            assert!(ids.contains(&expected), "catálogo deve conter `{expected}`");
        }
        // Backends dos tiers (ADR-001): whisper (STT), ort (NLLB), llama (Tower).
        assert_eq!(
            cat.models
                .iter()
                .filter(|m| m.kind == ModelKind::Translation)
                .map(|m| m.backend)
                .collect::<HashSet<_>>(),
            HashSet::from([Backend::Ort, Backend::Llama])
        );
    }

    #[test]
    fn rejeita_ids_duplicados() {
        let err = catalog(vec![model("a"), model("a")])
            .validate()
            .unwrap_err();
        assert!(err.to_string().contains("id duplicado: `a`"));
    }

    #[test]
    fn rejeita_quality_e_speed_fora_do_range() {
        for key in ["quality", "speed"] {
            let mut m = model("a");
            m[key] = json!(6);
            let err = catalog(vec![m]).validate().unwrap_err();
            assert!(err.to_string().contains("fora de 1..=5"), "{key}");
        }
    }

    #[test]
    fn rejeita_stt_com_languages() {
        let mut m = model("a");
        m["languages"] = json!(["pt", "en"]);
        let err = catalog(vec![m]).validate().unwrap_err();
        assert!(err
            .to_string()
            .contains("stt não deve declarar `languages`"));
    }

    #[test]
    fn rejeita_translation_sem_languages() {
        let mut m = model("a");
        m["kind"] = json!("translation");
        m["backend"] = json!("llama");
        let err = catalog(vec![m]).validate().unwrap_err();
        assert!(err.to_string().contains("tradução exige `languages`"));
    }

    #[test]
    fn rejeita_files_que_nao_inclui_file() {
        let mut m = model("a");
        m["files"] = json!(["outro.bin"]);
        let err = catalog(vec![m]).validate().unwrap_err();
        assert!(err.to_string().contains("`file` não consta em `files`"));
    }

    #[test]
    fn aceita_files_com_file_incluido() {
        let mut m = model("a");
        m["files"] = json!(["model.bin", "extra.bin"]);
        assert!(catalog(vec![m]).validate().is_ok());
    }

    #[test]
    fn aceita_multi_arquivo_com_file_primario() {
        let cat = Catalog::embedded().unwrap();
        let nllb = cat
            .models
            .iter()
            .find(|m| m.id == "nllb-200-distilled-600m")
            .unwrap();
        assert!(!nllb.files.is_empty());
        assert!(nllb.files.contains(&nllb.file));
        // Tower 7B é single-file GGUF (não split)
        let tower = cat
            .models
            .iter()
            .find(|m| m.id == "towerinstruct-7b-q6_k")
            .unwrap();
        assert!(tower.files.is_empty());
        assert!(!tower.file.is_empty());
    }

    #[test]
    fn model_info_round_trip_serde() {
        let m = catalog(vec![model("a")]).models.remove(0);
        let back: ModelInfo = serde_json::from_value(serde_json::to_value(&m).unwrap()).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn languages_ausente_no_stt_nao_quebra_parse() {
        // Campo `languages` omitido (campo de tradução) → None no STT.
        let m = model("a");
        let parsed: ModelInfo = serde_json::from_value(m).unwrap();
        assert_eq!(parsed.languages, None);
    }

    #[test]
    fn rejeita_sha256_malformado() {
        let mut m = model("a");
        m["sha256"] = json!("xyz");
        let err = catalog(vec![m]).validate().unwrap_err();
        assert!(err.to_string().contains("sha256 malformado"));
    }

    #[test]
    fn aceita_sha256_hex_valido() {
        let mut m = model("a");
        m["sha256"] = json!("be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21");
        assert!(catalog(vec![m]).validate().is_ok());
    }

    #[test]
    fn sha256_ausente_nao_quebra_parse() {
        let m = model("a");
        let parsed: ModelInfo = serde_json::from_value(m).unwrap();
        assert_eq!(parsed.sha256, None);
    }

    #[test]
    fn catalogo_embutido_tem_sha256_para_arquivos_principais() {
        let cat = Catalog::embedded().unwrap();
        let whisper = cat.models.iter().find(|m| m.id == "whisper-tiny").unwrap();
        assert_eq!(
            whisper.sha256.as_deref(),
            Some("be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21")
        );
    }
}
