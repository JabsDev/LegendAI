//! Comandos IPC de informações do aplicativo (tarefa 4.8).
//!
//! `get_app_info` expõe à UI dados estáveis usados pelo dialog de "erro
//! inesperado": o caminho do diretório de log (para o usuário copiar/consultar)
//! e a versão do app (para pré-preencher a issue do GitHub).

use serde::Serialize;

/// Informações do aplicativo para a UI (serde/IPC).
#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    /// Caminho do diretório de logs (`dirs`-platforma; vazio se indisponível).
    pub log_path: String,
    /// Versão semântica do app (do Cargo.toml).
    pub version: String,
}

/// Devolve dados do app para a UI (log path + versão).
#[tauri::command(rename_all = "snake_case")]
pub fn get_app_info() -> Result<AppInfo, String> {
    let log_path = crate::logging::log_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(AppInfo {
        log_path,
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_inclui_versao_e_log_path() {
        let info = get_app_info().unwrap();
        assert!(!info.version.is_empty(), "versão deve vir do Cargo.toml");
        // No CI/headless o log dir pode não existir, mas o comando não falha.
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }
}
