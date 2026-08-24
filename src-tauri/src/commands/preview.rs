//! Comandos IPC do preview de vídeo com legenda (tarefa 4.4).
//!
//! `load_preview` valida que o vídeo e a legenda existem e devolve o conteúdo
//! do SRT para a UI exibir sincronizado sobre o `<video>` nativo. A URL
//! acessível do vídeo é montada no frontend com `convertFileSrc` (asset
//! protocol, escopo em `tauri.conf.json`); a UI converte o SRT em cues
//! (SRT→WebVTT é trivial — só trocar `,` por `.`).

use std::path::Path;

use serde::Serialize;

/// Dados carregados para o preview.
#[derive(Debug, Clone, Serialize)]
pub struct PreviewData {
    /// Texto do SRT a exibir sobre o vídeo.
    pub srt: String,
}

/// Carrega os dados do preview: valida `video_path` e `srt_path` em runtime e
/// lê o conteúdo do SRT. Erros são mensagens estáveis/acionáveis (padrão 4.8).
#[tauri::command(rename_all = "snake_case")]
pub async fn load_preview(video_path: String, srt_path: String) -> Result<PreviewData, String> {
    read_preview(&video_path, &srt_path)
}

/// Núcleo testável do comando (sem dependência de `AppHandle`).
fn read_preview(video_path: &str, srt_path: &str) -> Result<PreviewData, String> {
    if !Path::new(video_path).exists() {
        return Err(format!("arquivo de vídeo não encontrado: `{video_path}`"));
    }
    if !Path::new(srt_path).exists() {
        return Err(format!("arquivo de legenda não encontrado: `{srt_path}`"));
    }
    let srt = std::fs::read_to_string(srt_path)
        .map_err(|e| format!("falha ao ler a legenda `{srt_path}`: {e}"))?;
    Ok(PreviewData { srt })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(prefix: &str, content: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "legendai-preview-{prefix}-{}.srt",
            std::process::id()
        ));
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn carrega_srt_de_arquivo_existente() {
        let video =
            std::env::temp_dir().join(format!("legendai-preview-vid-{}", std::process::id()));
        std::fs::write(&video, "video bytes").unwrap();
        let srt = write_temp("ok", "1\n00:00:01,000 --> 00:00:02,000\nOlá\n");
        let data = read_preview(video.to_str().unwrap(), srt.to_str().unwrap()).unwrap();
        assert!(data.srt.contains("Olá"));
        let _ = std::fs::remove_file(video);
        let _ = std::fs::remove_file(srt);
    }

    #[test]
    fn video_ausente_retorna_erro_acaoavel() {
        let srt = write_temp("vid", "1\n00:00:01,000 --> 00:00:02,000\nOlá\n");
        let err = read_preview("/nao/existe/video.mp4", srt.to_str().unwrap()).unwrap_err();
        assert!(err.contains("vídeo"), "{err}");
        let _ = std::fs::remove_file(srt);
    }

    #[test]
    fn srt_ausente_retorna_erro_acaoavel() {
        let video =
            std::env::temp_dir().join(format!("legendai-preview-vid2-{}", std::process::id()));
        std::fs::write(&video, "video bytes").unwrap();
        let err = read_preview(video.to_str().unwrap(), "/nao/existe/legenda.srt").unwrap_err();
        assert!(err.contains("legenda"), "{err}");
        let _ = std::fs::remove_file(video);
    }
}
