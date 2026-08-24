use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use thiserror::Error;
use tracing::{debug, warn};

use crate::ffmpeg::{self, FFMPEG};

/// Erros do pipeline de extração de áudio.
#[derive(Debug, Error)]
#[allow(dead_code)] // API pública consumida pelo pipeline (1.9) e comandos IPC
pub enum AudioError {
    #[error(transparent)]
    Ffmpeg(#[from] ffmpeg::FfmpegError),
    #[error("falha ao executar ffmpeg em `{command}`: {source}")]
    Spawn {
        command: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("ffmpeg saiu com código {code}: {stderr}")]
    Exit { code: i32, stderr: String },
    #[error("WAV de saída não gerado ou vazio: {path}")]
    EmptyOutput { path: PathBuf },
    #[error("resposta JSON do ffprobe inválida: {0}")]
    Json(String),
}

/// Extrai a trilha de áudio `stream_index` de `video_path` para `out_path`
/// como WAV 16kHz mono (PCM s16le), o formato esperado pelo Whisper.
///
/// `stream_index` é o índice GLOBAL da stream no container (como reportado pelo
/// ffprobe — `AudioTrack::index`). Usamos `-map 0:<global>` e NÃO `-map 0:a:N`:
/// este último é relativo ao tipo e erraria em vídeo com vídeo+áudio (ex: a 1ª
/// trilha de áudio tem índice global 1, mas `0:a:1` selecionaria a 2ª trilha).
///
/// Retorna o caminho do WAV gerado e a duração estimada do vídeo (0 se o
/// ffmpeg não reportar duração). O chamador é responsável por remover
/// `out_path` após o uso (o pipeline 1.9 gerencia o temp dir).
#[allow(dead_code)] // consumida pelo pipeline 1.9 / comandos IPC
pub fn extract_wav(
    video_path: &Path,
    stream_index: usize,
    out_path: &Path,
) -> Result<(PathBuf, Duration), AudioError> {
    let bin = ffmpeg::binary_path(FFMPEG)?;
    let video = video_path.to_string_lossy();
    let map = format!("0:{stream_index}");
    let out = out_path.to_string_lossy();
    let args = [
        "-y",
        "-i",
        &video,
        "-map",
        &map,
        "-ar",
        "16000",
        "-ac",
        "1",
        "-c:a",
        "pcm_s16le",
        &out,
    ];
    let output = Command::new(&bin)
        .args(args)
        .output()
        .map_err(|source| AudioError::Spawn {
            command: bin.clone(),
            source,
        })?;

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    debug!("stderr do ffmpeg:\n{stderr}");

    if !output.status.success() {
        return Err(AudioError::Exit {
            code: output.status.code().unwrap_or(-1),
            stderr,
        });
    }

    let duration = parse_duration(&stderr).unwrap_or_else(|| {
        warn!(
            "duração não detectada na saída do ffmpeg para {}",
            video_path.display()
        );
        Duration::ZERO
    });

    // Cabeçalho WAV PCM mínimo = 44 bytes; menos que isso é saída vazia.
    let empty = out_path.metadata().map(|m| m.len() <= 44).unwrap_or(true);
    if empty {
        return Err(AudioError::EmptyOutput {
            path: out_path.to_path_buf(),
        });
    }

    Ok((out_path.to_path_buf(), duration))
}

/// Lê `Duration: HH:MM:SS.cs` da saída do ffmpeg.
fn parse_duration(stderr: &str) -> Option<Duration> {
    let line = stderr.lines().find(|l| l.contains("Duration:"))?;
    let after = line.split("Duration:").nth(1)?;
    let time = after.split(',').next()?.trim();
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].trim().parse().ok()?;
    let m: f64 = parts[1].trim().parse().ok()?;
    let s: f64 = parts[2].trim().parse().ok()?;
    Some(Duration::from_secs_f64(h * 3600.0 + m * 60.0 + s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffmpeg_path() -> Option<PathBuf> {
        // Binários são download (gitignored) — se ausentes, skip sem falhar (CI).
        ffmpeg::binary_path(FFMPEG).ok()
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("legendai-test-{}-{name}", std::process::id()))
    }

    /// Gera uma fixture WAV (44100 mono) de `seconds`s de um seno via lavfi —
    /// sem rede e sem commitar binário no repo.
    fn make_sine_wav(path: &Path, seconds: u32) {
        let bin = ffmpeg_path().expect("fixture exige ffmpeg sidecar");
        let out = Command::new(bin)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("sine=frequency=440:duration={seconds}"),
                "-ar",
                "44100",
                "-ac",
                "1",
                &path.to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "lavfi falhou: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Valida o header WAV: PCM 16-bit, mono, 16000 Hz.
    fn assert_wav_16000_mono_s16(path: &Path) {
        let bytes = std::fs::read(path).unwrap();
        assert!(
            bytes.len() > 44,
            "WAV de saída deve ter mais que o header de 44 bytes, veio {}",
            bytes.len()
        );
        assert_eq!(&bytes[0..4], b"RIFF", "assinatura RIFF");
        assert_eq!(&bytes[8..12], b"WAVE", "tipo WAVE");
        assert_eq!(
            u16::from_le_bytes(bytes[20..22].try_into().unwrap()),
            1,
            "formato PCM"
        );
        assert_eq!(
            u16::from_le_bytes(bytes[22..24].try_into().unwrap()),
            1,
            "canais = mono"
        );
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            16000,
            "sample rate = 16kHz"
        );
        assert_eq!(
            u16::from_le_bytes(bytes[34..36].try_into().unwrap()),
            16,
            "16-bit"
        );
    }

    #[test]
    fn extrai_wav_16khz_mono_valido() {
        let Some(_) = ffmpeg_path() else { return };
        let src = temp_path("src.wav");
        let out = temp_path("out.wav");
        make_sine_wav(&src, 2);

        let (wav, duration) = extract_wav(&src, 0, &out).unwrap();
        assert_eq!(wav, out);
        assert!(
            duration.as_secs_f64().abs() - 2.0 < 0.2,
            "duração ~2s, veio {duration:?}"
        );
        assert_wav_16000_mono_s16(&out);

        // Limpeza do temp após o uso (responsabilidade do chamador).
        std::fs::remove_file(&out).unwrap();
        std::fs::remove_file(&src).unwrap();
        assert!(!out.exists());
    }

    #[test]
    fn arquivo_inexistente_retorna_erro_tipado() {
        let Some(_) = ffmpeg_path() else { return };
        let out = temp_path("inexistente-out.wav");
        let err = extract_wav(&temp_path("nao-existe.mp4"), 0, &out).unwrap_err();
        assert!(
            matches!(err, AudioError::Exit { .. }),
            "esperava Exit, veio {err}"
        );
        std::fs::remove_file(&out).ok();
    }

    #[test]
    fn arquivo_corrompido_retorna_erro_tipado() {
        let Some(_) = ffmpeg_path() else { return };
        let src = temp_path("corrompido.mp4");
        let out = temp_path("corrompido-out.wav");
        std::fs::write(&src, b"isto nao e um video valido").unwrap();

        let err = extract_wav(&src, 0, &out).unwrap_err();
        assert!(
            matches!(err, AudioError::Exit { .. }),
            "esperava Exit, veio {err}"
        );
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&out).ok();
    }

    /// Vídeo com 2 trilhas de áudio PCM (sem áudio) — não expõe.
    #[test]
    fn parse_duration_de_stderr() {
        let stderr = "  Duration: 00:01:30.25, start: 0.000000, bitrate: 706 kb/s\n";
        let d = parse_duration(stderr).unwrap();
        assert_eq!(d.as_secs_f64(), 90.25);
        assert!(parse_duration("no duration here").is_none());
        assert!(parse_duration("Duration: N/A").is_none());
    }

    /// Cria um mkv com 1 stream de VÍDEO (índice global 0) + 2 de áudio
    /// (índices globais 1 e 2) — cenário que expunha o bug de `-map 0:a:N`.
    fn make_video_with_two_audio(path: &Path) {
        let bin = ffmpeg_path().expect("fixture exige ffmpeg sidecar");
        let out = Command::new(bin)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:d=1:r=5",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:duration=1",
                "-map",
                "0:v",
                "-map",
                "1:a",
                "-map",
                "2:a",
                "-c:v",
                "rawvideo",
                "-c:a",
                "pcm_s16le",
                "-f",
                "matroska",
                &path.to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "fixture vídeo+áudio falhou: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn seleciona_trilha_por_indice_global_em_video_com_video_e_audio() {
        let Some(_) = ffmpeg_path() else { return };
        let src = temp_path("v-a.mkv");
        make_video_with_two_audio(&src);

        // Índice global da 2ª trilha de áudio = 2 (vídeo 0, áudio 1, áudio 2).
        // Com o código antigo (`-map 0:a:2`) isso falhava: só existem 2 áudios.
        let out1 = temp_path("out1.wav");
        let out2 = temp_path("out2.wav");
        let (_, _) = extract_wav(&src, 1, &out1).unwrap();
        let (_, _) = extract_wav(&src, 2, &out2).unwrap();
        assert_wav_16000_mono_s16(&out1);
        assert_wav_16000_mono_s16(&out2);

        std::fs::remove_file(&out1).ok();
        std::fs::remove_file(&out2).ok();
        std::fs::remove_file(&src).ok();
    }
}
