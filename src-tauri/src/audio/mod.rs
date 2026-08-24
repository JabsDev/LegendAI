pub mod ffmpeg_extract;
pub mod ffprobe;

use crate::audio::ffmpeg_extract::AudioError;
use crate::errors::LegendaiError;
use crate::ffmpeg::FfmpegError;

/// Classifica um [`AudioError`] do fluxo STT em um [`LegendaiError`] de código
/// estável para a UI (tarefa 1.10). O contexto completo é registrado no log —
/// a mensagem exibida ao usuário nunca contém caminhos internos.
impl From<AudioError> for LegendaiError {
    fn from(e: AudioError) -> Self {
        match e {
            AudioError::Ffmpeg(FfmpegError::NotFound(name)) => {
                tracing::error!("sidecar `{name}` não encontrado");
                LegendaiError::FfmpegMissing
            }
            AudioError::Spawn { command, .. } => {
                tracing::error!("falha ao executar sidecar em {}", command.display());
                LegendaiError::FfmpegMissing
            }
            AudioError::Exit { stderr, .. } => {
                // ffmpeg reporta exatamente esta string quando a trilha pedida
                // não existe (mensagem fixa do ffmpeg) — não é corrupção.
                let no_track = stderr.contains("matches no streams");
                tracing::error!(
                    no_track,
                    "ffmpeg falhou na extração (stderr completo em debug)"
                );
                if no_track {
                    LegendaiError::NoAudioTrack
                } else {
                    LegendaiError::CorruptedFile
                }
            }
            AudioError::EmptyOutput { .. } => {
                tracing::error!("ffmpeg não produziu áudio (trilha vazia?)");
                LegendaiError::NoAudioTrack
            }
            AudioError::Json(_) => {
                tracing::error!("ffprobe retornou JSON inválido");
                LegendaiError::CorruptedFile
            }
            // FfmpegError::Triple/Spawn/Exit — fora do fluxo normal de STT.
            AudioError::Ffmpeg(other) => {
                tracing::error!("erro de ffmpeg não classificado: {other}");
                LegendaiError::CorruptedFile
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::path::PathBuf;

    fn classify(e: AudioError) -> LegendaiError {
        LegendaiError::from(e)
    }

    #[test]
    fn sidecar_ausente_vira_ffmpeg_missing() {
        assert!(matches!(
            classify(AudioError::Ffmpeg(FfmpegError::NotFound("ffmpeg".into()))),
            LegendaiError::FfmpegMissing
        ));
        assert!(matches!(
            classify(AudioError::Spawn {
                command: PathBuf::from("ffmpeg-x86_64-unknown-linux-gnu"),
                source: io::Error::new(io::ErrorKind::NotFound, "não existe"),
            }),
            LegendaiError::FfmpegMissing
        ));
    }

    #[test]
    fn trilha_inexistente_vira_no_audio_track() {
        // Mensagem fixa do ffmpeg quando `-map 0:a:<idx>` não casa nenhuma trilha.
        let missing = AudioError::Exit {
            code: 1,
            stderr: "Stream map '0:a:0' matches no streams.\nError opening input!".into(),
        };
        assert!(matches!(classify(missing), LegendaiError::NoAudioTrack));

        let empty = AudioError::EmptyOutput {
            path: PathBuf::from("/tmp/out.wav"),
        };
        assert!(matches!(classify(empty), LegendaiError::NoAudioTrack));
    }

    #[test]
    fn midia_invalida_vira_corrupted_file() {
        let bad = AudioError::Exit {
            code: 1,
            stderr: "Invalid data found when processing input".into(),
        };
        assert!(matches!(classify(bad), LegendaiError::CorruptedFile));
        assert!(matches!(
            classify(AudioError::Json("não é json".into())),
            LegendaiError::CorruptedFile
        ));
        assert!(matches!(
            classify(AudioError::Ffmpeg(FfmpegError::Exit(
                "ffmpeg".into(),
                1,
                "erro".into()
            ))),
            LegendaiError::CorruptedFile
        ));
    }
}
