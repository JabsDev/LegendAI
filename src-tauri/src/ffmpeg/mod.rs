use std::path::PathBuf;

use tauri_plugin_shell::{process::CommandEvent, ShellExt};
use thiserror::Error;

pub const FFMPEG: &str = "ffmpeg";
pub const FFPROBE: &str = "ffprobe";

#[derive(Debug, Error)]
pub enum FfmpegError {
    #[error("sidecar `{0}` não encontrado — em dev coloque em src-tauri/binaries/, em produção o bundle deve incluí-lo")]
    NotFound(String),
    #[error("falha ao resolver target triple: {0}")]
    Triple(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("falha ao executar `{0}`: {1}")]
    Spawn(String, String),
    #[error("`{0}` saiu com status {1}: {2}")]
    Exit(String, i32, String),
}

/// Nome do arquivo sidecar com sufixo do target triple (ex: `ffmpeg-x86_64-unknown-linux-gnu`).
/// No Windows o arquivo tem extensão `.exe` (exigido pelo Tauri para sidecars Windows).
fn sidecar_file(name: &str) -> Result<PathBuf, FfmpegError> {
    let triple =
        tauri::utils::platform::target_triple().map_err(|e| FfmpegError::Triple(Box::new(e)))?;
    let mut file = format!("{name}-{triple}");
    if triple.contains("windows") {
        file.push_str(".exe");
    }
    Ok(PathBuf::from(file))
}

/// Resolve o caminho do binário sidecar.
/// Dev: `src-tauri/binaries/<name>-<triple>[.exe]` (fonte).
/// Prod: `exe_dir/<name>[.exe]` (o tauri extrai o sidecar ao lado do executável, sem sufixo de triple mas mantendo .exe no Windows).
pub fn binary_path(name: &str) -> Result<PathBuf, FfmpegError> {
    if tauri::is_dev() {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(sidecar_file(name)?);
        if dev.exists() {
            return Ok(dev);
        }
        // Fallback: tentar sem/com .exe para compatibilidade com arquivos legados
        let alt = if dev.extension().is_some() {
            dev.with_extension("")
        } else {
            dev.with_extension("exe")
        };
        if alt.exists() {
            return Ok(alt);
        }
    }

    // Produção: sidecar é extraído pelo Tauri ao lado do executável
    // (sem sufixo de triple, mas com .exe no Windows). `current_exe` cobre
    // NSIS (Windows), deb/AppImage (Linux) e .app/Contents/MacOS (macOS).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
            // macOS/Linux: em alguns bundles o recurso pode estar em
            // `../Resources` relativo ao executável — tenta também.
            for candidate in [
                dir.join(format!("../Resources/{name}")),
                dir.join(format!("../Resources/{name}.exe")),
            ] {
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
    }

    // Último fallback dev: tentar binário do sistema no PATH (ex: ffmpeg
    // instalado via apt/brew) para não bloquear totalmente o dev sem sidecar.
    // Em produção o sidecar do bundle é obrigatório — este fallback não
    // deve ser alcançado no instalador.
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for candidate in [dir.join(name), dir.join(format!("{name}.exe"))] {
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }
    }

    Err(FfmpegError::NotFound(name.to_string()))
}

/// Executa um sidecar via `tauri-plugin-shell` (sem shell intermediário — ver ADR-003)
/// e devolve stdout em caso de sucesso.
pub async fn run<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    name: &str,
    args: &[&str],
) -> Result<String, FfmpegError> {
    let (mut rx, _child) = app
        .shell()
        .sidecar(name)
        .map_err(|e| FfmpegError::Spawn(name.into(), e.to_string()))?
        .args(args)
        .spawn()
        .map_err(|e| FfmpegError::Spawn(name.into(), e.to_string()))?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut code = None;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) => stdout.push_str(&String::from_utf8_lossy(&bytes)),
            CommandEvent::Stderr(bytes) => stderr.push_str(&String::from_utf8_lossy(&bytes)),
            CommandEvent::Error(e) => return Err(FfmpegError::Spawn(name.into(), e)),
            CommandEvent::Terminated(payload) => {
                code = payload.code;
                break;
            }
            _ => {}
        }
    }

    match code {
        Some(0) => Ok(stdout),
        Some(c) => Err(FfmpegError::Exit(name.into(), c, stderr)),
        None => Err(FfmpegError::Spawn(
            name.into(),
            "processo terminou sem código de saída".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_file_tem_sufixo_do_triple() {
        let file = sidecar_file(FFMPEG).unwrap();
        let name = file.to_string_lossy();
        assert!(
            name.starts_with("ffmpeg-"),
            "esperava `ffmpeg-...`, veio {name}"
        );
        let triple = name.trim_start_matches("ffmpeg-");
        assert!(!triple.is_empty());
        assert!(
            triple.contains('-'),
            "triple deve ter a forma arch-os-env, veio {triple}"
        );
    }

    #[test]
    fn binary_path_resolve_em_dev() {
        // Binários são download (gitignored) — se ausentes, skip sem falhar (CI).
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(sidecar_file(FFMPEG).unwrap());
        if !dev.exists() {
            return;
        }
        assert_eq!(binary_path(FFMPEG).unwrap(), dev);
    }
}
