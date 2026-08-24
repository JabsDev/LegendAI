//! Comandos IPC da importação de vídeo (tarefa 4.2).
//!
//! `inspect_video` inspeciona um arquivo de vídeo local via ffprobe (1.2) e
//! devolve à UI as informações necessárias para o passo "escolher trilha de
//! áudio / legenda embutida": duração, trilhas de áudio e streams de legenda
//! de texto. O frontend envia o caminho obtido pelo dialog do Tauri ou pelo
//! drag-and-drop (não upload de bytes).

use std::path::Path;

use serde::Serialize;

use crate::audio::ffprobe::{
    list_audio_tracks, list_subtitle_tracks, probe_duration, AudioTrack, SubtitleStream,
};

/// Resultado da inspeção de um vídeo para a UI (serde/IPC).
#[derive(Debug, Clone, Serialize)]
pub struct VideoInspection {
    /// Caminho absoluto do vídeo inspecionado (usado pelo pipeline 4.3).
    pub path: String,
    pub file_name: String,
    /// Duração em segundos (0 se o container não expuser duração).
    pub duration_secs: f64,
    pub audio_tracks: Vec<AudioTrack>,
    pub subtitle_streams: Vec<SubtitleStream>,
}

/// Inspeciona `path` (vídeo local) e devolve duração + trilhas de áudio +
/// streams de legenda de texto para a tela de importação.
#[tauri::command(rename_all = "snake_case")]
pub fn inspect_video(path: String) -> Result<VideoInspection, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("arquivo não encontrado: `{path}`"));
    }
    let file_name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let audio_tracks = list_audio_tracks(p).map_err(|e| e.to_string())?;
    let subtitle_streams = list_subtitle_tracks(p).map_err(|e| e.to_string())?;
    let duration_secs = probe_duration(p)
        .map_err(|e| e.to_string())?
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Ok(VideoInspection {
        path,
        file_name,
        duration_secs,
        audio_tracks,
        subtitle_streams,
    })
}
