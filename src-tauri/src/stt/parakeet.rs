//! Engine de transcrição Parakeet TDT (NVIDIA, 25 línguas) via ONNX Runtime.
//!
//! Usa o crate `parakeet-rs` (ort) — mesma stack do NLLB — para rodar os
//! modelos FastConformer-TDT convertidos para ONNX (`istupakov/parakeet-tdt-0.6b-v3-onnx`:
//! encoder-model.onnx, decoder_joint-model.onnx, vocab.txt). O TDT prediz
//! pontuação e timestamps por token, então `TimestampMode::Sentences` devolve
//! sentenças com tempo — equivalente ao segmento do Whisper.
//!
//! O Parakeet auto-detecta o idioma mas não o expõe via API; para tradução o
//! caller deve passar o override (select "Idioma do áudio" da importação).

use std::path::Path;

use parakeet_rs::{ExecutionConfig, ExecutionProvider, ParakeetTDT, TimestampMode, Transcriber};
use thiserror::Error;

use crate::domain::{Language, Segment, Timestamp};

/// Erros do pipeline Parakeet.
#[derive(Debug, Error)]
pub enum ParakeetError {
    #[error("modelo Parakeet não encontrado em `{0}` — baixe um modelo ONNX do Parakeet")]
    ModelNotFound(String),
    #[error("falha ao carregar modelo Parakeet em `{path}`: {source}")]
    ModelLoad {
        path: String,
        #[source]
        source: parakeet_rs::Error,
    },
    #[error("falha ao transcrever com Parakeet: {0}")]
    Transcribe(#[source] parakeet_rs::Error),
    #[error("falha ao ler o WAV em `{path}`: {source}")]
    WavRead {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("WAV inválido em `{path}`: {reason}")]
    InvalidWav {
        path: std::path::PathBuf,
        reason: String,
    },
}

/// Modelo Parakeet TDT carregado (encoder + decoder_joint + vocab).
pub struct ParakeetModel {
    engine: ParakeetTDT,
}

impl ParakeetModel {
    /// Carrega o modelo do diretório que contém os arquivos ONNX do Parakeet
    /// (`encoder-model.onnx`, `encoder-model.onnx.data`, `decoder_joint-model.onnx`,
    /// `vocab.txt`). `use_cuda` tenta CUDA EP quando o binário tem a feature.
    pub fn load(dir: &Path, threads: usize, use_cuda: bool) -> Result<Self, ParakeetError> {
        if !dir.is_dir() {
            return Err(ParakeetError::ModelNotFound(dir.display().to_string()));
        }
        let provider = if use_cuda {
            #[cfg(feature = "cuda")]
            {
                ExecutionProvider::Cuda
            }
            #[cfg(not(feature = "cuda"))]
            {
                tracing::warn!("Parakeet: GPU detectada mas binário sem feature `cuda` — CPU");
                ExecutionProvider::Cpu
            }
        } else {
            ExecutionProvider::Cpu
        };
        let config = ExecutionConfig {
            execution_provider: provider,
            intra_threads: threads.max(1),
            inter_threads: 1,
            ..Default::default()
        };
        let provider_label = format!("{:?}", config.execution_provider);
        let engine = ParakeetTDT::from_pretrained(dir, Some(config)).map_err(|e| {
            ParakeetError::ModelLoad {
                path: dir.display().to_string(),
                source: e,
            }
        })?;
        tracing::info!(
            "Parakeet TDT carregado de {} ({provider_label})",
            dir.display()
        );
        Ok(Self { engine })
    }

    /// Transcreve um WAV 16kHz mono (PCM s16). `language` é o override (o TDT
    /// auto-detecta mas não reporta); `None` mantém `auto`.
    pub fn transcribe(
        &mut self,
        wav_path: &Path,
        language: Option<Language>,
    ) -> Result<super::whisper::Transcription, ParakeetError> {
        let samples = super::whisper::read_wav_samples(wav_path).map_err(|e| match e {
            super::whisper::SttError::WavRead { path, source } => {
                ParakeetError::WavRead { path, source }
            }
            super::whisper::SttError::InvalidWav { path, reason } => {
                ParakeetError::InvalidWav { path, reason }
            }
            _ => ParakeetError::Transcribe(parakeet_rs::Error::Audio("wav inválido".into())),
        })?;
        let result = self
            .engine
            .transcribe_samples(samples, 16000, 1, Some(TimestampMode::Sentences))
            .map_err(ParakeetError::Transcribe)?;

        let lang = language.unwrap_or_else(Language::auto);
        let mut segments: Vec<Segment> = Vec::new();
        for tok in &result.tokens {
            let text = tok.text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            let start = Timestamp::from_ms((tok.start.max(0.0) * 1000.0) as u64);
            let end = Timestamp::from_ms((tok.end.max(0.0) * 1000.0) as u64);
            match Segment::new(text, start, end, lang.clone()) {
                Ok(seg) => segments.push(seg),
                Err(e) => tracing::debug!("segmento parakeet inválido ignorado: {e}"),
            }
        }
        Ok(super::whisper::Transcription {
            language: lang,
            segments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modelo_ausente_retorna_erro_tipado() {
        match ParakeetModel::load(Path::new("/nao/existe"), 4, false) {
            Ok(_) => panic!("esperava erro para modelo ausente"),
            Err(e) => assert!(matches!(e, ParakeetError::ModelNotFound(_))),
        }
    }

    #[test]
    fn wav_invalido_vira_erro_tipado() {
        let path =
            std::env::temp_dir().join(format!("legendai-pk-test-{}-lixo.wav", std::process::id()));
        std::fs::write(&path, b"nao e um wav").unwrap();
        let err = super::super::whisper::read_wav_samples(&path).unwrap_err();
        std::fs::remove_file(&path).ok();
        assert!(matches!(
            err,
            super::super::whisper::SttError::InvalidWav { .. }
        ));
    }
}
