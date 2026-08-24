//! Comandos IPC do pipeline de tradução (tarefas 3.10 e 4.3).
//!
//! 3.10: `translate_subtitle` traduz um SRT pronto (lê → engine → formata →
//! grava), usado para re-traduzir legendas existentes.
//!
//! 4.3: `run_pipeline` executa o fluxo completo a partir de um vídeo
//! (extrair → transcrever → traduzir → formatar → exportar) em background,
//! emitindo `pipeline-progress` (etapa + pct) e `pipeline-finished`
//! (resumo/erro/cancelado). `cancel_pipeline` interrompe o job de forma
//! cooperativa: o token é checado na transcrição (abort callback do whisper) e
//! entre lotes de tradução (callback de progresso da 3.10). O job roda em
//! `spawn_blocking` — o trabalho pesado (Whisper/LLM) nunca bloqueia a thread
//! principal do Tauri.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::config::AppConfig;
use crate::domain::Subtitle;
use crate::errors::ErrorDetail;
use crate::hardware::detect::detect;
use crate::model_manager::{cache, catalog};
use crate::pipeline::steps::{PipelineFinished, PipelineProgress, PipelineStep, PipelineSummary};
use crate::pipeline::stt_pipeline::{clamp_to_audio, formatted_to_subtitles};
use crate::pipeline::{load_embedded_subtitle, run_translate, run_translate_with_engine_progress};
#[cfg(feature = "parakeet")]
use crate::stt::parakeet::ParakeetModel;
use crate::stt::{SttOptions, WhisperModel};
use crate::subtitles::srt::{parse_srt, to_srt};
use crate::translate::TranslationEngineFactory;

/// Origem do pipeline: trilha de áudio a transcrever ou legenda embutida
/// (pula o STT). Serde `tag = "type"` — o frontend envia
/// `{ type: "audio", track_index }` ou `{ type: "embedded", stream_index }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineSource {
    /// Transcrever a trilha de áudio (índice GLOBAL do ffprobe).
    Audio { track_index: usize },
    /// Usar a legenda embutida (índice GLOBAL da stream).
    Embedded { stream_index: u32 },
}

/// Opções do pipeline enviadas pela tela de importação.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PipelineOptions {
    /// Traduzir após transcrever (default: sim — campo omitido vira `true`).
    #[serde(default = "default_true")]
    pub translate: bool,
    /// Caminho de saída do SRT. `None`/vazio → derivado do vídeo (`<stem>.srt`).
    pub out_path: Option<String>,
    /// Idioma de destino para tradução (override da config). `None`/vazio/`auto` mantém a config.
    pub target_lang: Option<String>,
    /// Idioma de origem do áudio (override da config/detecção). `None`/vazio/`auto` = auto-detect via Whisper.
    pub source_lang: Option<String>,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            translate: true,
            out_path: None,
            target_lang: None,
            source_lang: None,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Tokens de cancelamento dos jobs em andamento (um por `job_id`). A tarefa
/// remove a entrada ao encerrar; `cancel_pipeline` cancela o token cooperativo.
static ACTIVE_JOBS: OnceLock<Mutex<HashMap<String, CancellationToken>>> = OnceLock::new();

fn active_jobs() -> &'static Mutex<HashMap<String, CancellationToken>> {
    ACTIVE_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Inicia o pipeline de processamento do vídeo em background. O comando retorna
/// na hora (UI não bloqueia); o progresso chega via eventos `pipeline-progress` e
/// o término via `pipeline-finished` (filtrados por `job_id`).
#[tauri::command(rename_all = "snake_case")]
pub fn run_pipeline(
    app: tauri::AppHandle,
    job_id: String,
    input_path: String,
    source: PipelineSource,
    options: Option<PipelineOptions>,
) -> Result<(), String> {
    if !Path::new(&input_path).exists() {
        return Err(format!("arquivo não encontrado: `{input_path}`"));
    }
    if active_jobs().lock().unwrap().contains_key(&job_id) {
        return Err(format!("o job `{job_id}` já está em andamento"));
    }
    let token = CancellationToken::new();
    active_jobs()
        .lock()
        .unwrap()
        .insert(job_id.clone(), token.clone());

    let handle = app.clone();
    // `spawn_blocking`: Whisper/LLM são síncronos e pesados — rodar fora da
    // thread principal e do pool async (a callback de progresso emite eventos
    // via `AppHandle`, que é `Send`).
    tauri::async_runtime::spawn_blocking(move || {
        let finished = execute_job(&handle, &job_id, &input_path, &source, options, &token);
        active_jobs().lock().unwrap().remove(&job_id);
        let _ = handle.emit("pipeline-finished", finished);
    });
    Ok(())
}

/// Cancela o job `job_id` (cooperativo: para na próxima checagem do token —
/// abort do whisper ou entre lotes de tradução). O estado final é reportado
/// como `cancelled` no `pipeline-finished` (limpo, não erro).
#[tauri::command(rename_all = "snake_case")]
pub fn cancel_pipeline(job_id: String) -> Result<(), String> {
    let token = active_jobs()
        .lock()
        .unwrap()
        .get(&job_id)
        .cloned()
        .ok_or_else(|| format!("nenhum processamento em andamento para o job `{job_id}`"))?;
    token.cancel();
    Ok(())
}

/// Classifica o resultado do job e monta o payload de `pipeline-finished`.
/// `pub(crate)`: consumido também pelo worker da fila (4.9).
pub(crate) fn execute_job(
    app: &tauri::AppHandle,
    job_id: &str,
    input_path: &str,
    source: &PipelineSource,
    options: Option<PipelineOptions>,
    token: &CancellationToken,
) -> PipelineFinished {
    let opts = options.unwrap_or_default();
    let result = run_job(app, job_id, input_path, source, &opts, token);
    let cancelled = token.is_cancelled();
    match result {
        Ok(summary) => {
            emit_progress(app, job_id, PipelineStep::Done, 100, None);
            PipelineFinished {
                job_id: job_id.into(),
                ok: true,
                cancelled: false,
                error: None,
                summary: Some(summary),
            }
        }
        Err(e) => PipelineFinished {
            job_id: job_id.into(),
            ok: false,
            cancelled,
            error: if cancelled { None } else { Some(e) },
            summary: None,
        },
    }
}

/// Executa o fluxo extrair → transcrever → traduzir → formatar → exportar,
/// emitindo progresso por etapa. Retorna o resumo para a UI.
fn run_job(
    app: &tauri::AppHandle,
    job_id: &str,
    input_path: &str,
    source: &PipelineSource,
    opts: &PipelineOptions,
    token: &CancellationToken,
) -> Result<PipelineSummary, ErrorDetail> {
    // Mede o tempo total do job para as estatísticas (5.5).
    let mut config = AppConfig::load_or_default();
    // Override de idioma de destino vindo da UI (seletor na importação)
    let target_override = opts
        .target_lang
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "auto")
        .map(String::from);
    if let Some(ref t) = target_override {
        config.target_lang = t.clone();
        // Persiste o último par escolhido (best-effort, não falha o job)
        let _ = config.save();
    }
    let hw = detect();
    let start = std::time::Instant::now();
    let input = Path::new(input_path);
    let out_path = resolve_out_path(input, opts.out_path.as_deref()).map_err(job_error)?;
    let mut duration_secs = crate::audio::ffprobe::probe_duration(input)
        .ok()
        .flatten()
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let mut source_lang = crate::domain::Language::from_code(&config.source_lang);

    emit_log(
        app,
        job_id,
        &format!("iniciando job {job_id} — {}", input.display()),
    );
    // Etapa 1 — extração (áudio ou legenda embutida).
    ensure_not_cancelled(token)?;
    emit_progress(app, job_id, PipelineStep::Extract, 0, None);
    emit_log(app, job_id, "etapa: extrair áudio/legenda");
    let temp_dir = std::env::temp_dir().join(format!("legendai-job-{job_id}"));
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| job_error(format!("falha ao criar diretório temporário: {e}")))?;
    let _guard = TempCleanup(temp_dir.clone());

    let subtitles: Vec<Subtitle>;
    match source {
        PipelineSource::Embedded { stream_index } => {
            subtitles = load_embedded_subtitle(input, *stream_index)
                .map_err(|e| job_error(e.to_string()))?;
            if let Some(l) = subtitles
                .iter()
                .find_map(|s| (!s.language.is_auto()).then(|| s.language.clone()))
            {
                source_lang = l;
            }
            emit_progress(app, job_id, PipelineStep::Extract, 100, None);
            emit_log(
                app,
                job_id,
                &format!("extração concluída — {} blocos", subtitles.len()),
            );
        }
        PipelineSource::Audio { track_index } => {
            let wav = temp_dir.join("audio.wav");
            let (_, audio_duration) =
                crate::audio::ffmpeg_extract::extract_wav(input, *track_index, &wav)
                    .map_err(|e| crate::errors::LegendaiError::from(e).to_detail())?;
            if audio_duration.as_secs_f64() > 0.0 {
                duration_secs = audio_duration.as_secs_f64();
            }
            emit_log(
                app,
                job_id,
                &format!("áudio extraído — duração {:.1}s", duration_secs),
            );
            ensure_not_cancelled(token)?;
            emit_progress(app, job_id, PipelineStep::Extract, 100, None);

            // Etapa 2 — transcrição (Whisper/Parakeet). O modelo é dropado dentro do
            // bloco escopado ANTES da engine de tradução existir (swap ADR-005).
            let stt = resolve_stt_model(&config)?;
            emit_progress(app, job_id, PipelineStep::Transcribe, 0, None);
            // source_lang override vindo da UI (ex: Crunchyroll tag eng mas áudio é ja) — "auto" = auto-detect
            let stt_source = opts
                .source_lang
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "auto")
                .or_else(|| {
                    let c = config.source_lang.trim();
                    if c.is_empty() || c == "auto" {
                        None
                    } else {
                        Some(c)
                    }
                })
                .map(crate::domain::Language::from_code);
            if let Some(ref l) = stt_source {
                emit_log(
                    app,
                    job_id,
                    &format!("idioma fonte forçado: {}", l.as_code()),
                );
            }
            emit_log(
                app,
                job_id,
                &format!("transcrevendo com backend {:?}", stt.backend),
            );
            #[cfg(feature = "parakeet")]
            let use_cuda = hw
                .gpu
                .is_some_and(|g| g == crate::hardware::detect::GpuKind::Cuda);
            let threads = config.threads.unwrap_or(hw.recommended_threads).max(1) as usize;
            let transcription = match stt.backend {
                catalog::Backend::Whisper => {
                    let model = WhisperModel::load(&stt.main_path).map_err(stt_detail)?;
                    let stt_opts = SttOptions { threads, language: stt_source.clone(), ..Default::default() };
                    // A callback do whisper é 'static: mover clones (AppHandle e
                    // token são Clone) em vez de emprestar as referências locais.
                    let app_for_cb = app.clone();
                    let token_for_cb = token.clone();
                    let job_id_for_cb = job_id.to_string();
                    let t = model
                        .transcribe_with_progress(&wav, &stt_opts, move |pct| {
                            emit_progress(&app_for_cb, &job_id_for_cb, PipelineStep::Transcribe, pct.clamp(0, 100) as u8, None);
                            !token_for_cb.is_cancelled()
                        })
                        .map_err(|e| {
                            if token.is_cancelled() {
                                job_error("processamento cancelado".to_string())
                            } else {
                                stt_detail(e)
                            }
                        })?;
                    drop(model);
                    t
                }
                #[cfg(feature = "parakeet")]
                catalog::Backend::Parakeet => {
                    ensure_not_cancelled(token)?;
                    let mut model = ParakeetModel::load(&stt.model_dir, threads, use_cuda)
                        .map_err(|e| crate::errors::LegendaiError::from(e).to_detail())?;
                    let t = model.transcribe(&wav, stt_source.clone()).map_err(|e| {
                        if token.is_cancelled() {
                            job_error("processamento cancelado".to_string())
                        } else {
                            crate::errors::LegendaiError::from(e).to_detail()
                        }
                    })?;
                    drop(model);
                    t
                }
                #[cfg(not(feature = "parakeet"))]
                catalog::Backend::Parakeet => {
                    return Err(job_error(
                        "backend Parakeet não compilado neste build (feature `parakeet` ausente)".into(),
                    ))
                }
                catalog::Backend::Canary | catalog::Backend::Nemotron => {
                    return Err(job_error(format!(
                        "backend `{:?}` de transcrição ainda não implementado (use Whisper ou Parakeet)",
                        stt.backend
                    )))
                }
                catalog::Backend::Llama | catalog::Backend::Ort => {
                    return Err(job_error(format!(
                        "backend `{:?}` não é de transcrição",
                        stt.backend
                    )))
                }
            };
            if transcription.segments.is_empty() {
                return Err(crate::errors::LegendaiError::NoSpeech.to_detail());
            }
            emit_progress(app, job_id, PipelineStep::Transcribe, 100, None);
            emit_log(
                app,
                job_id,
                &format!(
                    "transcrição concluída — {} segmentos, idioma {}",
                    transcription.segments.len(),
                    transcription.language.as_code()
                ),
            );
            source_lang = transcription.language.clone();
            // Um Subtitle por segmento preserva os intervalos de silêncio do
            // Whisper; um único Subtitle com todos os segmentos faria o
            // `format_subtitles` redistribuir o tempo proporcionalmente,
            // eliminando gaps e dessincronizando a legenda.
            subtitles = transcription
                .segments
                .into_iter()
                .enumerate()
                .map(|(i, seg)| Subtitle {
                    index: (i + 1) as u32,
                    segments: vec![seg],
                    language: transcription.language.clone(),
                })
                .collect();
        }
    }

    if opts.translate {
        // Etapa 3 — tradução (engine da factory 3.4), com progresso por lote.
        ensure_not_cancelled(token)?;
        emit_progress(app, job_id, PipelineStep::Translate, 0, None);
        emit_log(app, job_id, "traduzindo…");
        let mut engine = TranslationEngineFactory::for_config(&config, &hw)
            .map_err(|e| crate::errors::LegendaiError::from(e).to_detail())?;
        let result = run_translate_with_engine_progress(
            &mut *engine,
            &subtitles,
            &config,
            Some(&mut |done, total| {
                let pct = if total > 0 {
                    (done as f64 / total as f64 * 100.0) as u8
                } else {
                    100
                };
                let detail = format!("{done}/{total} lotes");
                emit_progress(app, job_id, PipelineStep::Translate, pct, Some(&detail));
                if done % 5 == 0 || done == total {
                    emit_log(app, job_id, &format!("tradução {detail} — {pct}%"));
                }
                !token.is_cancelled()
            }),
        )
        .map_err(|e| match e {
            crate::pipeline::translate_pipeline::TranslatePipelineError::Legendai(le) => {
                le.to_detail()
            }
            crate::pipeline::translate_pipeline::TranslatePipelineError::Translate(te) => {
                crate::errors::LegendaiError::from(te).to_detail()
            }
            crate::pipeline::translate_pipeline::TranslatePipelineError::Domain(de) => {
                job_error(de.to_string())
            }
            crate::pipeline::translate_pipeline::TranslatePipelineError::Cancelled => {
                job_error("processamento cancelado".into())
            }
        })?;
        emit_progress(app, job_id, PipelineStep::Translate, 100, None);
        emit_log(app, job_id, "tradução concluída");

        // Etapa 4 — formatação (1.8), reaplicada ao texto traduzido dentro do
        // pipeline 3.10; aqui só sinalizamos a transição (etapa quase instantânea).
        emit_progress(app, job_id, PipelineStep::Format, 0, None);
        emit_progress(app, job_id, PipelineStep::Format, 100, None);
        emit_log(app, job_id, "formatação concluída");

        // Etapa 5 — exportação do SRT (traduzido + o original ao lado, para o
        // preview duplo da 4.5).
        ensure_not_cancelled(token)?;
        emit_progress(app, job_id, PipelineStep::Export, 0, None);
        std::fs::write(&out_path, &result.srt)
            .map_err(|e| job_error(format!("falha ao gravar `{}`: {e}", out_path.display())))?;
        let original_out = original_sidecar_path(&out_path);
        let original_srt = {
            let mut formatted = crate::format::format_subtitles(&subtitles);
            if duration_secs > 0.0 {
                clamp_to_audio(
                    &mut formatted,
                    std::time::Duration::from_secs_f64(duration_secs),
                );
            }
            to_srt(&formatted_to_subtitles(&formatted))
        };
        std::fs::write(&original_out, &original_srt)
            .map_err(|e| job_error(format!("falha ao gravar `{}`: {e}", original_out.display())))?;
        emit_progress(app, job_id, PipelineStep::Export, 100, None);
        emit_log(
            app,
            job_id,
            &format!("SRT gravado em {}", out_path.display()),
        );
        persist_recent(input_path, &out_path);

        let stats = crate::stats::compute_stats(
            start.elapsed().as_secs_f64(),
            duration_secs,
            &result.formatted,
            crate::hardware::tier::tier_for(&hw),
        );
        emit_log(
            app,
            job_id,
            &format!(
                "concluído em {:.1}s — {} segmentos — {:.1}× realtime",
                stats.processing_secs, stats.segments, stats.translation_ratio
            ),
        );

        Ok(PipelineSummary {
            output_path: out_path.to_string_lossy().into_owned(),
            duration_secs,
            segments: result.formatted.len(),
            source_lang: result.source_lang.as_code().into(),
            target_lang: result.target_lang.as_code().into(),
            kept_original: result.kept_original_count,
            stats,
        })
    } else {
        // Sem tradução: formata as legendas originais (transcritas ou embutidas).
        emit_log(app, job_id, "sem tradução — formatando originais");
        emit_progress(app, job_id, PipelineStep::Format, 0, None);
        let mut formatted = crate::format::format_subtitles(&subtitles);
        if duration_secs > 0.0 {
            clamp_to_audio(
                &mut formatted,
                std::time::Duration::from_secs_f64(duration_secs),
            );
        }
        emit_progress(app, job_id, PipelineStep::Format, 100, None);

        ensure_not_cancelled(token)?;
        emit_progress(app, job_id, PipelineStep::Export, 0, None);
        let srt = to_srt(&formatted_to_subtitles(&formatted));
        std::fs::write(&out_path, &srt)
            .map_err(|e| job_error(format!("falha ao gravar `{}`: {e}", out_path.display())))?;
        emit_progress(app, job_id, PipelineStep::Export, 100, None);
        emit_log(
            app,
            job_id,
            &format!("SRT gravado em {}", out_path.display()),
        );
        persist_recent(input_path, &out_path);

        let stats = crate::stats::compute_stats(
            start.elapsed().as_secs_f64(),
            duration_secs,
            &formatted,
            crate::hardware::tier::tier_for(&hw),
        );
        emit_log(
            app,
            job_id,
            &format!(
                "concluído em {:.1}s — {} segmentos — {:.1}× realtime",
                stats.processing_secs, stats.segments, stats.translation_ratio
            ),
        );

        Ok(PipelineSummary {
            output_path: out_path.to_string_lossy().into_owned(),
            duration_secs,
            segments: formatted.len(),
            source_lang: source_lang.as_code().into(),
            target_lang: String::new(),
            kept_original: 0,
            stats,
        })
    }
}

/// Persiste as "últimas escolhas" (4.10): arquivo recente + último diretório de
/// saída. Falha de save não falha o job — só loga (a gravação é best-effort).
fn persist_recent(input_path: &str, out_path: &Path) {
    let mut cfg = AppConfig::load_or_default();
    cfg.record_recent(input_path, out_path);
    if let Err(e) = cfg.save() {
        tracing::error!("falha ao persistir preferências após o job: {e}");
    }
}

/// Código estável de erro "inesperado" do pipeline (sem mapa i18n dedicado —
/// o frontend cai no dialog de erro inesperado com log path + issue).
pub const PIPELINE_FAILED: &str = "pipeline_failed";

/// Código estável quando o modelo de transcrição ativo está indisponível
/// (não ativo, não baixado ou arquivo ausente) — ação: abrir Model Manager.
pub const STT_MODEL_UNAVAILABLE: &str = "stt_model_unavailable";

/// Empacota um erro do pipeline sem código específico em [`ErrorDetail`] com o
/// código genérico `pipeline_failed` (a mensagem segue amigável/acionável).
fn job_error(message: String) -> ErrorDetail {
    ErrorDetail {
        code: PIPELINE_FAILED,
        message,
        hint: None,
    }
}

/// Resolve o caminho do modelo STT ativo da config (valida presença + checksum
/// via cache 2.4) com mensagem acionável e código estável `stt_model_unavailable`.
/// Modelo STT resolvido da config: backend + caminhos (arquivo principal e
/// diretório do modelo — o Parakeet ONNX é multi-arquivo no dir).
struct ResolvedStt {
    backend: catalog::Backend,
    /// Arquivo principal (Whisper ggml/gguf).
    main_path: std::path::PathBuf,
    /// Diretório do modelo (Parakeet: encoder/decoder/vocab juntos).
    #[cfg(feature = "parakeet")]
    model_dir: std::path::PathBuf,
}

fn resolve_stt_model(config: &AppConfig) -> Result<ResolvedStt, ErrorDetail> {
    let id = config.active_models.stt.trim();
    if id.is_empty() {
        return Err(ErrorDetail {
            code: STT_MODEL_UNAVAILABLE,
            message: "nenhum modelo de transcrição ativo — baixe e selecione um na aba Modelos"
                .into(),
            hint: None,
        });
    }
    let catalog = catalog::Catalog::embedded().map_err(|e| job_error(e.to_string()))?;
    let model = catalog
        .models
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| ErrorDetail {
            code: STT_MODEL_UNAVAILABLE,
            message: format!("modelo de transcrição `{id}` não existe no catálogo"),
            hint: None,
        })?;
    let backend = model.backend;
    // Canary/Nemotron ainda não têm engine (Canary exige FastConformer+Transformer decoder).
    if matches!(
        backend,
        catalog::Backend::Canary | catalog::Backend::Nemotron
    ) {
        return Err(ErrorDetail {
            code: "stt_backend_not_implemented",
            message: format!(
                "backend `{:?}` ainda não implementado — use Whisper ou Parakeet para transcrever",
                backend
            ),
            hint: Some("Selecione um Whisper/Parakeet na aba Modelos, ou aguarde o suporte a Canary/Nemotron."),
        });
    }
    #[cfg(feature = "parakeet")]
    let dir = cache::model_dir(model).map_err(|e| ErrorDetail {
        code: STT_MODEL_UNAVAILABLE,
        message: format!("cache do modelo `{id}`: {e}"),
        hint: None,
    })?;
    let main_path = cache::resolve_model_path(model).map_err(|e| ErrorDetail {
        code: STT_MODEL_UNAVAILABLE,
        message: format!("modelo de transcrição `{id}` indisponível: {e}"),
        hint: None,
    })?;
    Ok(ResolvedStt {
        backend,
        main_path,
        #[cfg(feature = "parakeet")]
        model_dir: dir,
    })
}

/// [`ErrorDetail`] do erro STT (código estável 1.10) sem expor caminhos
/// internos — o contexto completo fica no log via `From<SttError>`.
fn stt_detail(e: crate::stt::SttError) -> ErrorDetail {
    crate::errors::LegendaiError::from(e).to_detail()
}

/// Caminho do SRT "original" (fonte da tradução) para o preview duplo (4.5):
/// fica ao lado do SRT traduzido como `<name>.original.srt`. O frontend deriva
/// o mesmo caminho ao montar o modo duplo.
fn original_sidecar_path(out_path: &Path) -> PathBuf {
    out_path.with_extension("").with_extension("original.srt")
}

/// Deriva o caminho de saída: o fornecido ou `<dir do vídeo>/<stem>.srt`.
fn resolve_out_path(input: &Path, given: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = given.map(str::trim).filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    let stem = input
        .file_stem()
        .ok_or_else(|| format!("caminho inválido: {}", input.display()))?;
    let dir = input.parent().unwrap_or(Path::new("."));
    Ok(dir.join(format!("{}.srt", stem.to_string_lossy())))
}

fn ensure_not_cancelled(token: &CancellationToken) -> Result<(), ErrorDetail> {
    if token.is_cancelled() {
        Err(job_error("processamento cancelado".into()))
    } else {
        Ok(())
    }
}

fn emit_progress(
    app: &tauri::AppHandle,
    job_id: &str,
    step: PipelineStep,
    pct: u8,
    detail: Option<&str>,
) {
    // Mantém o progresso do item da fila (4.9) em dia no backend — a UI
    // também o patcha via `pipeline-progress`, mas o estado gravado fica
    // consistente mesmo se a UI recarregar `queue_list` no meio da execução.
    crate::pipeline::queue::update_progress(job_id, step, pct, detail);
    let _ = app.emit(
        "pipeline-progress",
        PipelineProgress {
            job_id: job_id.into(),
            step,
            pct: pct.clamp(0, 100),
            detail: detail.map(String::from),
        },
    );
}

/// Espelha o log do terminal para a UI via `pipeline-log` (tela de Detalhes).
/// Ativo após o próximo restart — o frontend já consome o evento, então o
/// vídeo em processamento agora não é interrompido.
fn emit_log(app: &tauri::AppHandle, job_id: &str, line: &str) {
    tracing::info!("[{}] {}", job_id, line);
    let _ = app.emit(
        "pipeline-log",
        serde_json::json!({ "job_id": job_id, "line": line }),
    );
}

/// Remove o diretório temporário do job ao sair do escopo (sucesso ou erro).
struct TempCleanup(PathBuf);

impl Drop for TempCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Opções opcionais do comando: override do par de idiomas da config.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct TranslateOptions {
    /// Override do idioma de origem (código ISO 639-1). `None`/vazio/`auto`
    /// mantém a config (ou o idioma detectado na legenda).
    pub source_lang: Option<String>,
    /// Override do idioma de destino. `None`/vazio/`auto` mantém a config.
    pub target_lang: Option<String>,
}

/// Resultado do comando para a UI (serde/IPC).
#[derive(Debug, Clone, Serialize)]
pub struct TranslateOutcome {
    /// Conteúdo do SRT traduzido (também gravado em `out_path`).
    pub srt: String,
    pub source_lang: String,
    pub target_lang: String,
    /// Nº de legendas formatadas no SRT final.
    pub segments: usize,
    /// Nº de segmentos que mantiveram o texto original (fallback 3.6).
    pub kept_original: usize,
}

/// Traduz o arquivo SRT em `src_path` para `out_path`, usando o modelo de
/// tradução ativo da config (3.4) e as regras de formatação 1.8.
#[tauri::command(rename_all = "snake_case")]
pub fn translate_subtitle(
    src_path: String,
    out_path: String,
    options: Option<TranslateOptions>,
) -> Result<TranslateOutcome, String> {
    let source_text = std::fs::read_to_string(&src_path)
        .map_err(|e| format!("não foi possível ler `{src_path}`: {e}"))?;
    let source = parse_srt(&source_text).map_err(|e| e.to_string())?;
    if source.is_empty() {
        return Err("o arquivo de legenda não contém nenhuma legenda".into());
    }

    let mut config = AppConfig::load_or_default();
    if let Some(opts) = options {
        if let Some(s) = opts.source_lang.filter(|s| !s.is_empty() && s != "auto") {
            config.source_lang = s;
        }
        if let Some(t) = opts.target_lang.filter(|t| !t.is_empty() && t != "auto") {
            config.target_lang = t;
        }
        // Persiste o par usado como "último par de idiomas" (4.10) — o override
        // vale para a próxima execução também.
        config.save().map_err(|e| e.to_string())?;
    }

    let hw = detect();
    let result = run_translate(&source, &config, &hw).map_err(|e| e.to_string())?;

    std::fs::write(&out_path, &result.srt)
        .map_err(|e| format!("não foi possível gravar `{out_path}`: {e}"))?;

    Ok(TranslateOutcome {
        srt: result.srt,
        source_lang: result.source_lang.as_code().into(),
        target_lang: result.target_lang.as_code().into(),
        segments: result.formatted.len(),
        kept_original: result.kept_original_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_serializa_com_tag_de_tipo() {
        let audio = PipelineSource::Audio { track_index: 2 };
        let v = serde_json::to_value(&audio).unwrap();
        assert_eq!(v["type"], "audio");
        assert_eq!(v["track_index"], 2);

        let embedded = PipelineSource::Embedded { stream_index: 1 };
        let v = serde_json::to_value(&embedded).unwrap();
        assert_eq!(v["type"], "embedded");
        assert_eq!(v["stream_index"], 1);

        let back: PipelineSource = serde_json::from_value(serde_json::json!({
            "type": "audio", "track_index": 3
        }))
        .unwrap();
        assert!(matches!(back, PipelineSource::Audio { track_index: 3 }));
    }

    #[test]
    fn out_path_usado_quando_fornecido() {
        let p = resolve_out_path(Path::new("/videos/filme.mkv"), Some("/tmp/saida.srt")).unwrap();
        assert_eq!(p, Path::new("/tmp/saida.srt"));
        // Vazio/whitespace cai no default.
        let p = resolve_out_path(Path::new("/videos/filme.mkv"), Some("   ")).unwrap();
        assert_eq!(p, Path::new("/videos/filme.srt"));
    }

    #[test]
    fn out_path_default_deriva_do_video() {
        let p = resolve_out_path(Path::new("/videos/filme.mkv"), None).unwrap();
        assert_eq!(p, Path::new("/videos/filme.srt"));
    }

    #[test]
    fn original_sidecar_path_fica_ao_lado_do_traduzido() {
        assert_eq!(
            original_sidecar_path(Path::new("/videos/filme.srt")),
            Path::new("/videos/filme.original.srt")
        );
        // Extensão custom do out_path também vira `.original.srt`.
        assert_eq!(
            original_sidecar_path(Path::new("/tmp/saida.txt")),
            Path::new("/tmp/saida.original.srt")
        );
        // Sem extensão: mesmo assim `.original.srt`.
        assert_eq!(
            original_sidecar_path(Path::new("/videos/filme")),
            Path::new("/videos/filme.original.srt")
        );
    }

    #[test]
    fn options_default_traduz_e_sem_caminho() {
        let opts = PipelineOptions::default();
        assert!(opts.translate);
        assert!(opts.out_path.is_none());
        // Campos ausentes no JSON (frontend) não quebram o parse.
        let opts: PipelineOptions = serde_json::from_str("{}").unwrap();
        assert!(opts.translate);
    }

    #[test]
    fn ensure_not_cancelled_distingue_cancelado() {
        let token = CancellationToken::new();
        assert!(ensure_not_cancelled(&token).is_ok());
        token.cancel();
        let err = ensure_not_cancelled(&token).unwrap_err();
        assert!(err.message.contains("cancelado"), "{err:?}");
    }
}
