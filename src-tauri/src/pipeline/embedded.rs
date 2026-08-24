//! Pipeline de legenda embutida (tarefa 3.9): quando o vídeo tem uma stream de
//! legenda de TEXTO embutida (srt/ass/webvtt), ela pode ser usada como origem —
//! **pulando o STT** — e o fluxo segue para formatação/tradução (3.10).
//!
//! Fluxo: `list_subtitle_tracks` (1.2 estendida) detecta as streams → aqui,
//! `extract_subtitle` extrai a escolhida para um arquivo temporário com o ffmpeg
//! (convertendo ASS/WebVTT para SRT) → `load_embedded_subtitle` usa o `parse_srt`
//! (1.7) e devolve as legendas do domínio.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use thiserror::Error;
use tracing::debug;

use crate::audio::ffmpeg_extract::AudioError;
use crate::domain::Subtitle;
use crate::ffmpeg::{self, FFMPEG};
use crate::subtitles::srt::{parse_srt, SrtError};

/// Erros do pipeline de legenda embutida.
#[derive(Debug, Error)]
pub enum EmbeddedError {
    #[error(transparent)]
    FfmpegSidecar(#[from] crate::ffmpeg::FfmpegError),
    #[error(transparent)]
    Ffmpeg(#[from] AudioError),
    #[error(transparent)]
    Srt(#[from] SrtError),
    #[error("falha ao ler legenda extraída em `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Extrai a stream de legenda `stream_index` (índice GLOBAL do ffprobe, ver
/// [`crate::audio::ffprobe::list_subtitle_tracks`]) de `video_path` para
/// `out_path`.
///
/// Usa `-map 0:<index>` (índice global do container, o mesmo que o ffprobe
/// reporta em `SubtitleStream::index`) em vez de `-map 0:s:<n>` — este último é
/// relativo ao tipo e divergiria quando a stream de legenda não é a primeira.
/// `out_path` deve terminar em `.srt`: o ffmpeg define o formato de saída pela
/// extensão e converte ASS/WebVTT para SRT no processo. Streams de bitmap falham
/// com erro tipado.
pub fn extract_subtitle(
    video_path: &Path,
    stream_index: u32,
    out_path: &Path,
) -> Result<(), EmbeddedError> {
    let bin = ffmpeg::binary_path(FFMPEG)?;
    let video = video_path.to_string_lossy();
    let map = format!("0:{stream_index}");
    let out = out_path.to_string_lossy();
    let output = Command::new(&bin)
        .args(["-y", "-i", &video, "-map", &map, &out])
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
        }
        .into());
    }
    Ok(())
}

/// Extrai a stream `stream_index` para um arquivo temporário e parseia como SRT,
/// devolvendo as legendas do domínio. É o "pula STT": o vídeo com legenda
/// embutida entra aqui e sai como `Vec<Subtitle>` pronto para formatação (1.8)
/// e/ou tradução (3.10). O arquivo temporário é removido ao final.
pub fn load_embedded_subtitle(
    video_path: &Path,
    stream_index: u32,
) -> Result<Vec<Subtitle>, EmbeddedError> {
    let tmp = std::env::temp_dir().join(format!(
        "legendai-embedded-{}-{stream_index}-{}.srt",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    let result = (|| {
        extract_subtitle(video_path, stream_index, &tmp)?;
        let srt = std::fs::read_to_string(&tmp).map_err(|source| EmbeddedError::Io {
            path: tmp.clone(),
            source,
        })?;
        parse_srt(&srt).map_err(Into::into)
    })();
    std::fs::remove_file(&tmp).ok();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::ffprobe::list_subtitle_tracks;
    use crate::domain::Timestamp;

    fn ffmpeg_path() -> Option<PathBuf> {
        ffmpeg::binary_path(FFMPEG).ok()
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("legendai-test-{}-{name}", std::process::id()))
    }

    fn srt_fixture() -> &'static str {
        "1\n00:00:00,000 --> 00:00:02,000\nOlá, mundo!\n\n\
         2\n00:00:02,500 --> 00:00:04,500\nSegunda legenda.\n\n\
         3\n00:00:05,000 --> 00:00:07,000\nTerceira, com acento.\n"
    }

    fn expected() -> Vec<Subtitle> {
        parse_srt(srt_fixture()).unwrap()
    }

    /// Muxa um SRT de texto em um mkv (vídeo lavfi no índice global 0, legenda
    /// no índice global 1) — sem rede e sem commitar binário no repo. O nome do
    /// arquivo SRT deriva do stem do alvo para evitar race entre testes
    /// paralelos (mesmo processo).
    fn make_embedded_srt(path: &Path) {
        let bin = ffmpeg_path().expect("fixture exige ffmpeg sidecar");
        let stem = path.file_stem().unwrap().to_string_lossy();
        let srt = temp_path(&format!("{stem}-sub.srt"));
        std::fs::write(&srt, srt_fixture()).unwrap();
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

    /// Muxa um ASS de texto em um mkv — cobre o "ASS como opcional" da nota.
    fn make_embedded_ass(path: &Path) {
        let bin = ffmpeg_path().expect("fixture exige ffmpeg sidecar");
        let stem = path.file_stem().unwrap().to_string_lossy();
        let ass = temp_path(&format!("{stem}-sub.ass"));
        std::fs::write(
            &ass,
            "[Script Info]\nScriptType: v4.00+\n\n\
             [Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
             Dialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,Olá, mundo!\n",
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
                "ass",
                "-i",
                &ass.to_string_lossy(),
                "-map",
                "0:v",
                "-map",
                "1:s",
                "-c:v",
                "rawvideo",
                "-c:s",
                "ass",
                "-f",
                "matroska",
                &path.to_string_lossy(),
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "fixture ASS falhou: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        std::fs::remove_file(&ass).ok();
    }

    #[test]
    fn extrai_legenda_embutida_preservando_timings() {
        let Some(_) = ffmpeg_path() else { return };
        let src = temp_path("emb.mkv");
        let out = temp_path("emb-out.srt");
        make_embedded_srt(&src);

        let tracks = list_subtitle_tracks(&src).unwrap();
        assert_eq!(tracks.len(), 1, "detecção deve achar a legenda embutida");
        extract_subtitle(&src, tracks[0].index, &out).unwrap();
        let parsed = parse_srt(&std::fs::read_to_string(&out).unwrap()).unwrap();

        std::fs::remove_file(&out).ok();
        std::fs::remove_file(&src).ok();

        // Critério 3.9: timings preservados após o round-trip pelo pipeline.
        assert_eq!(
            parsed,
            expected(),
            "timings/textos devem sobreviver à extração"
        );
    }

    #[test]
    fn load_embedded_subtitle_pula_stt() {
        let Some(_) = ffmpeg_path() else { return };
        let src = temp_path("emb2.mkv");
        make_embedded_srt(&src);

        let subs = load_embedded_subtitle(&src, 1).unwrap();
        std::fs::remove_file(&src).ok();

        assert_eq!(subs, expected());
    }

    #[test]
    fn extrai_ass_convertendo_para_srt() {
        let Some(_) = ffmpeg_path() else { return };
        let src = temp_path("emb-ass.mkv");
        let out = temp_path("emb-ass-out.srt");
        make_embedded_ass(&src);

        let tracks = list_subtitle_tracks(&src).unwrap();
        assert_eq!(tracks[0].codec, "ass");
        extract_subtitle(&src, tracks[0].index, &out).unwrap();
        let parsed = parse_srt(&std::fs::read_to_string(&out).unwrap()).unwrap();

        std::fs::remove_file(&out).ok();
        std::fs::remove_file(&src).ok();

        let seg = &parsed[0].segments[0];
        assert_eq!(seg.text, "Olá, mundo!");
        assert_eq!(seg.start_ms, Timestamp::from_ms(0));
        assert_eq!(seg.end_ms, Timestamp::from_ms(2000));
    }

    #[test]
    fn stream_inexistente_retorna_erro_tipado() {
        let Some(_) = ffmpeg_path() else { return };
        let src = temp_path("emb3.mkv");
        let out = temp_path("emb3-out.srt");
        make_embedded_srt(&src);

        let err = extract_subtitle(&src, 99, &out).unwrap_err();
        std::fs::remove_file(&src).ok();
        std::fs::remove_file(&out).ok();

        assert!(
            matches!(err, EmbeddedError::Ffmpeg(AudioError::Exit { .. })),
            "map de stream inexistente deve falhar tipado, veio {err}"
        );
    }
}
