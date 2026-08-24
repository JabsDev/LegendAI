//! Comandos IPC de exportação de legendas em formatos adicionais (tarefa 5.7).
//!
//! `export_subtitle` lê um SRT de origem, converte para o formato pedido
//! (WebVTT, texto puro ou ASS — o SRT também é suportado para re-exportação
//! uniforme) e grava o arquivo ao lado da origem com a extensão do formato.
//! O frontend chama com `invoke("export_subtitle", { path, format, options })`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::domain::Subtitle;
use crate::subtitles::{parse_srt, to_ass, to_srt, to_txt, to_vtt};

/// Formato de exportação. Serializa em snake_case para a UI (`"vtt"`, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Srt,
    Vtt,
    Txt,
    Ass,
}

impl ExportFormat {
    fn ext(self) -> &'static str {
        match self {
            ExportFormat::Srt => "srt",
            ExportFormat::Vtt => "vtt",
            ExportFormat::Txt => "txt",
            ExportFormat::Ass => "ass",
        }
    }
}

/// Opções de exportação. `with_timestamps` só afeta o formato TXT (prefixa
/// cada bloco com `start --> end`); os demais formatos o ignoram.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportOptions {
    pub with_timestamps: bool,
}

/// Resultado da exportação: caminho do arquivo gravado + conteúdo gerado.
#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub content: String,
}

/// Exporta a legenda SRT em `path` para o formato pedido, gravando o arquivo
/// de saída ao lado da origem (extensão trocada). Retorna caminho e conteúdo.
#[tauri::command(rename_all = "snake_case")]
pub fn export_subtitle(
    path: String,
    format: ExportFormat,
    options: ExportOptions,
) -> Result<ExportResult, String> {
    let subs = load_subtitles(&path)?;
    let content = render(&subs, format, &options)?;
    let out_path = output_path(&path, format);
    std::fs::write(&out_path, &content)
        .map_err(|e| format!("falha ao gravar a legenda `{}`: {e}", out_path.display()))?;
    Ok(ExportResult {
        path: out_path.to_string_lossy().into_owned(),
        content,
    })
}

/// Lê e parseia o SRT de origem em `Vec<Subtitle>`.
fn load_subtitles(path: &str) -> Result<Vec<Subtitle>, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("falha ao ler a legenda `{path}`: {e}"))?;
    parse_srt(&content).map_err(|e| format!("legenda inválida em `{path}`: {e}"))
}

/// Serializa as legendas no formato pedido com as opções dadas.
fn render(
    subs: &[Subtitle],
    format: ExportFormat,
    options: &ExportOptions,
) -> Result<String, String> {
    match format {
        ExportFormat::Srt => Ok(to_srt(subs)),
        ExportFormat::Vtt => Ok(to_vtt(subs)),
        ExportFormat::Txt => Ok(to_txt(subs, options.with_timestamps)),
        ExportFormat::Ass => Ok(to_ass(subs)),
    }
}

/// Caminho de saída: mesmo diretório/nome da origem, extensão do formato.
fn output_path(path: &str, format: ExportFormat) -> PathBuf {
    Path::new(path).with_extension(format.ext())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRT: &str =
        "1\n00:00:01,000 --> 00:00:03,000\nOlá, mundo!\n\n2\n00:00:04,000 --> 00:00:06,500\nSegunda legenda.\n";

    fn write_srt(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("legendai-export-{tag}-{}.srt", std::process::id()));
        std::fs::write(&p, SRT).unwrap();
        p
    }

    #[test]
    fn exporta_vtt_valido() {
        let src = write_srt("vtt");
        let res = export_subtitle(
            src.to_str().unwrap().into(),
            ExportFormat::Vtt,
            ExportOptions::default(),
        )
        .unwrap();
        assert!(res.path.ends_with(".vtt"));
        assert!(res.content.starts_with("WEBVTT\n"));
        assert!(res.content.contains("00:00:01.000 --> 00:00:03.000"));
        let _ = std::fs::remove_file(&res.path);
        let _ = std::fs::remove_file(src);
    }

    #[test]
    fn exporta_txt_sem_timestamps_por_default() {
        let src = write_srt("txt");
        let res = export_subtitle(
            src.to_str().unwrap().into(),
            ExportFormat::Txt,
            ExportOptions::default(),
        )
        .unwrap();
        assert!(res.path.ends_with(".txt"));
        assert!(res.content.contains("Olá, mundo!"));
        assert!(!res.content.contains("-->"));
        let _ = std::fs::remove_file(&res.path);
        let _ = std::fs::remove_file(src);
    }

    #[test]
    fn exporta_txt_com_timestamps() {
        let src = write_srt("txt2");
        let res = export_subtitle(
            src.to_str().unwrap().into(),
            ExportFormat::Txt,
            ExportOptions {
                with_timestamps: true,
            },
        )
        .unwrap();
        assert!(res.content.contains("00:00:01,000 --> 00:00:03,000"));
        let _ = std::fs::remove_file(&res.path);
        let _ = std::fs::remove_file(src);
    }

    #[test]
    fn exporta_ass_com_estrutura_valida() {
        let src = write_srt("ass");
        let res = export_subtitle(
            src.to_str().unwrap().into(),
            ExportFormat::Ass,
            ExportOptions::default(),
        )
        .unwrap();
        assert!(res.path.ends_with(".ass"));
        assert!(res.content.contains("[Script Info]"));
        assert!(res.content.contains("Dialogue: "));
        let _ = std::fs::remove_file(&res.path);
        let _ = std::fs::remove_file(src);
    }

    #[test]
    fn arquivo_inexistente_retorna_erro_acaoavel() {
        let err = export_subtitle(
            "/nao/existe/legenda.srt".into(),
            ExportFormat::Vtt,
            ExportOptions::default(),
        )
        .unwrap_err();
        assert!(err.contains("ler a legenda"), "{err}");
    }
}
