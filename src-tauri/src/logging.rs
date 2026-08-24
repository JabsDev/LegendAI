use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

const LOG_PREFIX: &str = "legendai";
const LOG_RETENTION_DEFAULT: usize = 7; // dias de rotação retidos

/// Mantém vivo o worker thread do appender de arquivo (senão eventos são perdidos).
static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Diretório de logs por plataforma. `dirs` não expõe `log_dir()` — usa
/// `state_dir()` (análogo XDG no Linux) com fallback para `data_local_dir()`
/// (Windows/macOS), ambos sob `legendai/`.
///
/// `pub` porque o comando `get_app_info` (4.8) o expõe à UI para o dialog de
/// "erro inesperado" (caminho do log).
pub fn log_dir() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|d| d.join("legendai"))
}

/// Retenção em dias: env `LEGENDAI_LOG_RETENTION` ou default 7.
fn retention_days() -> usize {
    std::env::var("LEGENDAI_LOG_RETENTION")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(LOG_RETENTION_DEFAULT)
}

/// Cria o appender de arquivo com rotação diária e retenção configurável.
/// `None` se o diretório de log não puder ser criado/gravado (fallback stdout).
fn file_appender() -> Option<(tracing_appender::non_blocking::NonBlocking, WorkerGuard)> {
    let dir = log_dir()?;
    if std::fs::create_dir_all(&dir).is_err() {
        eprintln!(
            "aviso: diretório de log não gravável ({:?}) — usando stdout",
            dir
        );
        return None;
    }
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_PREFIX)
        .max_log_files(retention_days())
        .build(&dir)
        .ok()?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    Some((writer, guard))
}

/// Inicializa o logger global (idempotente): arquivo (sempre) + stdout (dev).
///
/// Nível via `RUST_LOG` (default `info`); rotação diária e retenção via
/// `LEGENDAI_LOG_RETENTION`. Log é local apenas — sem telemetria (princípio offline).
/// Se `log_dir` não for gravável, degrada para stdout sem crash.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let registry = tracing_subscriber::registry().with(filter);

    #[cfg(debug_assertions)]
    let registry = registry.with(fmt::Layer::new().with_ansi(true));

    match file_appender() {
        Some((writer, guard)) => {
            let _ = LOG_GUARD.set(guard);
            registry
                .with(fmt::Layer::new().with_ansi(false).with_writer(writer))
                .init();
        }
        None => {
            #[cfg(not(debug_assertions))]
            let registry = registry.with(fmt::Layer::new().with_ansi(true));
            registry.init();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_usa_env_quando_valido() {
        std::env::set_var("LEGENDAI_LOG_RETENTION", "30");
        assert_eq!(retention_days(), 30);
        std::env::set_var("LEGENDAI_LOG_RETENTION", "abc");
        assert_eq!(retention_days(), LOG_RETENTION_DEFAULT);
        std::env::remove_var("LEGENDAI_LOG_RETENTION");
    }

    #[test]
    fn log_dir_aponta_para_legendai() {
        let dir = log_dir().expect("dirs::log_dir() deve existir");
        assert_eq!(
            dir.file_name().map(|s| s.to_string_lossy().into_owned()),
            Some("legendai".into())
        );
    }

    #[test]
    fn arquivo_de_log_criado_com_eventos() {
        use std::io::Write;
        let Some((mut writer, guard)) = file_appender() else {
            eprintln!("log dir não disponível — pulando teste");
            return;
        };
        writeln!(writer, "evento de teste").unwrap();
        drop(writer);
        drop(guard); // encerra o worker e libera o buffer

        let dir = log_dir().unwrap();
        let files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(LOG_PREFIX))
            .collect();
        assert!(
            !files.is_empty(),
            "nenhum arquivo de log criado em {:?}",
            dir
        );
    }

    #[test]
    fn rust_log_controla_nivel_sem_recompilar() {
        std::env::set_var("RUST_LOG", "debug");
        let filter = EnvFilter::try_from_default_env().expect("RUST_LOG válido");
        assert_eq!(
            filter.max_level_hint(),
            Some(tracing_subscriber::filter::LevelFilter::DEBUG)
        );
        std::env::remove_var("RUST_LOG");
    }
}
