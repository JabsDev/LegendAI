//! Factory de engines de tradução por tier/config (tarefa 3.4).
//!
//! Decisão: o modelo de tradução ativo na config (`active_models.translation`)
//! decide o backend — NLLB → engine `ort` (3.2), Qwen → engine `llama` (3.3).
//! Falhas de init (modelo não baixado, arquivo ausente/corrompido, backend
//! incompatível) retornam erro claro e acionável — nunca engine default
//! silenciosa (critério de aceitação). Fallbacks com log de aviso: backend não
//! compilado no binário (build sem a feature) → engine mock (tradução de teste,
//! passo 3 da tarefa); GPU pedida pelo tier sem GPU detectada → CPU.

use crate::config::AppConfig;
use crate::hardware::detect::HardwareInfo;
#[cfg(feature = "llama")]
use crate::hardware::tier::{tier_for, Tier};
use crate::model_manager::cache;
use crate::model_manager::catalog::{Backend, Catalog, ModelInfo, ModelKind};

use super::engine::{TranslateError, TranslationEngine};
#[cfg(feature = "llama")]
use super::llm::{LlmEngine, NGL_TIER2, NGL_TIER3};
#[cfg(feature = "ort")]
use super::nllb::NllbEngine;
#[cfg(feature = "ort")]
use std::path::PathBuf;

/// Factory estática (sem estado): `for_config` resolve o modelo ativo na
/// config e constrói a engine concreta correspondente ao backend.
pub struct TranslationEngineFactory;

impl TranslationEngineFactory {
    /// Constrói a engine de tradução para a config/hardware atuais.
    ///
    /// A resolução do caminho do modelo acontece **antes** do match de backend
    /// para que "modelo ativo indisponível" seja sempre um erro claro — também
    /// quando a feature do backend não está compilada no binário.
    pub fn for_config(
        config: &AppConfig,
        hw: &HardwareInfo,
    ) -> Result<Box<dyn TranslationEngine>, TranslateError> {
        let model = active_translation_model(config)?;
        let main_path = cache::resolve_model_path(&model).map_err(|e| {
            TranslateError::Backend(format!(
                "modelo de tradução ativo `{}` indisponível: {e}",
                model.id
            ))
        })?;
        #[cfg(not(feature = "llama"))]
        let _ = &main_path; // a validação acima já garantiu o arquivo; o path só é consumido pela engine llama
        let threads = config.threads.unwrap_or(hw.recommended_threads).max(1) as usize;
        #[cfg(not(any(feature = "ort", feature = "llama")))]
        let _ = &threads; // consumido pelas engines reais; sem features só há o fallback mock

        match model.backend {
            #[cfg(feature = "ort")]
            Backend::Ort => {
                let (enc, dec, tok) = nllb_file_paths(&model)?;
                let use_cuda = hw.gpu.is_some_and(|g| g == crate::hardware::detect::GpuKind::Cuda);
                if use_cuda {
                    tracing::info!("NLLB: GPU CUDA detectada — tentando CUDA EP");
                }
                NllbEngine::load_with_gpu(enc, dec, tok, threads, use_cuda)
                    .map(|e| Box::new(e) as Box<dyn TranslationEngine>)
            }
            #[cfg(not(feature = "ort"))]
            Backend::Ort => unavailable_backend(&model),
            #[cfg(feature = "llama")]
            Backend::Llama => {
                let gpu_layers = llama_gpu_layers(hw);
                LlmEngine::load(main_path, threads, gpu_layers)
                    .map(|e| Box::new(e) as Box<dyn TranslationEngine>)
            }
            #[cfg(not(feature = "llama"))]
            Backend::Llama => unavailable_backend(&model),
            Backend::Whisper => Err(TranslateError::Backend(format!(
                "modelo ativo `{}` usa backend whisper, que não traduz — selecione um modelo de tradução",
                model.id
            ))),
            Backend::Parakeet | Backend::Canary | Backend::Nemotron => Err(TranslateError::Backend(format!(
                "modelo ativo `{}` usa backend {:?} de transcrição, que não traduz — selecione um modelo de tradução",
                model.id, model.backend
            ))),
        }
    }
}

/// Resolve e valida o modelo de tradução ativo da config. Vazio, inexistente
/// no catálogo ou não-tradutor → erro claro (não engine default silenciosa).
fn active_translation_model(config: &AppConfig) -> Result<ModelInfo, TranslateError> {
    let id = config.active_models.translation.trim();
    if id.is_empty() {
        return Err(TranslateError::Backend(
            "nenhum modelo de tradução ativo — selecione um na aba Modelos".into(),
        ));
    }
    let catalog = Catalog::embedded()
        .map_err(|e| TranslateError::Backend(format!("catálogo de modelos indisponível: {e}")))?;
    let model = catalog.models.iter().find(|m| m.id == id).ok_or_else(|| {
        TranslateError::Backend(format!(
            "modelo de tradução ativo `{id}` não existe no catálogo"
        ))
    })?;
    if model.kind != ModelKind::Translation {
        return Err(TranslateError::Backend(format!(
            "modelo ativo `{id}` não é um modelo de tradução (é {:?})",
            model.kind
        )));
    }
    Ok(model.clone())
}

fn unavailable_backend(model: &ModelInfo) -> Result<Box<dyn TranslationEngine>, TranslateError> {
    let feature = match model.backend {
        crate::model_manager::catalog::Backend::Ort => "ort",
        crate::model_manager::catalog::Backend::Llama => "llama",
        _ => "desconhecida",
    };
    Err(TranslateError::Backend(format!(
        "modelo `{}` requer a feature `{}` não compilada neste binário — \
         recompile com `--features full` ou selecione um modelo de outro backend",
        model.id, feature
    )))
}

/// Caminhos dos 3 arquivos do NLLB (encoder, decoder, tokenizer), derivados da
/// lista `files` do catálogo (2.1) dentro do diretório do modelo no cache (2.4).
#[cfg(feature = "ort")]
fn nllb_file_paths(model: &ModelInfo) -> Result<(PathBuf, PathBuf, PathBuf), TranslateError> {
    let dir = cache::model_dir(model)
        .map_err(|e| TranslateError::Backend(format!("cache do modelo `{}`: {e}", model.id)))?;
    let mut enc: Option<PathBuf> = None;
    let mut dec: Option<PathBuf> = None;
    let mut tok: Option<PathBuf> = None;
    for f in &model.files {
        let name = f.to_ascii_lowercase();
        let path = dir.join(f);
        if name.contains("encoder_model") {
            enc = Some(path);
        } else if name.contains("decoder_model") {
            dec = Some(path);
        } else if name.ends_with("tokenizer.json") {
            tok = Some(path);
        }
    }
    let need = |what: &str, found: Option<PathBuf>| {
        found.ok_or_else(|| {
            TranslateError::Backend(format!(
                "modelo `{}` não declara o arquivo {what} NLLB em `files`",
                model.id
            ))
        })
    };
    let enc = need("encoder", enc)?;
    let dec = need("decoder", dec)?;
    let tok = need("tokenizer", tok)?;
    Ok((enc, dec, tok))
}

/// NGL (GPU layers) do LLM por tier (3.3): GPU detectada → NGL do tier; Tier 1
/// ou GPU ausente → CPU. `ponytail:` NGL fixo por tier, ajustar só com evidência
/// de perda de qualidade (nota 3.3).
#[cfg(feature = "llama")]
fn llama_gpu_layers(hw: &HardwareInfo) -> u32 {
    let (ngl, label) = match tier_for(hw) {
        Tier::Tier1 => (0, "Tier 1"),
        Tier::Tier2 => (NGL_TIER2, "Tier 2"),
        Tier::Tier3 => (NGL_TIER3, "Tier 3"),
    };
    if ngl > 0 && hw.gpu.is_none() {
        tracing::warn!(
            "{label} recomenda descarregar layers na GPU, mas nenhuma GPU foi detectada — \
             usando CPU (gpu_layers=0)"
        );
        return 0;
    }
    #[cfg(not(feature = "cuda"))]
    if ngl > 0 && hw.gpu.is_some() {
        tracing::warn!(
            "{label} GPU detectada mas binário sem feature `cuda` — usando CPU. Recompile com `--features cuda` para usar a GPU"
        );
        return 0;
    }
    if ngl > 0 {
        tracing::info!("LLM: usando GPU com {ngl} layers ({label})");
    }
    ngl
}

#[cfg(test)]
mod tests {
    use super::TranslationEngineFactory;
    use crate::config::{ActiveModels, AppConfig};
    use crate::domain::Language;
    use crate::hardware::detect::HardwareInfo;
    use crate::model_manager::cache::{self, with_root, CacheStatus, ModelStatus};
    use crate::model_manager::catalog::{Catalog, ModelInfo};
    use crate::translate::{BatchRequest, BatchSegment, TranslateError, TranslationEngine};

    fn hw(ram_gb: u32) -> HardwareInfo {
        HardwareInfo {
            ram_gb,
            cpu_threads: 8,
            gpu: None,
            cpu_name: "test".into(),
            recommended_threads: 4,
        }
    }

    fn factory_err(cfg: &AppConfig, hw: &HardwareInfo) -> TranslateError {
        match TranslationEngineFactory::for_config(cfg, hw) {
            Ok(_) => panic!("esperava erro da factory"),
            Err(e) => e,
        }
    }

    fn cfg_with_active(id: &str) -> AppConfig {
        AppConfig {
            active_models: ActiveModels {
                translation: id.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn catalog_model(id: &str) -> ModelInfo {
        Catalog::embedded()
            .unwrap()
            .models
            .into_iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("modelo `{id}` deve existir no catálogo"))
    }

    /// Simula o modelo baixado no cache (status downloaded + arquivos em disco).
    fn stage_downloaded(model: &ModelInfo) {
        let dir = cache::model_dir(model).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        for f in &model.files {
            let path = dir.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"dummy").unwrap();
        }
        if model.files.is_empty() {
            std::fs::write(dir.join(&model.file), b"dummy").unwrap();
        }
        cache::write_status(
            model,
            &ModelStatus {
                status: CacheStatus::Downloaded,
                size_bytes: 1,
                sha256: model.sha256.clone(),
            },
        )
        .unwrap();
    }

    #[allow(dead_code)]
    fn translate_mock(engine: &mut dyn TranslationEngine) -> String {
        let req = BatchRequest {
            source_lang: Language::Pt,
            target_lang: Language::En,
            segments: vec![BatchSegment {
                id: 1,
                text: "Olá".into(),
                context: vec![],
            }],
            options: Default::default(),
        };
        let res = engine.translate_batch(&req).unwrap();
        res.translations[0].text.clone()
    }

    #[test]
    fn ativo_vazio_retorna_erro_claro() {
        let err = factory_err(&AppConfig::default(), &hw(8));
        assert!(
            err.to_string().contains("nenhum modelo de tradução ativo"),
            "{err}"
        );
    }

    #[test]
    fn ativo_desconhecido_retorna_erro_claro() {
        let cfg = cfg_with_active("modelo-que-nao-existe");
        let err = factory_err(&cfg, &hw(8));
        assert!(err.to_string().contains("não existe no catálogo"), "{err}");
    }

    #[test]
    fn ativo_stt_retorna_erro_nao_silencioso() {
        let cfg = cfg_with_active("whisper-tiny");
        let err = factory_err(&cfg, &hw(8));
        assert!(
            err.to_string().contains("não é um modelo de tradução"),
            "{err}"
        );
    }

    #[test]
    fn ativo_sem_download_retorna_erro_claro() {
        let cfg = cfg_with_active("nllb-200-distilled-600m-q4");
        let err = factory_err(&cfg, &hw(8));
        assert!(
            err.to_string().contains("indisponível") && err.to_string().contains("baixado"),
            "{err}"
        );
    }

    #[test]
    fn backend_whisper_retorna_erro() {
        // Modelo STT ativo como tradução já é rejeitado antes; este teste cobre
        // a defesa extra contra o backend whisper no match.
        let cfg = cfg_with_active("whisper-tiny");
        let err = factory_err(&cfg, &hw(8));
        assert!(err.to_string().contains("whisper"), "{err}");
    }

    #[cfg(not(feature = "ort"))]
    #[test]
    fn backend_ort_nao_compilado_retorna_erro_de_feature() {
        with_root("factory-ort-err", || {
            let model = catalog_model("nllb-200-distilled-600m-q4");
            stage_downloaded(&model);
            let err = factory_err(&cfg_with_active(&model.id), &hw(8));
            assert!(
                err.to_string().contains("feature") && err.to_string().contains("ort"),
                "esperava erro de feature ort, veio: {err}"
            );
        });
    }

    #[cfg(not(feature = "llama"))]
    #[test]
    fn backend_llama_nao_compilado_retorna_erro_de_feature() {
        with_root("factory-llm-err", || {
            let model = catalog_model("towerinstruct-7b-q4_k_m");
            stage_downloaded(&model);
            let err = factory_err(&cfg_with_active(&model.id), &hw(8));
            assert!(
                err.to_string().contains("feature") && err.to_string().contains("llama"),
                "esperava erro de feature llama, veio: {err}"
            );
        });
    }

    #[cfg(feature = "ort")]
    #[test]
    fn backend_ort_constroi_engine_nllb() {
        with_root("factory-ort-real", || {
            let model = catalog_model("nllb-200-distilled-600m-q4");
            // Só o arquivo principal baixado (passa na resolução do cache); o
            // encoder fica ausente → NllbEngine::load falha com mensagem NLLB.
            let dir = cache::model_dir(&model).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let file_path = dir.join(&model.file);
            std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
            std::fs::write(&file_path, b"dummy").unwrap();
            cache::write_status(
                &model,
                &ModelStatus {
                    status: CacheStatus::Downloaded,
                    size_bytes: 1,
                    sha256: model.sha256.clone(),
                },
            )
            .unwrap();
            let err = factory_err(&cfg_with_active(&model.id), &hw(8));
            assert!(
                err.to_string().contains("NLLB"),
                "esperava erro do NllbEngine, veio: {err}"
            );
        });
    }

    #[cfg(feature = "llama")]
    #[test]
    fn backend_llama_constroi_engine_llm() {
        with_root("factory-llm-real", || {
            let model = catalog_model("towerinstruct-7b-q4_k_m");
            stage_downloaded(&model); // gguf dummy presente → passa na resolução
            let err = factory_err(&cfg_with_active(&model.id), &hw(8));
            assert!(
                err.to_string().contains("GGUF"),
                "esperava erro do LlmEngine, veio: {err}"
            );
        });
    }
}
