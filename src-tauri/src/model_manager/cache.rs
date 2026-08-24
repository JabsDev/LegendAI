//! Cache local organizado de modelos (tarefa 2.4).
//!
//! Layout canônico: `cache_dir()/legendai/models/<kind>/<model_id>/` com:
//! - `status.json` — estado do modelo (`downloading` | `downloaded` | `error`),
//!   tamanho em bytes do arquivo principal e checksum SHA256 registrado;
//! - `<file>` — arquivo principal (e demais arquivos do `ModelInfo::files`);
//! - `.lock` — lockfile de download (conteúdo `PID\nunix_secs`), impedindo que
//!   dois downloads do mesmo modelo rodem em paralelo.
//!
//! O layout da 1.6 (`whisper_dir`, `models/whisper/` plano) é legado do MVP e
//! fica intacto; o caminho canônico desta tarefa é [`models_root`]/[`model_dir`].
//!
//! Integração com o download (2.2): o chamador adquire o lock com
//! [`acquire_download_lock`], baixa via [`super::download::download_model`],
//! grava `status.json` com [`write_status`] e solta o lock (Drop remove o
//! arquivo). [`resolve_model_path`] é a porta de entrada do pipeline: só retorna
//! caminho para modelo íntegro e baixado.

use std::cell::RefCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::catalog::{ModelInfo, ModelKind};

// Override de teste (thread-local, sem mutação de env global): quando setado,
// vira a raiz do cache. Permite testar layout/lock/status sem tocar no XDG
// real do usuário e sem race entre testes paralelos.
thread_local! {
    static ROOT_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Guarda de teste (unitário): restaura a raiz do cache no `Drop` — mesmo em
/// caso de panic — e remove o diretório temporário. Usada pelos testes de cache
/// e da factory de engines (3.4).
#[cfg(test)]
pub(crate) struct RootGuard {
    dir: PathBuf,
}

#[cfg(test)]
impl Drop for RootGuard {
    fn drop(&mut self) {
        ROOT_OVERRIDE.with(|o| *o.borrow_mut() = None);
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// Roda `f` com a raiz do cache redirecionada para um diretório temporário
/// (thread-local, sem mutar env global — seguro sob testes paralelos).
#[cfg(test)]
pub(crate) fn with_root(tag: &str, f: impl FnOnce()) {
    let dir = std::env::temp_dir().join(format!("legendai-cache-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ROOT_OVERRIDE.with(|o| *o.borrow_mut() = Some(dir.clone()));
    let _guard = RootGuard { dir };
    f();
}

/// Estado persistido de um modelo no cache (`status.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheStatus {
    Downloading,
    Downloaded,
    Error,
}

/// Conteúdo de `status.json` de um modelo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelStatus {
    pub status: CacheStatus,
    /// Tamanho em bytes do arquivo principal (`ModelInfo::file`).
    pub size_bytes: u64,
    /// SHA256 registrado na conclusão do download (verificação da 2.3).
    pub sha256: Option<String>,
}

/// Erros do cache de modelos. Mensagens estáveis para a UI (padrão 4.8).
#[derive(Debug, Error)]
pub enum CacheError {
    #[error("diretório de cache de modelos não encontrado: {0}")]
    CacheDirMissing(String),
    #[error("erro de I/O em `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("`status.json` do modelo `{model_id}` não parseia: {source}")]
    StatusParse {
        model_id: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("modelo `{model_id}` ainda não foi baixado")]
    NotDownloaded { model_id: String },
    #[error("download do modelo `{model_id}` já está em andamento")]
    DownloadInProgress { model_id: String },
    #[error("último download do modelo `{model_id}` falhou (status=error)")]
    StatusError { model_id: String },
    #[error("arquivo do modelo `{model_id}` ausente em `{path}`")]
    FileMissing { model_id: String, path: PathBuf },
    #[error(
        "checksum registrado do modelo `{model_id}` diverge do catálogo \
         (esperado `{expected}`, registro `{actual}`)"
    )]
    ChecksumInconsistent {
        model_id: String,
        expected: String,
        actual: String,
    },
}

const STATUS_FILE: &str = "status.json";
const STATUS_TMP: &str = "status.json.tmp";
const LOCK_FILE: &str = ".lock";

/// Lock de download por modelo; removido no `Drop` (ou ao final do processo).
#[derive(Debug)]
pub struct DownloadLock {
    path: PathBuf,
}

impl Drop for DownloadLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Raiz do cache de modelos: `cache_dir()/legendai/models`.
pub fn models_root() -> Result<PathBuf, CacheError> {
    ROOT_OVERRIDE.with(|o| o.borrow().clone()).map_or_else(
        || {
            dirs::cache_dir()
                .map(|d| d.join("legendai").join("models"))
                .ok_or_else(|| {
                    CacheError::CacheDirMissing("dirs::cache_dir() retornou None".into())
                })
        },
        Ok,
    )
}

/// Diretório de um modelo: `models_root()/<kind>/<model_id>`.
pub fn model_dir(model: &ModelInfo) -> Result<PathBuf, CacheError> {
    Ok(models_root()?.join(kind_dir(model.kind)).join(&model.id))
}

/// Subdiretório do kind (espelha o serde `rename_all = "lowercase"` do catálogo).
fn kind_dir(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Stt => "stt",
        ModelKind::Translation => "translation",
    }
}

/// Lê `status.json` do modelo. `None` se o modelo nunca foi baixado.
pub fn read_status(model: &ModelInfo) -> Result<Option<ModelStatus>, CacheError> {
    let path = model_dir(model)?.join(STATUS_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|source| CacheError::StatusParse {
                    model_id: model.id.clone(),
                    source,
                })
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(CacheError::Io {
            path: path.clone(),
            source,
        }),
    }
}

/// Grava `status.json` do modelo (temp + rename — escrita atômica, padrão 0.7).
pub fn write_status(model: &ModelInfo, status: &ModelStatus) -> Result<(), CacheError> {
    let dir = model_dir(model)?;
    std::fs::create_dir_all(&dir).map_err(|source| CacheError::Io {
        path: dir.clone(),
        source,
    })?;
    let tmp = dir.join(STATUS_TMP);
    let final_path = dir.join(STATUS_FILE);
    let json = serde_json::to_vec_pretty(status).map_err(|source| CacheError::StatusParse {
        model_id: model.id.clone(),
        source,
    })?;
    std::fs::write(&tmp, json).map_err(|source| CacheError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, &final_path).map_err(|source| CacheError::Io {
        path: final_path.clone(),
        source,
    })
}

/// Adquire o lock de download do modelo. Se outro download do mesmo modelo já
/// estiver em andamento (lock presente e recente), retorna erro com mensagem
/// clara. Lock órfão (crash) é considerado stale após [`LOCK_STALE_AFTER_SECS`]
/// e é tomado, permitindo retomar o download via `.part` da 2.2.
pub fn acquire_download_lock(model: &ModelInfo) -> Result<DownloadLock, CacheError> {
    let dir = model_dir(model)?;
    let path = dir.join(LOCK_FILE);
    std::fs::create_dir_all(&dir).map_err(|source| CacheError::Io {
        path: dir.clone(),
        source,
    })?;
    match create_lock(&path) {
        Ok(lock) => Ok(lock),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            // Se o lock for stale (ou ilegível), remove e tenta uma vez de novo.
            if matches!(lock_is_stale(&path), Ok(true) | Err(_)) {
                let _ = std::fs::remove_file(&path);
                return match create_lock(&path) {
                    Ok(lock) => Ok(lock),
                    // Perdeu a corrida de novo (outra instância criou no meio).
                    Err(_) => Err(CacheError::DownloadInProgress {
                        model_id: model.id.clone(),
                    }),
                };
            }
            Err(CacheError::DownloadInProgress {
                model_id: model.id.clone(),
            })
        }
        Err(source) => Err(CacheError::Io {
            path: path.clone(),
            source,
        }),
    }
}

/// Lock a partir do qual um `.lock` é considerado órfão e tomado por outro
/// download. 1h cobre downloads grandes em conexão lenta com folga.
/// `ponytail:` stale por idade (não checa PID vivo) — portável e suficiente no
/// MVP; trocar por `flock`/checagem de processo se o app ganhar multi-instância
/// real com downloads longos.
const LOCK_STALE_AFTER_SECS: u64 = 3600;

/// Cria o lockfile (falha com `AlreadyExists` se já existir).
fn create_lock(path: &Path) -> Result<DownloadLock, std::io::Error> {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    writeln!(f, "{}\n{}", std::process::id(), unix_now())?;
    Ok(DownloadLock {
        path: path.to_path_buf(),
    })
}

/// Lock stale = PID morto (crash) OU timestamp mais velho que [`LOCK_STALE_AFTER_SECS`].
/// Conteúdo ilegível (crash entre `create_new` e a escrita) também é stale.
pub(crate) fn lock_is_stale(path: &Path) -> Result<bool, CacheError> {
    let content = std::fs::read_to_string(path).map_err(|source| CacheError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut lines = content.lines();
    let pid = lines.next().and_then(|l| l.trim().parse::<u32>().ok());
    let ts = lines.next().and_then(|l| l.trim().parse::<u64>().ok());

    // PID morto => stale imediato (crash), sem esperar 1h.
    if let Some(pid) = pid {
        if pid != std::process::id() && !pid_is_alive(pid) {
            return Ok(true);
        }
    } else {
        // Primeira linha ilegível => stale (crash antes de escrever PID)
        return Ok(true);
    }

    Ok(match ts {
        Some(ts) => unix_now().saturating_sub(ts) > LOCK_STALE_AFTER_SECS,
        None => true,
    })
}

/// Verifica se um PID ainda está vivo (portável: /proc no Unix, sysinfo no Windows/macOS).
fn pid_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    if std::path::Path::new(&format!("/proc/{pid}")).exists() {
        return true;
    }
    // Fallback genérico via sysinfo (cobre Windows e macOS sem /proc)
    {
        use sysinfo::{Pid, System};
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);
        sys.process(Pid::from_u32(pid)).is_some()
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Status efetivo para a UI: se `status.json` diz `downloading` mas o lock
/// está ausente ou stale (crash), trata como `None` (não baixado) para
/// permitir novo `Baixar` com retomada via `.part`. O arquivo em disco
/// permanece, mas a UI não fica presa em "baixando".
pub fn effective_status(model: &ModelInfo) -> Result<Option<ModelStatus>, CacheError> {
    let raw = read_status(model)?;
    if let Some(s) = &raw {
        if s.status == CacheStatus::Downloading {
            let lock_path = model_dir(model)?.join(LOCK_FILE);
            let is_stale = if lock_path.exists() {
                lock_is_stale(&lock_path).unwrap_or(true)
            } else {
                true // sem lock mas status downloading => crash, stale
            };
            if is_stale {
                // Limpeza automática: remove lock órfão e permite retry
                let _ = std::fs::remove_file(&lock_path);
                return Ok(None);
            }
        }
    }
    Ok(raw)
}

/// Resolve o caminho do arquivo principal do modelo, retornando-o **apenas** se
/// o modelo estiver baixado e íntegro:
/// 1. `status.json` deve existir com `status = downloaded` (senão erro claro);
/// 2. o arquivo `ModelInfo::file` deve existir em disco;
/// 3. o checksum registrado deve bater com o do catálogo (quando o catálogo
///    declara um) — sem re-hash do arquivo (modelos têm GB; integridade real é
///    verificada na conclusão do download pela 2.3).
pub fn resolve_model_path(model: &ModelInfo) -> Result<PathBuf, CacheError> {
    let dir = model_dir(model)?;
    // Usa effective_status para não confundir crash (Downloading stale) com download real em andamento
    let status = effective_status(model)?.ok_or_else(|| CacheError::NotDownloaded {
        model_id: model.id.clone(),
    })?;
    match status.status {
        CacheStatus::Downloading => {
            return Err(CacheError::DownloadInProgress {
                model_id: model.id.clone(),
            })
        }
        CacheStatus::Error => {
            return Err(CacheError::StatusError {
                model_id: model.id.clone(),
            })
        }
        CacheStatus::Downloaded => {}
    }

    let file_path = dir.join(&model.file);
    if !file_path.exists() {
        return Err(CacheError::FileMissing {
            model_id: model.id.clone(),
            path: file_path,
        });
    }

    if let Some(expected) = &model.sha256 {
        match &status.sha256 {
            Some(actual) if actual.eq_ignore_ascii_case(expected) => {}
            Some(actual) => {
                return Err(CacheError::ChecksumInconsistent {
                    model_id: model.id.clone(),
                    expected: expected.clone(),
                    actual: actual.clone(),
                })
            }
            None => tracing::warn!(
                "modelo `{}` baixado sem checksum registrado (catálogo declara um) — confiar no arquivo como está",
                model.id
            ),
        }
    }
    Ok(file_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, kind: ModelKind, file: &str, sha256: Option<&str>) -> ModelInfo {
        let mut v = serde_json::json!({
            "id": id, "kind": kind, "name": "Teste", "repo_id": "o/r",
            "file": file, "backend": "whisper", "quantization": "q5",
            "size_mb": 1, "min_ram_gb": 1, "quality": 3, "speed": 3,
            "threads_supported": true,
        });
        if let Some(h) = sha256 {
            v["sha256"] = serde_json::json!(h);
        }
        serde_json::from_value(v).unwrap()
    }

    fn stt(id: &str) -> ModelInfo {
        model(id, ModelKind::Stt, "model.bin", None)
    }

    #[test]
    fn models_root_aponta_para_cache_da_plataforma() {
        if let Some(cache) = dirs::cache_dir() {
            assert_eq!(
                models_root().unwrap(),
                cache.join("legendai").join("models")
            );
        }
    }

    #[test]
    fn model_dir_usa_kind_e_id() {
        with_root("model_dir", || {
            let stt_dir = model_dir(&stt("whisper-tiny")).unwrap();
            assert!(
                stt_dir.ends_with(Path::new("stt").join("whisper-tiny")),
                "stt: {stt_dir:?}"
            );
            let tr = model("nllb", ModelKind::Translation, "x.onnx", None);
            let tr_dir = model_dir(&tr).unwrap();
            assert!(tr_dir.ends_with(Path::new("translation").join("nllb")));
        });
    }

    #[test]
    fn status_round_trip_serde() {
        with_root("status", || {
            let m = stt("whisper-tiny");
            let status = ModelStatus {
                status: CacheStatus::Downloaded,
                size_bytes: 1234,
                sha256: Some(
                    "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21".into(),
                ),
            };
            write_status(&m, &status).unwrap();
            assert_eq!(read_status(&m).unwrap(), Some(status));
            assert!(model_dir(&m).unwrap().join("status.json").exists());
        });
    }

    #[test]
    fn status_ausente_retorna_none() {
        with_root("status_none", || {
            assert_eq!(read_status(&stt("nunca-baixado")).unwrap(), None);
        });
    }

    #[test]
    fn status_corrompido_retorna_erro_tipado() {
        with_root("status_corrompido", || {
            let m = stt("corrompido");
            let dir = model_dir(&m).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(STATUS_FILE), "{{{ não é json").unwrap();
            assert!(matches!(
                read_status(&m).unwrap_err(),
                CacheError::StatusParse { .. }
            ));
        });
    }

    #[test]
    fn lock_impede_download_duplo() {
        with_root("lock_duplo", || {
            let m = stt("whisper-tiny");
            let lock1 = acquire_download_lock(&m).unwrap();
            let err = acquire_download_lock(&m).unwrap_err();
            assert!(
                matches!(&err, CacheError::DownloadInProgress { model_id } if model_id == "whisper-tiny"),
                "esperava DownloadInProgress, veio: {err}"
            );
            drop(lock1);
            // Lock liberado → nova aquisição funciona.
            let lock2 = acquire_download_lock(&m).unwrap();
            assert!(model_dir(&m).unwrap().join(LOCK_FILE).exists());
            drop(lock2);
            assert!(!model_dir(&m).unwrap().join(LOCK_FILE).exists());
        });
    }

    #[test]
    fn lock_fresh_nao_e_stale() {
        with_root("lock_fresh", || {
            let m = stt("fresh");
            let dir = model_dir(&m).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(LOCK_FILE);
            std::fs::write(&path, format!("{}\n{}", std::process::id(), unix_now())).unwrap();
            assert!(!lock_is_stale(&path).unwrap());
        });
    }

    #[test]
    fn lock_antigo_e_stale_e_tomado() {
        with_root("lock_stale", || {
            let m = stt("antigo");
            let dir = model_dir(&m).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join(LOCK_FILE);
            // Timestamp de 2h atrás → stale.
            let antigo = unix_now() - (LOCK_STALE_AFTER_SECS * 2);
            std::fs::write(&path, format!("99999\n{antigo}")).unwrap();
            assert!(lock_is_stale(&path).unwrap());
            // Lock stale não bloqueia nova aquisição.
            let lock = acquire_download_lock(&m).unwrap();
            assert!(path.exists());
            drop(lock);
            assert!(!path.exists());
        });
    }

    #[test]
    fn resolve_so_retorna_se_downloaded_e_arquivo_existe() {
        with_root("resolve", || {
            let m = stt("whisper-tiny");
            let dir = model_dir(&m).unwrap();
            std::fs::create_dir_all(&dir).unwrap();

            // Sem status → NotDownloaded.
            let err = resolve_model_path(&m).unwrap_err();
            assert!(matches!(err, CacheError::NotDownloaded { .. }));

            // Status downloading + lock fresco → DownloadInProgress.
            write_status(
                &m,
                &ModelStatus {
                    status: CacheStatus::Downloading,
                    size_bytes: 10,
                    sha256: None,
                },
            )
            .unwrap();
            std::fs::write(
                dir.join(LOCK_FILE),
                format!("{}\n{}", std::process::id(), unix_now()),
            )
            .unwrap();
            let err = resolve_model_path(&m).unwrap_err();
            assert!(matches!(err, CacheError::DownloadInProgress { .. }));
            // Sem lock, downloading stale => NotDownloaded (permite retry)
            std::fs::remove_file(dir.join(LOCK_FILE)).unwrap();
            let err = resolve_model_path(&m).unwrap_err();
            assert!(matches!(err, CacheError::NotDownloaded { .. }));
            // Recria lock fresco para seguir o teste original (próximo passo exige Downloaded)
            std::fs::write(
                dir.join(LOCK_FILE),
                format!("{}\n{}", std::process::id(), unix_now()),
            )
            .unwrap();
            let _ = std::fs::remove_file(dir.join(LOCK_FILE));

            // Status downloaded mas arquivo ausente → FileMissing.
            write_status(
                &m,
                &ModelStatus {
                    status: CacheStatus::Downloaded,
                    size_bytes: 10,
                    sha256: None,
                },
            )
            .unwrap();
            let err = resolve_model_path(&m).unwrap_err();
            assert!(matches!(err, CacheError::FileMissing { .. }));

            // Downloaded + arquivo presente → caminho do arquivo principal.
            std::fs::write(dir.join("model.bin"), b"dados").unwrap();
            let path = resolve_model_path(&m).unwrap();
            assert_eq!(path, dir.join("model.bin"));
        });
    }

    #[test]
    fn resolve_rejeita_checksum_inconsistente() {
        with_root("resolve_csum", || {
            let m = model(
                "m",
                ModelKind::Stt,
                "model.bin",
                Some("be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"),
            );
            let dir = model_dir(&m).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("model.bin"), b"dados").unwrap();
            // Registro com hash divergente do catálogo.
            write_status(
                &m,
                &ModelStatus {
                    status: CacheStatus::Downloaded,
                    size_bytes: 5,
                    sha256: Some(
                        "0000000000000000000000000000000000000000000000000000000000000000".into(),
                    ),
                },
            )
            .unwrap();
            let err = resolve_model_path(&m).unwrap_err();
            assert!(matches!(err, CacheError::ChecksumInconsistent { .. }));
        });
    }

    #[test]
    fn resolve_aceita_registro_igual_ao_catalogo() {
        with_root("resolve_ok", || {
            let m = model(
                "m",
                ModelKind::Stt,
                "model.bin",
                Some("be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21"),
            );
            let dir = model_dir(&m).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("model.bin"), b"dados").unwrap();
            write_status(
                &m,
                &ModelStatus {
                    status: CacheStatus::Downloaded,
                    size_bytes: 5,
                    sha256: Some(
                        "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21".into(),
                    ),
                },
            )
            .unwrap();
            assert_eq!(resolve_model_path(&m).unwrap(), dir.join("model.bin"));
        });
    }

    #[test]
    fn resolve_aceita_modelo_sem_checksum_no_catalogo() {
        with_root("resolve_semhash", || {
            let m = stt("sem-hash");
            let dir = model_dir(&m).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("model.bin"), b"dados").unwrap();
            write_status(
                &m,
                &ModelStatus {
                    status: CacheStatus::Downloaded,
                    size_bytes: 5,
                    sha256: None,
                },
            )
            .unwrap();
            assert_eq!(resolve_model_path(&m).unwrap(), dir.join("model.bin"));
        });
    }
}
