use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use tracing::debug;

use crate::audio::ffmpeg_extract::AudioError;
use crate::ffmpeg::{self, FFPROBE};

/// Trilha de áudio detectada por ffprobe em um vídeo.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AudioTrack {
    pub index: u32,
    pub codec: String,
    pub lang: Option<String>,
    pub channels: u32,
    pub default: bool,
}

/// Stream de legenda de TEXTO detectada por ffprobe em um vídeo (codecs
/// srt/ass/webvtt/text). Streams de bitmap (pgs/dvd) são excluídas — não são
/// extraíveis como SRT sem OCR.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SubtitleStream {
    pub index: u32,
    pub codec: String,
    pub lang: Option<String>,
    pub default: bool,
}

#[derive(Debug, Deserialize)]
struct StreamsJson {
    streams: Vec<Stream>,
}

#[derive(Debug, Deserialize)]
struct Stream {
    index: u32,
    #[serde(rename = "codec_type")]
    codec_type: String,
    #[serde(default, rename = "codec_name")]
    codec_name: String,
    #[serde(default)]
    channels: u32,
    #[serde(default)]
    disposition: Option<Disposition>,
    #[serde(default)]
    tags: Option<Tags>,
}

#[derive(Debug, Deserialize)]
struct Disposition {
    #[serde(default, deserialize_with = "de_int_bool")]
    default: bool,
}

#[derive(Debug, Deserialize)]
struct Tags {
    #[serde(default, rename = "language")]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FormatJson {
    format: Format,
}

#[derive(Debug, Deserialize)]
struct Format {
    #[serde(default)]
    duration: Option<String>,
}

/// ffprobe serializa `disposition.default` como 0/1; serde não converte
/// int→bool sozinho.
fn de_int_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(u8::deserialize(deserializer)? != 0)
}

/// Lista as trilhas de áudio de `video_path` via ffprobe.
/// Vídeo sem áudio retorna lista vazia (não é erro).
#[allow(dead_code)] // consumida pelo pipeline (1.9) e comandos IPC
pub fn list_audio_tracks(video_path: &Path) -> Result<Vec<AudioTrack>, AudioError> {
    let json = probe_json(video_path, &["-show_streams"])?;
    let parsed: StreamsJson =
        serde_json::from_str(&json).map_err(|e| AudioError::Json(e.to_string()))?;
    Ok(parsed
        .streams
        .into_iter()
        .filter(|s| s.codec_type == "audio")
        .map(|s| AudioTrack {
            index: s.index,
            codec: s.codec_name,
            lang: s.tags.and_then(|t| t.language),
            channels: s.channels,
            default: s.disposition.map(|d| d.default).unwrap_or(false),
        })
        .collect())
}

/// Lista as streams de legenda de TEXTO de `video_path` via ffprobe.
/// Vídeo sem legenda retorna lista vazia (não é erro).
///
/// `index` é o índice GLOBAL da stream no container (mesmo usado por
/// `-map 0:<index>` do ffmpeg — ver `pipeline::embedded`).
#[allow(dead_code)] // consumida pelo pipeline 3.9 e comandos IPC (4.2)
pub fn list_subtitle_tracks(video_path: &Path) -> Result<Vec<SubtitleStream>, AudioError> {
    let json = probe_json(video_path, &["-show_streams"])?;
    let parsed: StreamsJson =
        serde_json::from_str(&json).map_err(|e| AudioError::Json(e.to_string()))?;
    Ok(parsed
        .streams
        .into_iter()
        .filter(|s| s.codec_type == "subtitle")
        .filter(|s| is_text_subtitle_codec(&s.codec_name))
        .map(|s| SubtitleStream {
            index: s.index,
            codec: s.codec_name,
            lang: s.tags.and_then(|t| t.language),
            default: s.disposition.map(|d| d.default).unwrap_or(false),
        })
        .collect())
}

/// Codecs de legenda de texto (conversíveis para SRT pelo ffmpeg). "SRT como
/// padrão, ASS como opcional" (nota da 3.9): bitmap (`hdmv_pgs_subtitle`,
/// `dvd_subtitle`) ficam de fora — precisariam de OCR.
fn is_text_subtitle_codec(codec: &str) -> bool {
    matches!(
        codec,
        "subrip" | "srt" | "ass" | "ssa" | "webvtt" | "text" | "mov_text"
    )
}

/// Duração do vídeo em segundos (via `format.duration` do ffprobe), ou
/// `None` se o arquivo não expuser duração.
#[allow(dead_code)] // consumida pelo pipeline (1.9) e comandos IPC
pub fn probe_duration(video_path: &Path) -> Result<Option<Duration>, AudioError> {
    let json = probe_json(video_path, &["-show_format"])?;
    let parsed: FormatJson =
        serde_json::from_str(&json).map_err(|e| AudioError::Json(e.to_string()))?;
    Ok(parsed
        .format
        .duration
        .as_deref()
        .and_then(|d| d.trim().parse::<f64>().ok())
        .map(Duration::from_secs_f64))
}

/// Executa `ffprobe -v quiet -print_format json <extra...> <video>` e
/// devolve o stdout em caso de sucesso.
fn probe_json(video_path: &Path, extra: &[&str]) -> Result<String, AudioError> {
    let bin = ffmpeg::binary_path(FFPROBE)?;
    let video = video_path.to_string_lossy();
    let mut args: Vec<&str> = vec!["-v", "quiet", "-print_format", "json"];
    args.extend_from_slice(extra);
    args.push(&video);

    let output = Command::new(&bin)
        .args(&args)
        .output()
        .map_err(|source| AudioError::Spawn {
            command: bin.clone(),
            source,
        })?;

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    debug!("stderr do ffprobe:\n{stderr}");

    if !output.status.success() {
        return Err(AudioError::Exit {
            code: output.status.code().unwrap_or(-1),
            stderr,
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ffprobe_path() -> Option<PathBuf> {
        // Binários são download (gitignored) — se ausentes, skip sem falhar (CI).
        ffmpeg::binary_path(FFPROBE).ok()
    }

    /// Fixtures são geradas com o binário ffmpeg (não ffprobe).
    fn ffmpeg_path() -> Option<PathBuf> {
        ffmpeg::binary_path(ffmpeg::FFMPEG).ok()
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("legendai-test-{}-{name}", std::process::id()))
    }

    /// Muxa dois senos em um mkv com 2 trilhas de áudio PCM (por/eng) —
    /// sem rede e sem commitar binário no repo.
    fn make_multi_track(path: &Path) {
        let bin = ffmpeg_path().expect("fixture exige ffmpeg sidecar");
        let out = Command::new(bin)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:duration=1",
                "-map",
                "0:a",
                "-map",
                "1:a",
                "-c:a",
                "pcm_s16le",
                "-metadata:s:a:0",
                "language=por",
                "-metadata:s:a:1",
                "language=eng",
                "-disposition:a:0",
                "default",
                "-f",
                "matroska",
                &path.to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "fixture multi-trilha falhou: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Vídeo sem áudio (rawvideo em mkv) — exercita o filtro de codec_type.
    fn make_no_audio(path: &Path) {
        let bin = ffmpeg_path().expect("fixture exige ffmpeg sidecar");
        let out = Command::new(bin)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:d=1:r=5",
                "-c:v",
                "rawvideo",
                "-f",
                "matroska",
                &path.to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "fixture sem áudio falhou: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Muxa um SRT de texto em um mkv (vídeo lavfi + stream de legenda subrip).
    fn make_embedded_srt(path: &Path) {
        let bin = ffmpeg_path().expect("fixture exige ffmpeg sidecar");
        let srt = temp_path("sub.srt");
        std::fs::write(
            &srt,
            "1\n00:00:00,000 --> 00:00:02,000\nOlá, mundo!\n\n\
             2\n00:00:02,500 --> 00:00:04,500\nSegunda legenda.\n",
        )
        .unwrap();
        let out = Command::new(bin)
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:d=8:r=5",
                "-f",
                "srt",
                "-i",
                &srt.to_string_lossy(),
                "-map",
                "0:v",
                "-map",
                "1:s",
                "-c:v",
                "rawvideo",
                "-c:s",
                "srt",
                "-metadata:s:s:0",
                "language=por",
                "-f",
                "matroska",
                &path.to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "fixture com legenda falhou: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::remove_file(&srt).ok();
    }

    #[test]
    fn lista_streams_de_legenda_embutida() {
        let Some(_) = ffprobe_path() else { return };
        let src = temp_path("legenda.mkv");
        make_embedded_srt(&src);

        let tracks = list_subtitle_tracks(&src).unwrap();
        std::fs::remove_file(&src).ok();

        assert_eq!(tracks.len(), 1, "esperava 1 stream, veio {tracks:?}");
        let t = &tracks[0];
        assert_eq!(t.index, 1, "vídeo no índice global 0, legenda no índice 1");
        assert_eq!(t.codec, "subrip", "ffmpeg reporta srt como subrip");
        assert_eq!(t.lang.as_deref(), Some("por"));
    }

    #[test]
    fn video_sem_legenda_retorna_lista_vazia() {
        let Some(_) = ffprobe_path() else { return };
        let src = temp_path("sem-legenda.mkv");
        make_no_audio(&src);

        let tracks = list_subtitle_tracks(&src).unwrap();
        std::fs::remove_file(&src).ok();

        assert!(tracks.is_empty(), "esperava vazio, veio {tracks:?}");
    }

    #[test]
    fn lista_trilhas_de_video_multi_audio() {
        let Some(_) = ffprobe_path() else { return };
        let src = temp_path("multi.mkv");
        make_multi_track(&src);

        let tracks = list_audio_tracks(&src).unwrap();
        std::fs::remove_file(&src).ok();

        assert_eq!(tracks.len(), 2, "esperava 2 trilhas, veio {tracks:?}");
        let por = tracks
            .iter()
            .find(|t| t.lang.as_deref() == Some("por"))
            .expect("trilha por");
        assert_eq!(por.index, 0);
        assert_eq!(por.codec, "pcm_s16le");
        assert_eq!(por.channels, 1);
        assert!(por.default, "primeira trilha deve ser a default");

        let eng = tracks
            .iter()
            .find(|t| t.lang.as_deref() == Some("eng"))
            .expect("trilha eng");
        assert_eq!(eng.index, 1);
        assert_eq!(eng.codec, "pcm_s16le");
        assert_eq!(eng.channels, 1);
        assert!(!eng.default);
    }

    #[test]
    fn video_sem_audio_retorna_lista_vazia() {
        let Some(_) = ffprobe_path() else { return };
        let src = temp_path("sem-audio.mkv");
        make_no_audio(&src);

        let tracks = list_audio_tracks(&src).unwrap();
        std::fs::remove_file(&src).ok();

        assert!(tracks.is_empty(), "esperava lista vazia, veio {tracks:?}");
    }

    #[test]
    fn arquivo_inexistente_retorna_erro_tipado() {
        let Some(_) = ffprobe_path() else { return };
        let err = list_audio_tracks(&temp_path("nao-existe.mp4")).unwrap_err();
        assert!(
            matches!(err, AudioError::Exit { .. }),
            "esperava Exit, veio {err}"
        );
    }

    #[test]
    fn probe_duration_le_duracao_do_arquivo() {
        let Some(_) = ffprobe_path() else { return };
        let src = temp_path("dur.mkv");
        make_multi_track(&src);

        let d = probe_duration(&src).unwrap().expect("duração presente");
        std::fs::remove_file(&src).ok();

        assert!(
            (d.as_secs_f64() - 1.0).abs() < 0.2,
            "duração ~1s, veio {d:?}"
        );
    }
}
