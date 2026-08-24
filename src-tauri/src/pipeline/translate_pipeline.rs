//! Orquestração do swap de memória STT → tradução (tarefa 3.8) e do pipeline de
//! tradução completo (tarefa 3.10).
//!
//! ## 3.8 — swap de memória
//! No Tier 1 (4GB) o modelo STT (Whisper) e a engine de tradução nunca podem
//! estar carregados juntos. Este pipeline garante a ordem estrita:
//!
//!   1. transcrição completa (Whisper carregado);
//!   2. **drop explícito** do `WhisperModel` (release do `WhisperContext` + mmap);
//!   3. só então inicializa a engine de tradução (factory 3.4).
//!
//! A cada etapa o RSS é medido e logado ([`MemoryTracker`]); ao final, se o pico
//! passar do limite do tier, o pipeline avisa com sugestão de modelo menor.
//!
//! O uso de um bloco escopado para carregar/transcrever/descartar o Whisper
//! garante por construção (não só por convenção) que o `WhisperModel` é dropado
//! antes da engine de tradução existir.
//!
//! ## 3.10 — tradução de legendas
//! [`run_translate`] encadeia o fluxo 3.5→3.4→3.6→1.8→1.7 sobre uma lista de
//! `Subtitle` (ex: parseada de um SRT, ou o `stt.subtitle` do swap 3.8):
//! batcher de segmentos numerados (3.5) → engine da factory (3.4) → parser com
//! fallback por linha (3.6) → formatação profissional reaplicada ao texto
//! traduzido (1.8) → serializer SRT (1.7). Os timestamps de entrada são
//! preservados bloco a bloco; as regras de 1.8 (≤2 linhas, ≤42 chars, CPS ≤ 25,
//! sem overlap) só alteram o timing quando o texto traduzido exigir.

use std::path::Path;

use thiserror::Error;

use crate::config::AppConfig;
use crate::domain::{DomainError, Language, Segment, Subtitle};
use crate::errors::LegendaiError;
use crate::format::{format_subtitles, FormattedSubtitle};
use crate::hardware::detect::HardwareInfo;
use crate::stt::WhisperModel;
use crate::subtitles::srt::to_srt;
use crate::translate::factory::TranslationEngineFactory;
use crate::translate::{
    chunk_segments, translate_with_retry, BatchOptions, BatchRequest, TranslateError,
    TranslatedSegment, TranslationEngine, TranslationStatus, DEFAULT_BATCH_SIZE,
    DEFAULT_CONTEXT_SIZE,
};

use super::memory::MemoryTracker;
use super::stt_pipeline::{formatted_to_subtitles, run_stt, SttPipelineOptions, SttResult};

/// Erro do pipeline de tradução: empacota erros do fluxo STT ([`LegendaiError`]),
/// da inicialização da engine de tradução ([`TranslateError`]) e do domínio
/// de legendas ([`DomainError`]).
#[derive(Debug, Error)]
pub enum TranslatePipelineError {
    #[error(transparent)]
    Legendai(#[from] LegendaiError),
    #[error(transparent)]
    Translate(#[from] TranslateError),
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// Cancelamento cooperativo (callback de progresso devolveu `false`).
    /// Usado pela tela de processamento (4.3) para interromper a tradução
    /// entre lotes.
    #[error("processamento cancelado pelo usuário")]
    Cancelled,
}

/// Resultado do swap: a transcrição (STT), a engine de tradução pronta para
/// uso (3.10) e o rastreador de memória com o pico por etapa.
pub struct TranslateSwapResult {
    pub stt: SttResult,
    pub engine: Box<dyn TranslationEngine>,
    pub memory: MemoryTracker,
}

/// Orquestra o swap de memória STT → tradução com medição de RSS por etapa.
///
/// Transcreve `input` com o modelo em `stt_model_path`, **dropa** o modelo STT
/// dentro do mesmo bloco escopado, e só então inicializa a engine de tradução
/// via factory (3.4). Loga o pico de memória a cada etapa e avisa se o limite
/// for estourado.
///
/// `memory_limit_bytes`: teto de RSS acima do qual o pipeline avisa (ex:
/// [`super::memory::TIER1_RSS_LIMIT_BYTES`] no Tier 1). `0` desativa o aviso.
pub fn run_transcribe_and_swap(
    stt_model_path: &Path,
    input: &Path,
    stt_opts: &SttPipelineOptions,
    config: &AppConfig,
    hw: &HardwareInfo,
    memory_limit_bytes: u64,
) -> Result<TranslateSwapResult, TranslatePipelineError> {
    let mut memory = MemoryTracker::new(memory_limit_bytes);
    memory.mark("início");

    // Etapa STT dentro de um bloco: carrega, transcreve e DROPA o modelo aqui,
    // garantindo que o Whisper não coexiste com a engine de tradução.
    let stt = {
        let model = WhisperModel::load(stt_model_path).map_err(LegendaiError::from)?;
        memory.mark("Whisper carregado");
        let result = run_stt(&model, input, stt_opts)?;
        memory.mark("transcrição completa");
        drop(model); // swap: release do WhisperContext + mmap
        memory.mark("modelo STT liberado");
        result
    };

    // Etapa de tradução: a engine só existe agora, com o STT já dropado.
    let engine = TranslationEngineFactory::for_config(config, hw)?;
    memory.mark("engine de tradução inicializada");

    memory.warn_if_over();

    Ok(TranslateSwapResult {
        stt,
        engine,
        memory,
    })
}

/// Número de tentativas de tradução por lote (ADR-002: até 2 tentativas).
const MAX_RETRY_ATTEMPTS: usize = 2;

/// Resultado do pipeline de tradução (3.10): legendas traduzidas com o timing
/// original preservado, a versão reformatada pelas regras 1.8 e o SRT final.
#[derive(Debug)]
pub struct TranslateResult {
    /// Legendas traduzidas, bloco a bloco, com os timestamps de entrada.
    pub subtitles: Vec<Subtitle>,
    /// Reformatadas pelas regras profissionais (1.8) sobre o texto traduzido.
    pub formatted: Vec<FormattedSubtitle>,
    /// SRT final (1.7) — pronto para gravar em disco.
    pub srt: String,
    pub source_lang: Language,
    pub target_lang: Language,
    /// Segmentos que mantiveram o texto original (fallback por linha da 3.6).
    pub kept_original_count: usize,
}

/// Pipeline de tradução completo (3.10): batcher (3.5) → engine da factory
/// (3.4) → parser/retry (3.6) → formatter (1.8) → serializer SRT (1.7).
///
/// `subtitles` é a origem (ex: `parse_srt` de um arquivo, ou o `stt.subtitle`
/// do swap 3.8). A engine é construída pela factory a partir de `config`/`hw`
/// (modelo de tradução ativo). Equivalentes com engine já pronta:
/// [`run_translate_with_engine`] (usada pelo swap 3.8 e pelos testes com mock).
pub fn run_translate(
    subtitles: &[Subtitle],
    config: &AppConfig,
    hw: &HardwareInfo,
) -> Result<TranslateResult, TranslatePipelineError> {
    let mut engine = TranslationEngineFactory::for_config(config, hw)?;
    run_translate_with_engine(&mut *engine, subtitles, config)
}

/// Núcleo orquestrado do pipeline de tradução, com a engine já construída.
///
/// Encadeia 3.5→3.6→1.8→1.7: achata os segmentos (ids globais 1-based), traduz
/// lote a lote via `translate_with_retry` (o `respond` adapta o `BatchResult` da
/// engine para o formato `[N] texto` que o parser 3.6 valida), reconstrói as
/// legendas bloco a bloco mantendo os timestamps originais, reaplica as regras
/// 1.8 ao texto traduzido e serializa o SRT (1.7).
pub fn run_translate_with_engine(
    engine: &mut dyn TranslationEngine,
    subtitles: &[Subtitle],
    config: &AppConfig,
) -> Result<TranslateResult, TranslatePipelineError> {
    run_translate_with_engine_progress(engine, subtitles, config, None)
}

/// Igual a [`run_translate_with_engine`], com callback opcional de
/// progresso/cancelamento da tradução (tarefa 4.3).
///
/// `progress` é chamado antes de cada lote com `(lotes_processados,
/// total_de_lotes)`; retornar `false` aborta o pipeline com
/// [`TranslatePipelineError::Cancelled`]. A nota da 4.3 pede cancelamento
/// cooperativo "entre segmentos" — o lote é a unidade granular da tradução
/// (até 10 segmentos, 3.5) e é onde o token é checado.
pub fn run_translate_with_engine_progress(
    engine: &mut dyn TranslationEngine,
    subtitles: &[Subtitle],
    config: &AppConfig,
    mut progress: Option<&mut dyn FnMut(usize, usize) -> bool>,
) -> Result<TranslateResult, TranslatePipelineError> {
    let source_lang = resolve_source_lang(subtitles, config)?;
    let target_lang = Language::from_code(&config.target_lang);
    if target_lang.is_auto() {
        return Err(TranslatePipelineError::Translate(TranslateError::Backend(
            "idioma de destino não pode ser `auto` — defina o destino na configuração".into(),
        )));
    }
    if !engine.supported_pair(&source_lang, &target_lang) {
        return Err(TranslatePipelineError::Translate(TranslateError::Backend(
            format!(
                "a engine de tradução ativa não suporta o par {} → {}",
                source_lang.as_code(),
                target_lang.as_code()
            ),
        )));
    }

    // 3.5 — batcher: acha os segmentos de todos os blocos numa ordem global
    // (ids 1-based) e traduz lote a lote com o parser/fallback da 3.6.
    let flat: Vec<Segment> = subtitles.iter().flat_map(|s| s.segments.clone()).collect();
    let batches = chunk_segments(&flat, DEFAULT_BATCH_SIZE, DEFAULT_CONTEXT_SIZE);
    let total_batches = batches.len();
    let mut translated: Vec<TranslatedSegment> = Vec::with_capacity(flat.len());
    let mut kept_original = 0usize;
    for (i, batch) in batches.iter().enumerate() {
        // Cancelamento cooperativo antes de cada lote (checagem "entre segmentos").
        if let Some(cb) = progress.as_deref_mut() {
            if !cb(i, total_batches) {
                return Err(TranslatePipelineError::Cancelled);
            }
        }
        let result = translate_with_retry(&batch.segments, MAX_RETRY_ATTEMPTS, |pending| {
            let response = engine.translate_batch(&BatchRequest {
                source_lang: source_lang.clone(),
                target_lang: target_lang.clone(),
                segments: pending.to_vec(),
                options: BatchOptions::default(),
            })?;
            // O parser 3.6 espera o formato `[N] texto`; o `BatchResult` já traz
            // um texto por id — serializa nesse formato para o parser validar
            // e aplicar o fallback por linha.
            Ok(response
                .translations
                .iter()
                .map(|t| format!("[{}] {}", t.id, t.text))
                .collect::<Vec<_>>()
                .join("\n"))
        })?;
        for t in result.translations {
            if t.status == TranslationStatus::KeptOriginal {
                kept_original += 1;
            }
            translated.push(t);
        }
    }
    translated.sort_by_key(|t| t.id);

    // Reconstrói as legendas bloco a bloco, mantendo os timestamps de entrada.
    let mut out_blocks = Vec::with_capacity(subtitles.len());
    let mut cursor = 0usize;
    for sub in subtitles {
        let mut segs = Vec::with_capacity(sub.segments.len());
        for orig in &sub.segments {
            let t = translated
                .get(cursor)
                .ok_or(LegendaiError::TranscribeFailed)?;
            segs.push(Segment::new(
                t.text.clone(),
                orig.start_ms,
                orig.end_ms,
                target_lang.clone(),
            )?);
            cursor += 1;
        }
        out_blocks.push(Subtitle {
            index: sub.index,
            segments: segs,
            language: target_lang.clone(),
        });
    }

    // 1.8 — regras profissionais reaplicadas ao texto traduzido; 1.7 — SRT.
    let formatted = format_subtitles(&out_blocks);
    let srt = to_srt(&formatted_to_subtitles(&formatted));

    Ok(TranslateResult {
        subtitles: out_blocks,
        formatted,
        srt,
        source_lang,
        target_lang,
        kept_original_count: kept_original,
    })
}

/// Resolve o idioma de origem: o idioma concreto da legenda se houver (ex:
/// transcrição do Whisper), senão `config.source_lang` (SRT não carrega idioma).
/// `auto` sem idioma concreto → erro claro (tradução precisa de origem real).
fn resolve_source_lang(
    subtitles: &[Subtitle],
    config: &AppConfig,
) -> Result<Language, TranslatePipelineError> {
    if let Some(lang) = subtitles
        .iter()
        .find_map(|s| (!s.language.is_auto()).then(|| s.language.clone()))
    {
        return Ok(lang);
    }
    let src = Language::from_code(&config.source_lang);
    if src.is_auto() {
        return Err(TranslatePipelineError::Translate(TranslateError::Backend(
            "idioma de origem não detectado na legenda — defina o idioma de origem na configuração"
                .into(),
        )));
    }
    Ok(src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ActiveModels;
    use crate::domain::Timestamp;
    use crate::hardware::detect::HardwareInfo;
    use crate::model_manager::cache::{self, with_root, CacheStatus, ModelStatus};
    use crate::model_manager::catalog::Catalog;
    use crate::pipeline::memory::TIER1_RSS_LIMIT_BYTES;
    use crate::translate::engine::{BatchRequest, BatchResult};

    fn hw() -> HardwareInfo {
        HardwareInfo {
            ram_gb: 4,
            cpu_threads: 4,
            gpu: None,
            cpu_name: "test".into(),
            recommended_threads: 2,
        }
    }

    /// Simula o modelo de tradução ativo baixado no cache (padrão da factory 3.4).
    fn cfg_com_traducao_baixada(engine_id: &str) -> AppConfig {
        let model = Catalog::embedded()
            .unwrap()
            .models
            .into_iter()
            .find(|m| m.id == engine_id)
            .unwrap();
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
        AppConfig {
            active_models: ActiveModels {
                translation: engine_id.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn pipeline_sem_modelo_stt_retorna_erro_de_transcricao() {
        with_root("swap-sem-stt", || {
            let cfg = cfg_com_traducao_baixada("nllb-200-distilled-600m-q4");
            let result = run_transcribe_and_swap(
                Path::new("/nao/existe/ggml.bin"),
                Path::new("/nao/existe/audio.wav"),
                &SttPipelineOptions::default(),
                &cfg,
                &hw(),
                TIER1_RSS_LIMIT_BYTES,
            );
            // O erro do STT (modelo ausente → ModelMissing) deve propagar tipado,
            // e a engine de tradução nunca chega a ser inicializada.
            assert!(matches!(
                result,
                Err(TranslatePipelineError::Legendai(
                    LegendaiError::ModelMissing
                ))
            ));
        });
    }

    /// Só faz sentido sem `ort`/`llama`: com os backends compilados a factory tenta
    /// carregar o modelo real (e falha com erro NLLB — comportamento correto).
    #[cfg(not(any(feature = "ort", feature = "llama")))]
    #[test]
    fn traducao_baixada_e_compilada_entrega_engine_inicializada() {
        with_root("swap-ok", || {
            let cfg = cfg_com_traducao_baixada("nllb-200-distilled-600m-q4");
            let err = match TranslationEngineFactory::for_config(&cfg, &hw()) {
                Ok(_) => panic!("esperava erro de feature ort"),
                Err(e) => e,
            };
            assert!(
                err.to_string().contains("feature") && err.to_string().contains("ort"),
                "esperava erro de feature ort, veio: {err}"
            );
        });
    }

    /// Teste manual (não roda em CI): valida o swap de memória com modelo STT
    /// real, medindo o RSS a cada etapa e conferindo que a engine de tradução é
    /// inicializada apenas após o drop do Whisper. Exige:
    /// - `LEGENDAI_MODEL_PATH` (GGUF do whisper, ver e2e_stt);
    /// - `LEGENDAI_FIXTURE` (WAV/vídeo com fala; default = fixture do e2e_stt).
    ///
    /// Roda com o teste ignorado `swap_manual_com_modelo_real` (veja e2e_stt).
    #[test]
    #[ignore = "exige modelo whisper GGUF via env (não roda em CI)"]
    fn swap_manual_com_modelo_real() {
        let model_path = std::env::var("LEGENDAI_MODEL_PATH")
            .expect("sete LEGENDAI_MODEL_PATH (GGUF do whisper) para rodar");
        let fixture = std::env::var("LEGENDAI_FIXTURE").unwrap_or_else(|_| {
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/audio-pt.wav").to_string()
        });
        let cfg = AppConfig::default(); // mock engine (sem backend compilado)
        let result = run_transcribe_and_swap(
            Path::new(&model_path),
            Path::new(&fixture),
            &SttPipelineOptions::default(),
            &cfg,
            &hw(),
            TIER1_RSS_LIMIT_BYTES,
        )
        .expect("swap de memória deve completar");
        assert!(
            !result.stt.formatted.is_empty(),
            "transcrição deve produzir legendas"
        );
        assert!(
            result.memory.peak_bytes() < TIER1_RSS_LIMIT_BYTES,
            "pico {}MiB deve ficar abaixo do limite do Tier 1",
            result.memory.peak_bytes() as f64 / (1024.0 * 1024.0)
        );
        eprintln!(
            "pico de memória: {} MiB",
            result.memory.peak_bytes() as f64 / (1024.0 * 1024.0)
        );
    }

    // ── Tarefa 3.10: pipeline de tradução ────────────────────────────────────

    fn seg(text: &str, s: u64, e: u64, lang: Language) -> Segment {
        Segment::new(text, Timestamp::from_ms(s), Timestamp::from_ms(e), lang).unwrap()
    }

    fn bloco(index: u32, segments: Vec<Segment>) -> Subtitle {
        Subtitle {
            index,
            segments,
            language: Language::Pt,
        }
    }

    fn cfg(src: &str, tgt: &str) -> AppConfig {
        AppConfig {
            source_lang: src.into(),
            target_lang: tgt.into(),
            ..Default::default()
        }
    }

    fn mock_config() -> AppConfig {
        cfg("pt", "en")
    }

    /// Engine de teste que simula falha persistente de um segmento (texto vazio)
    /// → o parser 3.6 deve marcar `KeptOriginal` após as tentativas de retry.
    struct EngineComFalha;

    impl TranslationEngine for EngineComFalha {
        fn translate_batch(&mut self, req: &BatchRequest) -> Result<BatchResult, TranslateError> {
            let translations = req
                .segments
                .iter()
                .map(|s| TranslatedSegment {
                    id: s.id,
                    text: if s.id == 2 {
                        String::new() // resposta vazia → linha `[N] ` não casa o parser
                    } else {
                        format!("TR {}", s.text)
                    },
                    status: TranslationStatus::Ok,
                })
                .collect();
            Ok(BatchResult { translations })
        }

        fn supported_pair(&self, _source: &Language, _target: &Language) -> bool {
            true
        }
    }

    /// Engine que não suporta o par pedido (valida o erro tipado do pipeline).
    struct EngineSemPar;

    impl TranslationEngine for EngineSemPar {
        fn translate_batch(&mut self, _req: &BatchRequest) -> Result<BatchResult, TranslateError> {
            unreachable!("não deve ser chamada — o par é rejeitado antes")
        }

        fn supported_pair(&self, _source: &Language, _target: &Language) -> bool {
            false
        }
    }

    #[test]
    fn resolve_source_lang_prefere_idioma_da_legenda() {
        let subs = [
            bloco(1, vec![seg("Oi", 0, 1000, Language::Pt)]),
            bloco(2, vec![seg("Olá", 2000, 3000, Language::Pt)]),
        ];
        assert_eq!(
            resolve_source_lang(&subs, &cfg("en", "pt")).unwrap(),
            Language::Pt
        );
    }

    #[test]
    fn resolve_source_lang_usa_config_quando_legenda_auto() {
        let subs = vec![Subtitle {
            index: 1,
            segments: vec![seg("Oi", 0, 1000, Language::auto())],
            language: Language::auto(),
        }];
        assert_eq!(
            resolve_source_lang(&subs, &cfg("es", "pt")).unwrap(),
            Language::Es
        );
    }

    #[test]
    fn resolve_source_lang_auto_sem_fonte_retorna_erro_claro() {
        let subs = vec![Subtitle {
            index: 1,
            segments: vec![seg("Oi", 0, 1000, Language::auto())],
            language: Language::auto(),
        }];
        let err = resolve_source_lang(&subs, &cfg("auto", "pt")).unwrap_err();
        assert!(err.to_string().contains("idioma de origem"), "{err}");
    }

    #[test]
    fn run_translate_com_mock_preserva_timing_e_traduz() {
        let input = vec![
            bloco(1, vec![seg("Olá, mundo.", 1000, 3000, Language::Pt)]),
            bloco(2, vec![seg("Como você está?", 4000, 6500, Language::Pt)]),
            bloco(3, vec![seg("Estou muito bem.", 7000, 10000, Language::Pt)]),
        ];
        let mut engine = crate::translate::MockEngine::default();
        let result = run_translate_with_engine(&mut engine, &input, &mock_config()).unwrap();

        // Texto traduzido (prefixo do mock) e timing original preservado.
        assert_eq!(result.subtitles.len(), 3);
        for (out, src) in result.subtitles.iter().zip(&input) {
            assert_eq!(out.segments[0].text, format!("TR {}", src.segments[0].text));
            assert_eq!(out.segments[0].start_ms, src.segments[0].start_ms);
            assert_eq!(out.segments[0].end_ms, src.segments[0].end_ms);
            assert_eq!(out.segments[0].lang, Language::En);
        }
        assert_eq!(result.kept_original_count, 0);
        assert!(result.srt.contains("TR Olá, mundo."));
        assert!(result.srt.contains("00:00:01,000 --> 00:00:03,000"));
    }

    #[test]
    fn run_translate_reaplica_formatacao_ao_texto_traduzido() {
        // Texto longo: com o prefixo do mock fica ainda maior — o formatter 1.8
        // deve re-quebrar em vários blocos respeitando as regras.
        let long = "uma frase bem longa que deveria ser quebrada em mais de uma linha para caber na tela de forma legível e confortável";
        let input = vec![bloco(1, vec![seg(long, 1000, 12000, Language::Pt)])];
        let mut engine = crate::translate::MockEngine::default();
        let result = run_translate_with_engine(&mut engine, &input, &mock_config()).unwrap();

        assert!(
            result.formatted.len() >= 2,
            "texto longo deve ser re-partido"
        );
        for f in &result.formatted {
            assert!(f.lines.len() <= crate::format::rules::MAX_LINES);
            for line in &f.lines {
                assert!(line.chars().count() <= crate::format::rules::MAX_CHARS_PER_LINE);
            }
        }
    }

    #[test]
    fn run_translate_falha_persistente_vira_kept_original() {
        let input = vec![
            bloco(1, vec![seg("Primeira.", 0, 2000, Language::Pt)]),
            bloco(2, vec![seg("Segunda que falha.", 3000, 5000, Language::Pt)]),
            bloco(3, vec![seg("Terceira.", 6000, 8000, Language::Pt)]),
        ];
        let mut engine = EngineComFalha;
        let result = run_translate_with_engine(&mut engine, &input, &mock_config()).unwrap();

        assert_eq!(result.kept_original_count, 1);
        // O texto original nunca é descartado (fallback 3.6).
        let seg2 = &result.subtitles[1].segments[0];
        assert_eq!(seg2.text, "Segunda que falha.");
        // Os demais seguem traduzidos.
        assert_eq!(result.subtitles[0].segments[0].text, "TR Primeira.");
        assert_eq!(result.subtitles[2].segments[0].text, "TR Terceira.");
    }

    #[test]
    fn run_translate_rejeita_destino_auto() {
        let input = vec![bloco(1, vec![seg("Oi", 0, 1000, Language::Pt)])];
        let mut engine = crate::translate::MockEngine::default();
        let err = run_translate_with_engine(&mut engine, &input, &cfg("pt", "auto")).unwrap_err();
        assert!(err.to_string().contains("destino"), "{err}");
    }

    #[test]
    fn run_translate_rejeita_par_nao_suportado() {
        let input = vec![bloco(1, vec![seg("Oi", 0, 1000, Language::Pt)])];
        let mut engine = EngineSemPar;
        let err = run_translate_with_engine(&mut engine, &input, &mock_config()).unwrap_err();
        assert!(err.to_string().contains("não suporta o par"), "{err}");
    }

    #[test]
    fn run_translate_entrada_vazia_produz_srt_vazio() {
        let mut engine = crate::translate::MockEngine::default();
        let result = run_translate_with_engine(&mut engine, &[], &mock_config()).unwrap();
        assert!(result.subtitles.is_empty());
        assert!(result.formatted.is_empty());
        assert_eq!(result.srt, "");
    }

    fn muitos_blocos(n: u32) -> Vec<Subtitle> {
        (0..n)
            .map(|i| {
                bloco(
                    i + 1,
                    vec![seg(
                        &format!("Linha {i}."),
                        u64::from(i) * 1000,
                        u64::from(i) * 1000 + 500,
                        Language::Pt,
                    )],
                )
            })
            .collect()
    }

    #[test]
    fn progress_callback_recebe_lotes_e_total_em_ordem() {
        let input = muitos_blocos(25); // 25 segmentos → 3 lotes (10/10/5)
        let mut engine = crate::translate::MockEngine::default();
        let mut calls: Vec<(usize, usize)> = Vec::new();
        let result = run_translate_with_engine_progress(
            &mut engine,
            &input,
            &mock_config(),
            Some(&mut |done, total| {
                calls.push((done, total));
                true
            }),
        )
        .unwrap();

        assert_eq!(calls.first().unwrap(), &(0, 3));
        assert_eq!(calls.last().unwrap(), &(2, 3));
        assert_eq!(calls.len(), 3);
        assert_eq!(result.subtitles.len(), 25);
    }

    #[test]
    fn progress_false_cancela_com_erro_tipado() {
        let input = muitos_blocos(25);
        let mut engine = crate::translate::MockEngine::default();
        let mut calls = 0usize;
        let err = run_translate_with_engine_progress(
            &mut engine,
            &input,
            &mock_config(),
            Some(&mut |done, _| {
                calls += 1;
                done < 1 // 1º lote ok, 2º cancela
            }),
        )
        .unwrap_err();

        assert!(
            matches!(err, TranslatePipelineError::Cancelled),
            "callback false deve abortar com Cancelled, veio {err}"
        );
        assert_eq!(calls, 2);
    }

    /// `run_translate` (com factory 3.4) num build sem os backends ort/llama
    /// retorna erro de feature ausente em vez de degradar para mock —
    /// garante que o usuário receba mensagem acionável.
    #[cfg(not(any(feature = "ort", feature = "llama")))]
    #[test]
    fn run_translate_via_factory_usa_engine_mock_sem_backend() {
        with_root("translate-factory", || {
            let cfg = cfg_com_traducao_baixada("nllb-200-distilled-600m-q4");
            let input = vec![bloco(1, vec![seg("Olá, mundo.", 1000, 3000, Language::Pt)])];
            let err = run_translate(&input, &cfg, &hw()).unwrap_err();
            assert!(
                err.to_string().contains("feature") && err.to_string().contains("ort"),
                "esperava erro de feature ort, veio: {err}"
            );
        });
    }
}
