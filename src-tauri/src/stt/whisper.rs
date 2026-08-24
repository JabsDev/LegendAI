use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use thiserror::Error;
use tracing::{debug, warn};
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperError,
};

use crate::domain::subtitle::{Language, Segment, Timestamp};

/// Erros do pipeline STT.
#[derive(Debug, Error)]
#[allow(dead_code)] // API pública consumida pelo pipeline (1.9) e comandos IPC
pub enum SttError {
    #[error("modelo não encontrado em `{0}` — baixe um GGUF do Whisper e aponte para ele (ex: LEGENDAI_MODEL_PATH)")]
    ModelNotFound(PathBuf),
    #[error("falha ao carregar o modelo em `{path}`: {source} — arquivo pode estar corrompido ou não ser um modelo whisper válido")]
    ModelLoad {
        path: PathBuf,
        #[source]
        source: WhisperError,
    },
    #[error("falha ao criar estado do whisper: {0}")]
    CreateState(WhisperError),
    #[error("falha ao ler o WAV em `{path}`: {source}")]
    WavRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("WAV inválido em `{path}`: {reason}")]
    InvalidWav { path: PathBuf, reason: String },
    #[error("falha ao transcrever: {0}")]
    Transcribe(WhisperError),
    #[error(
        "idioma `{0}` não é suportado pelo Whisper — use um código ISO 639-1 (ex: pt, en, es)"
    )]
    UnsupportedLanguage(String),
}

/// Opções de transcrição. Valores "conforme tier" chegam com a detecção de
/// hardware (2.5/2.6) — aqui os defaults já são sanidade para qualquer máquina.
#[derive(Debug, Clone)]
#[allow(dead_code)] // tier real configura via 2.6
pub struct SttOptions {
    /// Threads do decodificador (default: núcleos disponíveis).
    pub threads: usize,
    /// `best_of` do sampling greedy (1-5; 5 = padrão do whisper.cpp).
    pub best_of: i32,
    /// Comprimento máximo por segmento em tokens (0 = sem limite).
    pub max_len: i32,
    /// Idioma forçado (override manual). `None` = auto-detecção do Whisper.
    pub language: Option<Language>,
}

impl Default for SttOptions {
    fn default() -> Self {
        Self {
            threads: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4),
            best_of: 5,
            max_len: 0,
            language: None,
        }
    }
}

/// Modelo Whisper carregado (GGUF ou ggml). `WhisperContext` é `Send + Sync`,
/// então o modelo pode ser compartilhado entre chamadas de transcrição.
#[derive(Debug)]
#[allow(dead_code)] // pipeline (1.9) e comandos IPC consomem
pub struct WhisperModel {
    ctx: WhisperContext,
}

/// Resultado de uma transcrição: idioma detectado + segmentos (timestamps em ms).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Transcription {
    pub language: Language,
    pub segments: Vec<Segment>,
}

#[allow(dead_code)] // pipeline (1.9) e comandos IPC consomem
impl WhisperModel {
    /// Carrega um modelo GGUF/ggml de `path`. Caminho inexistente retorna
    /// `SttError::ModelNotFound` (mensagem acionável), em vez de falhar no init.
    pub fn load(path: &Path) -> Result<Self, SttError> {
        if !path.exists() {
            return Err(SttError::ModelNotFound(path.to_path_buf()));
        }
        let params = WhisperContextParameters::default(); // CPU (use_gpu=false sem feature _gpu)
        let ctx = WhisperContext::new_with_params(path, params).map_err(|source| {
            SttError::ModelLoad {
                path: path.to_path_buf(),
                source,
            }
        })?;
        tracing::info!("modelo whisper carregado de {}", path.display());
        Ok(Self { ctx })
    }

    /// Transcreve um WAV 16kHz mono (PCM s16), retornando segmentos com
    /// timestamps em ms. O idioma é auto-detectado, ou forçado pelo
    /// `opts.language` (override manual validado contra a lista do Whisper).
    pub fn transcribe(
        &self,
        wav_path: &Path,
        opts: &SttOptions,
    ) -> Result<Transcription, SttError> {
        self.transcribe_with_progress(wav_path, opts, |_| true)
    }

    /// Transcreve reportando progresso e permitindo cancelamento cooperativo.
    ///
    /// `progress` é chamado com a porcentagem (0-100) conforme o whisper avança;
    /// retornar `false` aborta a transcrição (a próxima checagem do abort
    /// callback interrompe a inferência). É o mecanismo usado pela tela de
    /// processamento (4.3) para emitir `pipeline-progress` e interromper a etapa
    /// STT (checagem entre segmentos/frames, nota da tarefa).
    pub fn transcribe_with_progress<F>(
        &self,
        wav_path: &Path,
        opts: &SttOptions,
        progress: F,
    ) -> Result<Transcription, SttError>
    where
        F: FnMut(i32) -> bool + 'static,
    {
        // Override inválido falha antes de tocar modelo/WAV (mensagem clara,
        // não fallback silencioso para outro idioma).
        if let Some(lang) = &opts.language {
            validate_language(lang)?;
        }
        let samples = read_wav_samples(wav_path)?;
        let mut state = self.ctx.create_state().map_err(SttError::CreateState)?;

        let mut params = FullParams::new(SamplingStrategy::Greedy {
            best_of: opts.best_of,
        });
        params.set_translate(false);
        // Override: força o modelo a transcrever no idioma pedido (setado
        // ANTES de rodar, não como pós-filtro). Sem override: auto-detecção
        // que CONTINUA transcrevendo (`set_language(None)`).
        // ⚠️ `set_detect_language(true)` faz o whisper.cpp retornar logo após
        // detectar o idioma (modo "só detecta", sem transcrever) — evitar.
        params.set_language(opts.language.as_ref().map(|l| l.as_code()));
        params.set_n_threads(opts.threads as i32);
        params.set_max_len(opts.max_len);
        // Silenciar a saída padrão do whisper.cpp (vai para stderr/console).
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Progresso + aborto cooperativo: a callback de progresso recebe o pct e
        // repassa ao chamador; se o chamador devolve `false`, liga a flag de
        // aborto que a callback de aborto expõe ao whisper.cpp (que a consulta
        // entre frames). As duas callbacks compartilham a flag via `Arc`.
        let abort = Arc::new(AtomicBool::new(false));
        let abort_flag = abort.clone();
        let mut cb = progress;
        params.set_progress_callback_safe::<Option<Box<dyn FnMut(i32)>>, Box<dyn FnMut(i32)>>(
            Some(Box::new(move |pct: i32| {
                if !cb(pct) {
                    abort_flag.store(true, Ordering::Relaxed);
                }
            }) as Box<dyn FnMut(i32)>),
        );
        params
            .set_abort_callback_safe::<Option<Box<dyn FnMut() -> bool>>, Box<dyn FnMut() -> bool>>(
                Some(Box::new(move || abort.load(Ordering::Relaxed)) as Box<dyn FnMut() -> bool>),
            );

        state.full(params, &samples).map_err(SttError::Transcribe)?;

        // Override manda: o idioma reportado é o forçado, não o que o modelo
        // "teria" detectado. Sem override, lê a detecção do estado pós-run.
        let language = opts
            .language
            .clone()
            .unwrap_or_else(|| detect_language(&state));
        debug!(
            "idioma: {} (override: {})",
            language.as_code(),
            opts.language.is_some()
        );

        let mut segments = Vec::new();
        for i in 0..state.full_n_segments() {
            let seg = state.get_segment(i).expect("i dentro dos limites");
            let text = seg.to_str_lossy().unwrap_or_default().trim().to_string();
            if text.is_empty() {
                continue;
            }
            // Timestamps do bind vêm em centésimos de segundo (×10 para ms).
            let start = Timestamp::from_ms((seg.start_timestamp().max(0) * 10) as u64);
            let end = Timestamp::from_ms((seg.end_timestamp().max(0) * 10) as u64);
            match Segment::new(text, start, end, language.clone()) {
                Ok(segment) => segments.push(segment),
                Err(e) => debug!("segmento inválido ignorado: {e}"),
            }
        }

        Ok(Transcription { language, segments })
    }
}

/// Valida um override contra a lista de idiomas suportados pelo Whisper
/// (`whisper_lang_id`). Código desconhecido vira erro tipado claro, não um
/// fallback silencioso para a auto-detecção.
#[allow(dead_code)] // consumida via WhisperModel::transcribe
fn validate_language(lang: &Language) -> Result<(), SttError> {
    // `auto`/vazio é sentinela de auto-detecção, não um idioma concreto para forçar.
    if lang.is_auto() {
        return Err(SttError::UnsupportedLanguage("auto".into()));
    }
    if whisper_rs::get_lang_id(lang.as_code()).is_some() {
        Ok(())
    } else {
        Err(SttError::UnsupportedLanguage(lang.as_code().into()))
    }
}

/// Lê o idioma detectado do estado pós-execução (`full_lang_id_from_state`).
#[allow(dead_code)] // consumida via WhisperModel::transcribe (1.9)
fn detect_language(state: &whisper_rs::WhisperState) -> Language {
    let id = state.full_lang_id_from_state();
    match whisper_rs::get_lang_str(id) {
        Some(code) => Language::from_code(code),
        None => {
            warn!("idioma desconhecido (id={id}), usando fallback");
            Language::auto()
        }
    }
}

/// Lê um WAV RIFF/PCM e converte para f32 mono (esperado pelo whisper).
/// Suporta apenas PCM não-comprimido; valida mono e 16-bit com mensagem acionável.
#[allow(dead_code)] // consumida via WhisperModel::transcribe (1.9) e ParakeetModel (stt/parakeet.rs)
pub(crate) fn read_wav_samples(path: &Path) -> Result<Vec<f32>, SttError> {
    let bytes = std::fs::read(path).map_err(|source| SttError::WavRead {
        path: path.to_path_buf(),
        source,
    })?;
    let invalid = |reason: String| SttError::InvalidWav {
        path: path.to_path_buf(),
        reason,
    };

    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(invalid("assinatura RIFF/WAVE ausente".into()));
    }

    let mut pos = 12usize;
    let mut sample_rate = 0u32;
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut data: Option<&[u8]> = None;

    while pos + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body = match bytes.get(pos + 8..pos + 8 + len) {
            Some(b) => b,
            None => return Err(invalid("chunk truncado".into())),
        };
        match &bytes[pos..pos + 4] {
            b"fmt " if body.len() >= 16 => {
                if u16::from_le_bytes(body[0..2].try_into().unwrap()) != 1 {
                    return Err(invalid(
                        "formato de áudio não é PCM (comprimido não suportado)".into(),
                    ));
                }
                channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
                sample_rate = u32::from_le_bytes(body[4..8].try_into().unwrap());
                bits = u16::from_le_bytes(body[14..16].try_into().unwrap());
            }
            b"data" => data = Some(body),
            _ => {}
        }
        // Chunks são alinhados a palavras pares (padding de 1 byte se ímpar).
        pos += 8 + len + (len & 1);
    }

    if channels != 1 {
        return Err(invalid(
            "áudio estéreo — extraia com `-ac 1` (ver audio::ffmpeg_extract)".into(),
        ));
    }
    if bits != 16 {
        return Err(invalid(format!(
            "profundidade de {bits} bits — o pipeline extrai PCM s16 (16 bits)"
        )));
    }
    if sample_rate != 16000 {
        warn!("sample rate de {sample_rate} Hz (whisper espera 16kHz) — transcrevendo mesmo assim");
    }
    let data = data.ok_or_else(|| invalid("sem chunk `data` (áudio vazio?)".into()))?;
    if data.is_empty() {
        return Err(invalid(
            "chunk `data` vazio — sem amostras para transcrever".into(),
        ));
    }

    Ok(data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| i16::from_le_bytes(*pair) as f32 / 32768.0)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_wav_bytes(channels: u16, bits: u16, samples: &[i16]) -> Vec<u8> {
        let data_len = samples.len() * (bits as usize / 8);
        let mut bytes = Vec::with_capacity(44 + data_len);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&16000u32.to_le_bytes());
        bytes.extend_from_slice(&(16000u32 * channels as u32 * (bits as u32 / 8)).to_le_bytes());
        bytes.extend_from_slice(&((bits / 8) * channels).to_le_bytes()); // block align (u16)
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        bytes
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("legendai-stt-{}-{name}", std::process::id()))
    }

    #[test]
    fn modelo_ausente_retorna_erro_tipado() {
        let err = WhisperModel::load(Path::new("/nao/existe/ggml-tiny.bin")).unwrap_err();
        assert!(
            matches!(err, SttError::ModelNotFound(_)),
            "esperava ModelNotFound, veio {err}"
        );
    }

    #[test]
    fn wav_invalido_retorna_erro_tipado() {
        let path = temp_path("lixo.wav");
        std::fs::write(&path, b"isto nao e um wav").unwrap();
        // Arquivo existe mas não é um modelo válido → ModelLoad (não ModelNotFound).
        let err = WhisperModel::load(&path).unwrap_err();
        assert!(
            matches!(err, SttError::ModelLoad { .. }),
            "esperava ModelLoad, veio {err}"
        );

        let err = read_wav_samples(&path).unwrap_err();
        assert!(
            matches!(err, SttError::InvalidWav { .. }),
            "esperava InvalidWav, veio {err}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn wav_pcm_16khz_mono_vira_f32() {
        let path = temp_path("ok.wav");
        let samples: Vec<i16> = [0, 16384, -16384, 32767, -32768].into_iter().collect();
        std::fs::write(&path, make_wav_bytes(1, 16, &samples)).unwrap();

        let got = read_wav_samples(&path).unwrap();
        assert_eq!(got.len(), samples.len());
        for (g, s) in got.iter().zip(samples.iter()) {
            let expected = *s as f32 / 32768.0;
            assert!((g - expected).abs() < 1e-6, "esperava {expected}, veio {g}");
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn wav_estereo_ou_nao_pcm_rejeitado() {
        let stereo = temp_path("stereo.wav");
        std::fs::write(&stereo, make_wav_bytes(2, 16, &[0, 0])).unwrap();
        let err = read_wav_samples(&stereo).unwrap_err();
        assert!(
            matches!(err, SttError::InvalidWav { .. }),
            "esperava InvalidWav (estéreo), veio {err}"
        );
        std::fs::remove_file(&stereo).ok();

        let mp3ish = temp_path("nao-pcm.wav");
        let mut b = make_wav_bytes(1, 16, &[0]);
        b[20..22].copy_from_slice(&6u16.to_le_bytes()); // formato 6 = ALAW, não PCM
        std::fs::write(&mp3ish, b).unwrap();
        let err = read_wav_samples(&mp3ish).unwrap_err();
        assert!(
            matches!(err, SttError::InvalidWav { .. }),
            "esperava InvalidWav (não PCM), veio {err}"
        );
        std::fs::remove_file(&mp3ish).ok();
    }

    #[test]
    fn lang_id_mapeia_para_codigo_iso() {
        let id = whisper_rs::get_lang_id("pt").unwrap();
        assert_eq!(whisper_rs::get_lang_str(id).unwrap(), "pt");
        assert_eq!(
            whisper_rs::get_lang_str(id).map(Language::from_code),
            Some(Language::Pt)
        );
        assert_eq!(whisper_rs::get_lang_id("zz").unwrap_or(-1), -1);
    }

    #[test]
    fn override_idioma_valido_aceito() {
        for code in ["pt", "en", "es", "zh", "ko", "hi"] {
            validate_language(&Language::from_code(code))
                .unwrap_or_else(|e| panic!("`{code}` deveria ser aceito: {e}"));
        }
    }

    #[test]
    fn override_idioma_invalido_retorna_erro_claro() {
        for code in ["zz", "xx", "pt-br-XX"] {
            let err = validate_language(&Language::from_code(code)).unwrap_err();
            let expected = code.to_ascii_lowercase();
            assert!(
                matches!(err, SttError::UnsupportedLanguage(ref c) if c == &expected),
                "esperava UnsupportedLanguage para `{code:?}`, veio {err}"
            );
        }
        // Vazio/auto = sentinela de auto-detecção → rejeitado como "auto".
        let err = validate_language(&Language::from_code("")).unwrap_err();
        assert!(
            matches!(err, SttError::UnsupportedLanguage(ref c) if c == "auto"),
            "esperava UnsupportedLanguage(auto), veio {err}"
        );
        // Tags de container normalizadas (pt-br → pt) viram override válido.
        assert!(validate_language(&Language::from_code("pt-br")).is_ok());
    }

    /// Teste manual de transcrição: requer modelo GGUF (env LEGENDAI_MODEL_PATH)
    /// e um WAV 16kHz mono com fala (env LEGENDAI_WAV_PATH). Roda com:
    /// `cargo test --features stt -- --ignored transcreve_fala`
    #[test]
    #[ignore = "exige modelo whisper GGUF + WAV de fala via env (não roda em CI)"]
    fn transcreve_fala_com_timestamps_monotonicos() {
        let model = std::env::var("LEGENDAI_MODEL_PATH")
            .expect("sete LEGENDAI_MODEL_PATH (GGUF do whisper) para rodar");
        let wav = std::env::var("LEGENDAI_WAV_PATH")
            .expect("sete LEGENDAI_WAV_PATH (WAV 16kHz mono com fala) para rodar");

        let m = WhisperModel::load(Path::new(&model)).expect("carregar modelo");
        let t = m
            .transcribe(Path::new(&wav), &SttOptions::default())
            .expect("transcrever");

        assert!(
            !t.segments.is_empty(),
            "nenhum segmento produzido — o WAV tem fala?"
        );
        let text: String = t
            .segments
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            text.split_whitespace().count() >= 3,
            "texto muito curto, suspeito: {text:?}"
        );
        for pair in t.segments.windows(2) {
            assert!(
                pair[1].start_ms >= pair[0].end_ms,
                "timestamps não monotônicos: {:?}..{:?} depois {:?}..{:?}",
                pair[0].start_ms,
                pair[0].end_ms,
                pair[1].start_ms,
                pair[1].end_ms
            );
        }
        eprintln!(
            "idioma: {} | segmentos: {} | texto: {text:?}",
            t.language.as_code(),
            t.segments.len()
        );
    }

    /// Teste manual do override de idioma: requer modelo GGUF (env
    /// LEGENDAI_MODEL_PATH), WAV 16kHz mono (env LEGENDAI_WAV_PATH) e um código
    /// ISO de 2 letras (env LEGENDAI_LANG). Roda com:
    /// `cargo test --features stt -- --ignored transcreve_com_override`
    #[test]
    #[ignore = "exige modelo whisper GGUF + WAV de fala via env (não roda em CI)"]
    fn transcreve_com_override_de_idioma() {
        let model = std::env::var("LEGENDAI_MODEL_PATH")
            .expect("sete LEGENDAI_MODEL_PATH (GGUF do whisper) para rodar");
        let wav = std::env::var("LEGENDAI_WAV_PATH")
            .expect("sete LEGENDAI_WAV_PATH (WAV 16kHz mono com fala) para rodar");
        let code = std::env::var("LEGENDAI_LANG").expect("sete LEGENDAI_LANG (ex: en) para rodar");

        let m = WhisperModel::load(Path::new(&model)).expect("carregar modelo");
        let opts = SttOptions {
            language: Some(Language::from_code(&code)),
            ..Default::default()
        };
        let t = m.transcribe(Path::new(&wav), &opts).expect("transcrever");

        assert_eq!(
            t.language.as_code(),
            code,
            "override deve forçar o idioma reportado para `{code}`"
        );
        assert!(!t.segments.is_empty(), "nenhum segmento produzido");
        eprintln!(
            "override={} | segmentos: {} | texto: {:?}",
            t.language.as_code(),
            t.segments.len(),
            t.segments
                .iter()
                .map(|s| s.text.clone())
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}
