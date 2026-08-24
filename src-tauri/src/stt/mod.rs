#[cfg(feature = "parakeet")]
pub mod parakeet;
pub mod whisper;

#[allow(unused_imports)] // API pública consumida pelo pipeline (1.9) e comandos IPC
pub use whisper::{SttError, SttOptions, Transcription, WhisperModel};

use crate::errors::LegendaiError;

/// Classifica um [`ParakeetError`] em um [`LegendaiError`] de código estável.
#[cfg(feature = "parakeet")]
impl From<parakeet::ParakeetError> for LegendaiError {
    fn from(e: parakeet::ParakeetError) -> Self {
        match e {
            parakeet::ParakeetError::ModelNotFound(_) => {
                tracing::error!("modelo Parakeet ausente");
                LegendaiError::ModelMissing
            }
            parakeet::ParakeetError::ModelLoad { .. } => {
                tracing::error!("modelo Parakeet não carrega (ONNX corrompido/ausente)");
                LegendaiError::ModelCorrupt
            }
            parakeet::ParakeetError::WavRead { .. }
            | parakeet::ParakeetError::InvalidWav { .. } => {
                tracing::error!("WAV de entrada ilegível ou inválido");
                LegendaiError::CorruptedFile
            }
            parakeet::ParakeetError::Transcribe(_) => {
                tracing::error!("falha no runtime do Parakeet (detalhes no stderr)");
                LegendaiError::TranscribeFailed
            }
        }
    }
}

/// Classifica um [`SttError`] em um [`LegendaiError`] de código estável para
/// a UI (tarefa 1.10). O contexto completo é registrado no log — a mensagem
/// exibida ao usuário nunca contém caminhos internos.
impl From<SttError> for LegendaiError {
    fn from(e: SttError) -> Self {
        match e {
            SttError::ModelNotFound(path) => {
                tracing::error!("modelo ausente em {}", path.display());
                LegendaiError::ModelMissing
            }
            SttError::ModelLoad { .. } => {
                tracing::error!("modelo whisper não carrega (corrompido/formato errado)");
                LegendaiError::ModelCorrupt
            }
            SttError::WavRead { .. } | SttError::InvalidWav { .. } => {
                tracing::error!("WAV de entrada ilegível ou inválido");
                LegendaiError::CorruptedFile
            }
            SttError::UnsupportedLanguage(code) => {
                tracing::error!("idioma não suportado: {code}");
                LegendaiError::UnsupportedLanguage(code)
            }
            SttError::CreateState(_) | SttError::Transcribe(_) => {
                tracing::error!("falha no runtime do whisper (detalhes no stderr)");
                LegendaiError::TranscribeFailed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::PathBuf;
    use whisper_rs::WhisperError;

    fn classify(e: SttError) -> LegendaiError {
        LegendaiError::from(e)
    }

    #[test]
    fn modelo_ausente_vira_model_missing() {
        assert!(matches!(
            classify(SttError::ModelNotFound(PathBuf::from("/x/ggml-tiny.bin"))),
            LegendaiError::ModelMissing
        ));
    }

    #[test]
    fn modelo_corrompido_vira_model_corrupt() {
        assert!(matches!(
            classify(SttError::ModelLoad {
                path: PathBuf::from("/x/model.bin"),
                source: WhisperError::InitError,
            }),
            LegendaiError::ModelCorrupt
        ));
    }

    #[test]
    fn wav_invalido_vira_corrupted_file() {
        assert!(matches!(
            classify(SttError::InvalidWav {
                path: PathBuf::from("/x.wav"),
                reason: "sem chunk data".into(),
            }),
            LegendaiError::CorruptedFile
        ));
        assert!(matches!(
            classify(SttError::WavRead {
                path: PathBuf::from("/x.wav"),
                source: io::Error::new(io::ErrorKind::PermissionDenied, "x"),
            }),
            LegendaiError::CorruptedFile
        ));
    }

    #[test]
    fn runtime_do_whisper_vira_transcribe_failed() {
        assert!(matches!(
            classify(SttError::CreateState(WhisperError::FailedToCreateState)),
            LegendaiError::TranscribeFailed
        ));
        assert!(matches!(
            classify(SttError::Transcribe(WhisperError::FailedToDecode)),
            LegendaiError::TranscribeFailed
        ));
    }

    #[test]
    fn idioma_invalido_preserva_codigo_e_mensagem_estavel() {
        let e = classify(SttError::UnsupportedLanguage("zz".into()));
        match &e {
            LegendaiError::UnsupportedLanguage(code) => assert_eq!(code, "zz"),
            other => panic!("esperava UnsupportedLanguage, veio {other}"),
        }
        assert_eq!(e.to_detail().code, "unsupported_language");
    }
}
