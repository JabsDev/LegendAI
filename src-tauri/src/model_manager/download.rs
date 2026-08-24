//! Download de modelos via Hugging Face (tarefas 1.6 e 2.2).
//!
//! - [`download_file`]: primitivo do MVP (1.6) via `hf-hub`, caminho fixo.
//! - [`download_model`]: versão da Fase 2 (2.2) com **retomada** (`.part` +
//!   header `Range`), **progresso em bytes** e **cancelamento cooperativo**
//!   via [`CancellationToken`]. Requisições HTTP por `reqwest`; `hf-hub`
//!   usado apenas para resolver `repo_id` (owner/name).

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use hf_hub::progress::{DownloadEvent, Progress, ProgressEvent, ProgressHandler};
use hf_hub::HFClientSync;
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

/// Erros do download de modelos. Rede/repo/I-O retornam variantes tipadas
/// (mensagem estável para a UI em 4.8), nunca panic.
#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("falha ao inicializar o cliente do Hugging Face: {0}")]
    Client(#[source] hf_hub::HFError),
    #[error("falha ao baixar `{file}` de `{repo_id}`: {source}")]
    Download {
        repo_id: String,
        file: String,
        #[source]
        source: hf_hub::HFError,
    },
    #[error("falha ao criar cliente HTTP: {0}")]
    HttpClient(#[source] reqwest::Error),
    #[error("falha de rede ao baixar `{file}`: {source}")]
    Network {
        file: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("erro de I/O em `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("download cancelado")]
    Cancelled,
    #[error("resposta inesperada do servidor: {0}")]
    UnexpectedStatus(StatusCode),
    #[error("diretório de cache de modelos não encontrado: {0}")]
    CacheDirMissing(String),
}

/// Diretório fixo dos modelos Whisper: `cache_dir()/legendai/models/whisper/`.
#[allow(dead_code)] // consumido pelo pipeline (1.9) e comandos IPC (Fase 2)
pub fn whisper_dir() -> Result<PathBuf, DownloadError> {
    dirs::cache_dir()
        .map(|d| d.join("legendai").join("models").join("whisper"))
        .ok_or_else(|| DownloadError::CacheDirMissing("dirs::cache_dir() retornou None".into()))
}

/// Baixa `file` do repo HF `repo_id` para `dest_dir` (arquivo salvo como
/// `dest_dir/<file>`), notificando `progress_cb(bytes_completos, bytes_totais)`.
/// Modelo já presente em `dest_dir` não é re-baixado (cache do hf-hub).
#[allow(dead_code)] // primitivo do MVP (1.6), mantido p/ compatibilidade
pub fn download_file<F>(
    repo_id: &str,
    file: &str,
    dest_dir: &Path,
    progress_cb: F,
) -> Result<PathBuf, DownloadError>
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    let client = HFClientSync::new().map_err(DownloadError::Client)?;
    let (owner, name) = hf_hub::split_id(repo_id);
    let repo = client.model(owner, name);
    repo.download_file()
        .filename(file.to_string())
        .local_dir(dest_dir.to_path_buf())
        .progress(Progress::new(CallbackHandler(progress_cb)))
        .send()
        .map_err(|source| DownloadError::Download {
            repo_id: repo_id.to_string(),
            file: file.to_string(),
            source,
        })
}

/// Adapta os eventos de progresso do hf-hub para o callback `Fn(u64, u64)`.
struct CallbackHandler<F>(F);

impl<F: Fn(u64, u64) + Send + Sync> ProgressHandler for CallbackHandler<F> {
    fn on_progress(&self, event: &ProgressEvent) {
        if let ProgressEvent::Download(ev) = event {
            match ev {
                DownloadEvent::Start { total_bytes, .. } => self.0(0, *total_bytes),
                DownloadEvent::Progress { files } => {
                    if let Some(f) = files.last() {
                        self.0(f.bytes_completed, f.total_bytes);
                    }
                }
                DownloadEvent::AggregateProgress {
                    bytes_completed,
                    total_bytes,
                    ..
                } => self.0(*bytes_completed, *total_bytes),
                DownloadEvent::Complete => {}
            }
        }
    }
}

/// Constrói a URL de download via endpoint `resolve` do HF:
/// `https://huggingface.co/{owner}/{name}/resolve/main/{file}`.
pub fn resolve_url(repo_id: &str, file: &str) -> String {
    let (owner, name) = hf_hub::split_id(repo_id);
    format!("https://huggingface.co/{owner}/{name}/resolve/main/{file}")
}

/// Baixa o modelo `repo_id/file` para `dest_dir` com retomada e cancelamento
/// cooperativo. O progresso parcial fica em `<dest_dir>/<file>.part`; o arquivo
/// final só é criado (renomeando o `.part`) quando o download chega a 100%.
///
/// - Se um `.part` já existir, retoma do seu tamanho via header `Range`
///   (servidor que ignore o `Range` → recomeça do zero).
/// - `token.cancel()` interrompe entre chunks mantendo o `.part` consistente
///   para uma futura retomada.
#[allow(dead_code)] // consumido por comandos IPC da Fase 2
pub async fn download_model<F>(
    repo_id: &str,
    file: &str,
    dest_dir: &Path,
    token: &CancellationToken,
    progress_cb: F,
) -> Result<PathBuf, DownloadError>
where
    F: Fn(u64, u64) + Send + Sync + 'static,
{
    let client = reqwest::Client::new();
    let dest = dest_dir.join(file);
    download_resumable(
        &client,
        &resolve_url(repo_id, file),
        file,
        &dest,
        token,
        &progress_cb,
    )
    .await?;
    Ok(dest)
}

/// Núcleo de download com retomada, independente de URL (testável contra um
/// servidor HTTP local). Escreve em `<dest>.part` e renomeia para `dest` em 100%.
async fn download_resumable<F>(
    client: &reqwest::Client,
    url: &str,
    file: &str,
    dest: &Path,
    token: &CancellationToken,
    progress_cb: &F,
) -> Result<(), DownloadError>
where
    F: Fn(u64, u64) + Sync,
{
    use futures_util::StreamExt;

    let part = part_path(dest);
    let offset = existing_len(&part)?;

    let resp = client
        .get(url)
        .header(RANGE, format!("bytes={offset}-"))
        .send()
        .await
        .map_err(|source| DownloadError::Network {
            file: file.to_string(),
            source,
        })?;

    let status = resp.status();
    let (offset, total) = match status {
        StatusCode::PARTIAL_CONTENT => {
            let content_len = resp.content_length().unwrap_or(0);
            (
                offset,
                content_range_total(resp.headers()).unwrap_or(offset + content_len),
            )
        }
        StatusCode::OK => {
            // Servidor ignorou o Range → recomeça do zero.
            (0, resp.content_length().unwrap_or(0))
        }
        _ => return Err(DownloadError::UnexpectedStatus(status)),
    };

    let mut part_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&part)
        .map_err(|source| DownloadError::Io {
            path: part.clone(),
            source,
        })?;
    if offset == 0 {
        part_file.set_len(0).map_err(|source| DownloadError::Io {
            path: part.clone(),
            source,
        })?;
    }
    part_file
        .seek(SeekFrom::Start(offset))
        .map_err(|source| DownloadError::Io {
            path: part.clone(),
            source,
        })?;

    let mut stream = resp.bytes_stream();
    let mut written = offset;
    loop {
        if token.is_cancelled() {
            return Err(DownloadError::Cancelled);
        }
        let chunk = match stream.next().await {
            None => break,
            Some(Err(source)) => {
                return Err(DownloadError::Network {
                    file: file.to_string(),
                    source,
                })
            }
            Some(Ok(chunk)) => chunk,
        };
        part_file
            .write_all(&chunk)
            .map_err(|source| DownloadError::Io {
                path: part.clone(),
                source,
            })?;
        written += chunk.len() as u64;
        progress_cb(written, total);
    }

    if total > 0 && written != total {
        return Err(DownloadError::Io {
            path: part.clone(),
            source: std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                format!("download incompleto: {written}/{total} bytes"),
            ),
        });
    }

    std::fs::rename(&part, dest).map_err(|source| DownloadError::Io {
        path: dest.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Caminho do arquivo parcial: `<dest>.part`.
fn part_path(dest: &Path) -> PathBuf {
    let mut os = dest.as_os_str().to_os_string();
    os.push(".part");
    PathBuf::from(os)
}

/// Tamanho de um `.part` existente (0 se não existir).
fn existing_len(path: &Path) -> Result<u64, DownloadError> {
    match std::fs::metadata(path) {
        Ok(m) => Ok(m.len()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(DownloadError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Extrai o tamanho total de um header `Content-Range: bytes s-e/total`.
fn content_range_total(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .trim()
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::*;

    #[test]
    fn whisper_dir_aponta_para_cache_da_plataforma() {
        if let Some(cache) = dirs::cache_dir() {
            assert_eq!(
                whisper_dir().unwrap(),
                cache.join("legendai").join("models").join("whisper")
            );
        }
    }

    #[test]
    fn split_repo_id_em_owner_e_name() {
        assert_eq!(
            hf_hub::split_id("ggerganov/whisper.cpp"),
            ("ggerganov", "whisper.cpp")
        );
        assert_eq!(hf_hub::split_id("gpt2"), ("", "gpt2"));
    }

    #[test]
    fn resolve_url_monta_endpoint_do_hf() {
        assert_eq!(
            resolve_url("ggerganov/whisper.cpp", "ggml-tiny.bin"),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin"
        );
    }

    /// Servidor HTTP mínimo de teste. Responde sempre 206 com `Content-Range`.
    /// No request com offset 0 aplica o `Behavior` (Drop ou Slow); requests com
    /// offset > 0 servem o restante normalmente.
    struct MockServer {
        addr: SocketAddr,
    }

    enum Behavior {
        /// No offset 0: envia o prefixo e fecha no meio do corpo (queda de rede).
        Drop { prefix: usize },
        /// No offset 0: envia em chunks pequenos com delay (permite cancelar).
        Slow { chunk: usize, delay: Duration },
    }

    impl MockServer {
        fn start(body: Vec<u8>, behavior: Behavior) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = match stream {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    // Servidor de teste vive enquanto o processo viver; uma
                    // conexão abortada (cancelamento) segue para a próxima.
                    let _ = handle(&mut stream, &body, &behavior);
                }
            });
            MockServer { addr }
        }

        fn url(&self, file: &str) -> String {
            format!("http://{}/{}", self.addr, file)
        }
    }

    fn handle(stream: &mut TcpStream, body: &[u8], behavior: &Behavior) -> bool {
        let mut req = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => return false,
                Ok(n) => {
                    req.extend_from_slice(&buf[..n]);
                    if req.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => return false,
            }
        }
        let req = String::from_utf8_lossy(&req).to_string();
        let offset = req
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))
            .and_then(|l| l.rsplit('=').next())
            .and_then(|s| s.trim().trim_end_matches('-').parse::<usize>().ok())
            .unwrap_or(0);

        let head = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
            body.len() - offset,
            offset,
            body.len() - 1,
            body.len()
        );
        if stream.write_all(head.as_bytes()).is_err() {
            return false;
        }

        if offset == 0 {
            match behavior {
                Behavior::Drop { prefix } => {
                    let n = (*prefix).min(body.len());
                    if stream.write_all(&body[..n]).is_err() {
                        return false;
                    }
                    let _ = stream.flush();
                    return true; // fecha no meio do corpo
                }
                Behavior::Slow { chunk, delay } => {
                    for part in body.chunks(*chunk) {
                        if stream.write_all(part).is_err() {
                            return false;
                        }
                        let _ = stream.flush();
                        thread::sleep(*delay);
                    }
                }
            }
        } else if stream.write_all(&body[offset..]).is_err() {
            return false;
        }
        let _ = stream.flush();
        true
    }

    fn body_pattern(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("legendai-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Interrompe o download no meio (queda de rede) e verifica que a retomada
    /// continua do offset via `Range`, gerando o arquivo final idêntico.
    #[tokio::test]
    async fn retoma_de_offset_apos_download_interrompido() {
        let body = body_pattern(64 * 1024);
        let server = MockServer::start(body.clone(), Behavior::Drop { prefix: 10000 });
        let dest = temp_dir("resume");
        let dest_path = dest.join("model.bin");
        let client = reqwest::Client::new();
        let token = CancellationToken::new();

        let err = download_resumable(
            &client,
            &server.url("model.bin"),
            "model.bin",
            &dest_path,
            &token,
            &|_, _| {},
        )
        .await;
        assert!(
            err.is_err(),
            "primeiro download deve falhar com a queda de rede"
        );

        let part = dest.join("model.bin.part");
        let plen = std::fs::metadata(&part).expect(".part deve existir").len();
        assert!(plen > 0 && plen < body.len() as u64, "parcial: {plen}");
        assert!(
            !dest_path.exists(),
            "arquivo final não deve existir antes de 100%"
        );

        download_resumable(
            &client,
            &server.url("model.bin"),
            "model.bin",
            &dest_path,
            &token,
            &|_, _| {},
        )
        .await
        .expect("retomada deve completar");
        assert_eq!(std::fs::read(&dest_path).unwrap(), body);
        assert!(!part.exists(), ".part deve sumir após o sucesso");

        std::fs::remove_dir_all(&dest).ok();
    }

    /// Cancelamento cooperativo deixa o `.part` consistente (sem arquivo final
    /// corrompido) e a retomada posterior completa o download.
    #[tokio::test]
    async fn cancelamento_deixa_part_consistente() {
        let body = body_pattern(32 * 1024);
        let server = MockServer::start(
            body.clone(),
            Behavior::Slow {
                chunk: 1024,
                delay: Duration::from_millis(5),
            },
        );
        let dest = temp_dir("cancel");
        let dest_path = dest.join("model.bin");
        let client = reqwest::Client::new();
        let token = CancellationToken::new();
        let t = token.clone();
        let seen = AtomicBool::new(false);

        let err = download_resumable(
            &client,
            &server.url("model.bin"),
            "model.bin",
            &dest_path,
            &token,
            &|done, _| {
                if !seen.load(Ordering::Relaxed) && done > 0 {
                    seen.store(true, Ordering::Relaxed);
                    t.cancel();
                }
            },
        )
        .await;

        assert!(
            matches!(err, Err(DownloadError::Cancelled)),
            "esperava cancelamento, veio: {err:?}"
        );
        let part = dest.join("model.bin.part");
        let plen = std::fs::metadata(&part)
            .expect(".part deve permanecer")
            .len();
        assert!(plen > 0 && plen < body.len() as u64, "parcial: {plen}");
        assert!(
            !dest_path.exists(),
            "cancelamento não deve criar o arquivo final"
        );

        let token2 = CancellationToken::new();
        download_resumable(
            &client,
            &server.url("model.bin"),
            "model.bin",
            &dest_path,
            &token2,
            &|_, _| {},
        )
        .await
        .expect("retomada pós-cancelamento deve completar");
        assert_eq!(std::fs::read(&dest_path).unwrap(), body);
        assert!(!part.exists());

        std::fs::remove_dir_all(&dest).ok();
    }

    /// Teste manual de download real (exige rede). Roda com:
    /// `cargo test -- --ignored baixa_modelo_de_repo_publico`
    #[test]
    #[ignore = "faz download real de ~77MB da rede (não roda em CI)"]
    fn baixa_modelo_de_repo_publico() {
        let repo =
            std::env::var("LEGENDAI_MODEL_REPO").unwrap_or_else(|_| "ggerganov/whisper.cpp".into());
        let file = std::env::var("LEGENDAI_MODEL_FILE").unwrap_or_else(|_| "ggml-tiny.bin".into());

        let dest = std::env::temp_dir().join(format!("legendai-dl-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dest);

        let path = download_file(&repo, &file, &dest, |done, total| {
            if total > 0 {
                tracing::info!("{done}/{total} bytes");
            }
        })
        .expect("download deve completar");

        assert_eq!(path, dest.join(&file));
        assert!(path.exists(), "arquivo deve existir no caminho esperado");
        assert!(
            std::fs::metadata(&path).unwrap().len() > 0,
            "arquivo não deve ser vazio"
        );
        eprintln!("modelo baixado em: {}", path.display());

        std::fs::remove_dir_all(&dest).ok();
    }
}
