//! Smoke test (tarefa 6.8): `legendai --smoke-test` verifica, sem rede e sem
//! abrir a GUI, a integridade de uma instalação — o sidecar ffmpeg responde
//! (`ffmpeg -version`), a config carrega e o estado de onboarding é
//! calculável (sem baixar modelos). Imprime um check por linha e sai com
//! código 0 (ok) ou 1 (falha): usado no CI pós-release
//! (`.github/workflows/smoke.yml`) e como diagnóstico de usuário ("rode com
//! `--smoke-test`" em issues — ver docs/INSTALL.md).
//!
//! O teste roda fora do runtime Tauri (sem `AppHandle`): o ffmpeg é invocado
//! com `std::process::Command` sobre o caminho de [`crate::ffmpeg::binary_path`],
//! mesmo padrão do módulo de extração de áudio (1.1) — sem shell intermediário
//! e sem risco de command injection (ADR-003).

use std::path::PathBuf;
use std::process::Command;

use crate::commands::onboarding::get_onboarding;
use crate::config::AppConfig;
use crate::ffmpeg::{self, FFMPEG};

/// Executa todos os checks e devolve o código de saída do processo
/// (0 = passou; qualquer outra coisa = falha).
pub fn run() -> i32 {
    let mut code = 0;
    for (name, result) in [
        ("ffmpeg sidecar", check_ffmpeg()),
        ("config", check_config()),
        ("onboarding", check_onboarding()),
    ] {
        match result {
            Ok(detail) => say(&format!("[ok]   {name}: {detail}")),
            Err(e) => {
                say(&format!("[FAIL] {name}: {e}"));
                code = 1;
            }
        }
    }
    say(&format!(
        "smoke test: {}",
        if code == 0 { "PASS" } else { "FAIL" }
    ));
    code
}

/// Resolve e executa `ffmpeg -version`, devolvendo a primeira linha da saída.
fn check_ffmpeg() -> Result<String, String> {
    let bin = resolve_ffmpeg()?;
    let out = Command::new(&bin)
        .arg("-version")
        .output()
        .map_err(|e| format!("falha ao executar `{}`: {e}", bin.display()))?;
    if !out.status.success() {
        return Err(format!(
            "`ffmpeg -version` saiu com status {}",
            out.status.code().unwrap_or(-1)
        ));
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .to_string();
    if line.starts_with("ffmpeg version") {
        Ok(line)
    } else {
        Err(format!("saída inesperada de `ffmpeg -version`: {line:?}"))
    }
}

/// Carrega a config persistida (ausente/corrompida → defaults sem crash).
fn check_config() -> Result<String, String> {
    let cfg = AppConfig::load_or_default();
    Ok(format!(
        "schema {}, origem {}→destino {}",
        cfg.schema_version, cfg.source_lang, cfg.target_lang
    ))
}

/// Calcula o estado de onboarding (hardware + tier + recomendações) — valida
/// de quebra o catálogo embutido, que é re-validado no boot do app.
fn check_onboarding() -> Result<String, String> {
    let info = get_onboarding().map_err(|e| e.to_string())?;
    Ok(format!(
        "first_run={}, tier={:?}, recomendações {} STT + {} tradução",
        info.first_run,
        info.tier,
        info.recommendations.stt.len(),
        info.recommendations.translation.len()
    ))
}

/// Resolve o caminho do sidecar ffmpeg via [`crate::ffmpeg::binary_path`]. Em
/// produção no Windows o Tauri extrai o sidecar como `ffmpeg.exe`, e
/// `binary_path` procura sem extensão — fallback explícito para o caso.
fn resolve_ffmpeg() -> Result<PathBuf, String> {
    let path = ffmpeg::binary_path(FFMPEG).map_err(|e| e.to_string())?;
    if path.exists() {
        return Ok(path);
    }
    let exe = PathBuf::from(format!("{}.exe", path.display()));
    if exe.exists() {
        return Ok(exe);
    }
    Err(format!(
        "sidecar `{FFMPEG}` não encontrado (esperado em {})",
        path.display()
    ))
}

/// Escreve uma linha em stdout sem panic quando não há console anexado
/// (o release Windows é GUI subsystem — `println!` poderia panificar).
fn say(msg: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_ffmpeg_ok_quando_sidecar_presente() {
        // Binários são download (gitignored) — se ausentes, skip sem falhar (CI).
        let present = ffmpeg::binary_path(FFMPEG)
            .map(|p| p.exists())
            .unwrap_or(false);
        if present {
            let line = check_ffmpeg().expect("ffmpeg presente deve responder");
            assert!(line.starts_with("ffmpeg version"), "veio {line}");
        }
    }

    #[test]
    fn smoke_ok_quando_sidecar_presente() {
        let present = ffmpeg::binary_path(FFMPEG)
            .map(|p| p.exists())
            .unwrap_or(false);
        if present {
            assert_eq!(run(), 0);
        }
    }
}
