//! Busca livre de modelos no Hugging Face (tarefa 2.7).
//!
//! Consulta `https://huggingface.co/api/models?search=...` com `full=true`
//! (inclui `siblings`, a lista de arquivos do repo) e normaliza os resultados
//! para [`HfSearchResult`], filtrando por compatibilidade com o `kind` pedido:
//!
//! - `Stt` (whisper): arquivos `ggml-*.bin`/`*.gguf` (nomes com `ggml`/`whisper`).
//! - `Translation`: arquivos `*.gguf` (backend `llama`, excluindo whisper) ou
//!   `*.onnx` (backend `ort`). GGUF tem precedência sobre ONNX quando ambos
//!   existem no mesmo repo.
//!
//! Repos sem nenhum arquivo compatível são excluídos. O filtro de tamanho
//! (arquivo escolhido < RAM disponível) é aplicado por chamada porque depende
//! do [`HardwareInfo`] da máquina (2.5); o cache em memória de 10min guarda o
//! resultado normalizado (independente de RAM) para respeitar o rate limit da
//! API pública do HF (nota da tarefa). Offline → [`SearchError::Network`] com
//! sugestão de verificar a conexão, nunca crash.
//!
//! Limitação conhecida: o `size` de `siblings` nem sempre vem na resposta da
//! API — repos sem tamanho passam pelo filtro de RAM (tratados como
//! compatíveis) e o campo fica `None` para a UI exibir "desconhecido".

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hardware::detect::HardwareInfo;
use crate::model_manager::catalog::{Backend, ModelKind};

/// TTL do cache em memória (rate limit da API pública do HF).
const CACHE_TTL: Duration = Duration::from_secs(600);
/// Tamanho de página padrão da busca (paginação via `offset`).
const DEFAULT_LIMIT: u32 = 20;

/// Erros da busca. Rede/servidor/JSON retornam variantes tipadas com mensagem
/// estável para a UI (padrão 4.8), nunca panic.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("sem conexão com o Hugging Face — verifique sua conexão com a internet: {0}")]
    Network(#[source] reqwest::Error),
    #[error("resposta inesperada do Hugging Face: {0}")]
    UnexpectedStatus(StatusCode),
    #[error("a resposta do Hugging Face não pôde ser interpretada: {0}")]
    Response(#[source] reqwest::Error),
}

/// Resultado normalizado de busca, para a UI exibir (consumido em 2.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HfSearchResult {
    /// Repo completo `owner/name` (identidade para baixar em 2.9).
    pub repo_id: String,
    /// Nome curto do repo (parte após o `/`), para exibição.
    pub name: String,
    pub kind: ModelKind,
    pub backend: Backend,
    /// Arquivo compatível escolhido como principal (maior do repo).
    pub file: String,
    /// Quantização detectada no nome do arquivo (`q4_k_m`, `fp16`, ...).
    pub quant: Option<String>,
    /// Tamanho em MiB (`None` se a API não informou o `size` do arquivo).
    pub size_mb: Option<u64>,
    pub downloads: u64,
    pub likes: u64,
    pub tags: Vec<String>,
}

/// Cliente de busca com cache curto em memória. Thread-safe (`Mutex` no
/// cache); `reqwest::Client` já é `Clone` e compartilha o pool de conexões.
#[allow(dead_code)] // consumido pelos comandos IPC da Fase 2 (2.8/2.9)
pub struct HfSearch {
    client: reqwest::Client,
    base_url: String,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

struct CacheEntry {
    at: Instant,
    /// Resultados normalizados e filtrados por compatibilidade (independentes
    /// de RAM); o filtro de tamanho é reaplicado a cada leitura.
    results: Vec<HfSearchResult>,
}

/// Chave do cache: query + kind + página (offset/limit). `ModelKind` não
/// deriva `Hash` — usar `Debug` como chave evita tocar no catálogo.
fn cache_key(query: &str, kind: ModelKind, limit: u32, offset: u32) -> String {
    format!("{query}\u{1f}{kind:?}\u{1f}{limit}\u{1f}{offset}")
}

#[allow(dead_code)] // consumido pelos comandos IPC da Fase 2 (2.8/2.9)
impl HfSearch {
    /// Cliente apontando para a API pública do Hugging Face.
    pub fn new() -> Self {
        Self::with_base_url("https://huggingface.co/api")
    }

    /// Cliente apontando para outra base — usada pelos testes com servidor
    /// HTTP local (mock, sem rede).
    fn with_base_url(base_url: &str) -> Self {
        HfSearch {
            client: reqwest::Client::new(),
            base_url: base_url.to_string(),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Nº de entradas atualmente no cache (para os testes de TTL).
    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.cache.lock().unwrap().len()
    }

    /// Primeira página da busca (até [`DEFAULT_LIMIT`] resultados).
    pub async fn search(
        &self,
        query: &str,
        kind: ModelKind,
        hw: &HardwareInfo,
    ) -> Result<Vec<HfSearchResult>, SearchError> {
        self.search_page(query, kind, hw, DEFAULT_LIMIT, 0).await
    }

    /// Busca uma página (paginação via `offset`). Cache de 10min por
    /// (query, kind, offset, limit); o filtro de RAM é reaplicado na leitura.
    pub async fn search_page(
        &self,
        query: &str,
        kind: ModelKind,
        hw: &HardwareInfo,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<HfSearchResult>, SearchError> {
        let key = cache_key(query, kind, limit, offset);
        if let Some(hit) = self.cached(&key) {
            return Ok(Self::apply_ram_filter(hit, hw));
        }

        let params: Vec<(String, String)> = vec![
            ("search".into(), query.to_string()),
            ("full".into(), "true".into()),
            ("limit".into(), limit.to_string()),
            ("offset".into(), offset.to_string()),
            ("sort".into(), "downloads".into()),
        ];
        let url = reqwest::Url::parse_with_params(&format!("{}/models", self.base_url), &params)
            .expect("base_url fixa/interna — parse não pode falhar");

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(SearchError::Network)?;
        let status = resp.status();
        if !status.is_success() {
            return Err(SearchError::UnexpectedStatus(status));
        }
        let models: Vec<HfModel> = resp.json().await.map_err(SearchError::Response)?;
        let results: Vec<HfSearchResult> =
            models.iter().filter_map(|m| normalize(m, kind)).collect();
        self.store(key, results.clone());
        Ok(Self::apply_ram_filter(results, hw))
    }

    fn cached(&self, key: &str) -> Option<Vec<HfSearchResult>> {
        let mut cache = self.cache.lock().unwrap();
        match cache.get(key) {
            Some(entry) if entry.at.elapsed() < CACHE_TTL => Some(entry.results.clone()),
            _ => {
                cache.remove(key);
                None
            }
        }
    }

    fn store(&self, key: String, results: Vec<HfSearchResult>) {
        self.cache.lock().unwrap().insert(
            key,
            CacheEntry {
                at: Instant::now(),
                results,
            },
        );
    }

    /// Exclui modelos maiores que a RAM disponível (`tamanho < RAM`). RAM
    /// desconhecida (0) → não filtra por tamanho.
    fn apply_ram_filter(results: Vec<HfSearchResult>, hw: &HardwareInfo) -> Vec<HfSearchResult> {
        let ram_limit_mb = (hw.ram_gb as u64).saturating_mul(1024);
        if ram_limit_mb == 0 {
            return results;
        }
        results
            .into_iter()
            .filter(|r| r.size_mb.map(|s| s < ram_limit_mb).unwrap_or(true))
            .collect()
    }
}

/// Resposta mínima de um item da API do HF (`full=true` inclui `siblings`).
#[derive(Debug, Deserialize)]
struct HfModel {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    siblings: Vec<HfSibling>,
}

/// Um arquivo do repo. `size` (bytes) só está presente quando a API o informa.
#[derive(Debug, Deserialize)]
struct HfSibling {
    rfilename: String,
    #[serde(default)]
    size: Option<u64>,
}

/// Normaliza um item da API para [`HfSearchResult`]; `None` se o repo não tem
/// nenhum arquivo compatível com o `kind`.
fn normalize(m: &HfModel, kind: ModelKind) -> Option<HfSearchResult> {
    let (file, size, backend) = pick_file(&m.siblings, kind)?;
    Some(HfSearchResult {
        repo_id: m.id.clone(),
        name: m.id.rsplit('/').next().unwrap_or(&m.id).to_string(),
        kind,
        backend,
        quant: detect_quant(&file),
        size_mb: size.map(|b| b / (1024 * 1024)),
        downloads: m.downloads,
        likes: m.likes,
        tags: m.tags.clone(),
        file,
    })
}

/// Compatibilidade de um arquivo com o `kind`:
/// - STT (whisper): `ggml-*.bin`/`*.gguf` (GGUF do whisper geralmente é nomeado
///   `ggml-*` ou contém "whisper").
/// - Tradução: `*.gguf` (llama) **que não seja whisper** (nomes `ggml-*` ou
///   contendo "whisper" são whisper) ou `*.onnx` (ort).
fn compatible(rfilename: &str, kind: ModelKind) -> bool {
    match kind {
        ModelKind::Stt => {
            rfilename.starts_with("ggml")
                || (rfilename.ends_with(".gguf") && rfilename.contains("whisper"))
        }
        ModelKind::Translation => {
            (rfilename.ends_with(".gguf")
                && !rfilename.starts_with("ggml")
                && !rfilename.contains("whisper"))
                || rfilename.ends_with(".onnx")
        }
    }
}

/// Escolhe o arquivo compatível principal de um repo e o backend correspondente.
/// Para tradução, GGUF (llama) tem precedência sobre ONNX (ort); entre os
/// compatíveis, escolhe o maior arquivo (o modelo principal, não auxiliares).
fn pick_file(siblings: &[HfSibling], kind: ModelKind) -> Option<(String, Option<u64>, Backend)> {
    let mut candidates: Vec<&HfSibling> = siblings
        .iter()
        .filter(|s| compatible(&s.rfilename, kind))
        .collect();
    if kind == ModelKind::Translation {
        let has_gguf = candidates.iter().any(|s| s.rfilename.ends_with(".gguf"));
        if has_gguf {
            candidates.retain(|s| s.rfilename.ends_with(".gguf"));
        }
    }
    let best = candidates.into_iter().max_by_key(|s| s.size.unwrap_or(0))?;
    let backend = match kind {
        ModelKind::Stt => Backend::Whisper,
        ModelKind::Translation => {
            if best.rfilename.ends_with(".onnx") {
                Backend::Ort
            } else {
                Backend::Llama
            }
        }
    };
    Some((best.rfilename.clone(), best.size, backend))
}

/// Extrai a quantização do nome do arquivo (ex: `q5_1`, `q4_k_m`, `fp16`).
/// `None` se o nome não contém um marcador de quantização reconhecível.
fn detect_quant(file: &str) -> Option<String> {
    let parts: Vec<&str> = file.split(['-', '_', '.']).collect();
    for p in &parts {
        if matches!(*p, "fp16" | "f16" | "int8") {
            return Some(p.to_string());
        }
    }
    let start = parts.iter().position(|p| {
        p.len() >= 2 && p.starts_with('q') && p[1..].chars().all(|c| c.is_ascii_digit())
    })?;
    let mut quant = parts[start].to_string();
    for p in &parts[start + 1..] {
        let modifier = p.len() == 1
            && (p.chars().all(|c| c.is_ascii_alphabetic())
                || p.chars().all(|c| c.is_ascii_digit()));
        if modifier {
            quant.push('_');
            quant.push_str(p);
        } else {
            break;
        }
    }
    Some(quant)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;

    use super::*;

    fn hw(ram_gb: u32) -> HardwareInfo {
        HardwareInfo {
            ram_gb,
            cpu_threads: 4,
            gpu: None,
            cpu_name: "test".into(),
            recommended_threads: 2,
        }
    }

    fn sibling(name: &str, size: Option<u64>) -> HfSibling {
        HfSibling {
            rfilename: name.into(),
            size,
        }
    }

    /// Corpo JSON simulando a resposta da API do HF (`full=true`).
    fn api_body() -> Vec<u8> {
        serde_json::json!([
            {
                "id": "org/llm-repo",
                "downloads": 1000,
                "likes": 50,
                "tags": ["gguf"],
                "siblings": [
                    {"rfilename": "model-q4_k_m.gguf", "size": 2100000000},
                    {"rfilename": "tokenizer.json", "size": 50000},
                    {"rfilename": "aux.onnx", "size": 100}
                ]
            },
            {
                "id": "org/onnx-repo",
                "downloads": 800,
                "likes": 10,
                "tags": ["onnx"],
                "siblings": [
                    {"rfilename": "encoder.onnx", "size": 700000000},
                    {"rfilename": "decoder.onnx", "size": 650000000}
                ]
            },
            {
                "id": "org/no-compat",
                "downloads": 900,
                "likes": 5,
                "siblings": [{"rfilename": "model.bin", "size": 1000000}]
            },
            {
                "id": "org/whisper-repo",
                "downloads": 500,
                "likes": 2,
                "tags": ["whisper"],
                "siblings": [
                    {"rfilename": "ggml-small-q5_1.bin", "size": 1000000000},
                    {"rfilename": "ggml-tiny.bin", "size": 70000000}
                ]
            },
            {
                "id": "org/whisper-gguf",
                "downloads": 300,
                "likes": 1,
                "siblings": [{"rfilename": "whisper-tiny-q8_0.gguf", "size": 50000000}]
            },
            {
                "id": "org/huge-llm",
                "downloads": 200,
                "likes": 0,
                "siblings": [{"rfilename": "model-q8_0.gguf", "size": 9_000_000_000u64}]
            }
        ])
        .to_string()
        .into_bytes()
    }

    /// Servidor HTTP mínimo servindo um corpo fixo e contando requisições.
    struct MockApi {
        addr: SocketAddr,
        hits: Arc<AtomicUsize>,
    }

    impl MockApi {
        fn start(body: Vec<u8>, status: u16) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let hits = Arc::new(AtomicUsize::new(0));
            let h = hits.clone();
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let _ = read_request(&mut stream);
                    h.fetch_add(1, Ordering::SeqCst);
                    let reason = if status == 200 { "OK" } else { "Error" };
                    let head = format!(
                        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(&body);
                    let _ = stream.flush();
                }
            });
            MockApi { addr, hits }
        }

        fn base_url(&self) -> String {
            format!("http://{}", self.addr)
        }
    }

    fn read_request(stream: &mut TcpStream) -> Vec<u8> {
        let mut req = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => return req,
                Ok(n) => {
                    req.extend_from_slice(&buf[..n]);
                    if req.windows(4).any(|w| w == b"\r\n\r\n") {
                        return req;
                    }
                }
                Err(_) => return req,
            }
        }
    }

    #[test]
    fn detect_quant_reconhece_padroes_comuns() {
        assert_eq!(detect_quant("ggml-small-q5_1.bin"), Some("q5_1".into()));
        assert_eq!(detect_quant("model-q4_k_m.gguf"), Some("q4_k_m".into()));
        assert_eq!(
            detect_quant("qwen2.5-7b-instruct-q4_k_m.gguf"),
            Some("q4_k_m".into())
        );
        assert_eq!(detect_quant("model-fp16.onnx"), Some("fp16".into()));
        assert_eq!(detect_quant("ggml-tiny.bin"), None);
    }

    #[test]
    fn pick_file_traducao_prefere_gguf_e_maior_arquivo() {
        let sibs = vec![
            sibling("a.onnx", Some(500)),
            sibling("b.gguf", Some(100)),
            sibling("c.gguf", Some(200)),
            sibling("README.md", None),
        ];
        let (file, size, backend) = pick_file(&sibs, ModelKind::Translation).unwrap();
        assert_eq!(file, "c.gguf");
        assert_eq!(size, Some(200));
        assert_eq!(backend, Backend::Llama);
    }

    #[test]
    fn pick_file_stt_so_aceita_ggml_ou_whisper() {
        let sibs = vec![
            sibling("model-q4_k_m.gguf", Some(300)), // LLM gguf não é STT
            sibling("ggml-tiny.bin", Some(70)),
        ];
        let (file, _, backend) = pick_file(&sibs, ModelKind::Stt).unwrap();
        assert_eq!(file, "ggml-tiny.bin");
        assert_eq!(backend, Backend::Whisper);
    }

    #[test]
    fn pick_file_sem_compativel_retorna_none() {
        let sibs = vec![sibling("model.bin", Some(10))];
        assert!(pick_file(&sibs, ModelKind::Translation).is_none());
        assert!(pick_file(&sibs, ModelKind::Stt).is_none());
    }

    #[tokio::test]
    async fn busca_mock_retorna_lista_normalizada_e_filtrada() {
        let api = MockApi::start(api_body(), 200);
        let s = HfSearch::with_base_url(&api.base_url());

        let results = s
            .search("teste", ModelKind::Translation, &hw(8))
            .await
            .expect("busca com mock deve funcionar");

        let ids: Vec<&str> = results.iter().map(|r| r.repo_id.as_str()).collect();
        assert!(ids.contains(&"org/llm-repo"), "llm gguf incluído");
        assert!(ids.contains(&"org/onnx-repo"), "onnx incluído");
        assert!(
            !ids.contains(&"org/no-compat"),
            "repo sem arquivos compatíveis excluído"
        );
        assert!(!ids.contains(&"org/whisper-repo"), "whisper ggml excluído");
        assert!(
            !ids.contains(&"org/whisper-gguf"),
            "whisper gguf excluído da busca de tradução"
        );
        assert!(
            !ids.contains(&"org/huge-llm"),
            "tamanho >= RAM disponível excluído"
        );

        let llm = results
            .iter()
            .find(|r| r.repo_id == "org/llm-repo")
            .unwrap();
        assert_eq!(llm.backend, Backend::Llama);
        assert_eq!(
            llm.file, "model-q4_k_m.gguf",
            "gguf tem precedência sobre onnx"
        );
        assert_eq!(llm.quant.as_deref(), Some("q4_k_m"));
        assert_eq!(llm.size_mb, Some(2100000000 / (1024 * 1024)));
        assert_eq!(llm.downloads, 1000);
        assert_eq!(llm.likes, 50);

        let onnx = results
            .iter()
            .find(|r| r.repo_id == "org/onnx-repo")
            .unwrap();
        assert_eq!(onnx.backend, Backend::Ort);
        assert_eq!(onnx.file, "encoder.onnx", "maior arquivo onnx");
    }

    #[tokio::test]
    async fn busca_stt_filtra_por_ggml_e_whisper() {
        let api = MockApi::start(api_body(), 200);
        let s = HfSearch::with_base_url(&api.base_url());

        let results = s
            .search("whisper", ModelKind::Stt, &hw(8))
            .await
            .expect("busca com mock deve funcionar");

        let ids: Vec<&str> = results.iter().map(|r| r.repo_id.as_str()).collect();
        assert!(ids.contains(&"org/whisper-repo"));
        assert!(ids.contains(&"org/whisper-gguf"));
        assert!(!ids.contains(&"org/llm-repo"), "llm gguf não é STT");
        assert!(!ids.contains(&"org/onnx-repo"));

        let wr = results
            .iter()
            .find(|r| r.repo_id == "org/whisper-repo")
            .unwrap();
        assert_eq!(wr.backend, Backend::Whisper);
        assert_eq!(wr.file, "ggml-small-q5_1.bin", "maior arquivo compatível");
        assert_eq!(wr.quant.as_deref(), Some("q5_1"));
    }

    #[tokio::test]
    async fn cache_de_10min_evita_requisicoes_repetidas() {
        let api = MockApi::start(api_body(), 200);
        let s = HfSearch::with_base_url(&api.base_url());

        let _ = s.search("x", ModelKind::Stt, &hw(8)).await.unwrap();
        let hits_first = api.hits.load(Ordering::SeqCst);
        assert_eq!(s.cache_len(), 1);

        let _ = s.search("x", ModelKind::Stt, &hw(8)).await.unwrap();
        assert_eq!(
            api.hits.load(Ordering::SeqCst),
            hits_first,
            "segunda busca deve vir do cache"
        );
    }

    #[tokio::test]
    async fn sem_conexao_retorna_erro_tipado_de_rede() {
        // Listener bindado e dropado → porta fechada → connection refused.
        let addr = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let s = HfSearch::with_base_url(&format!("http://{addr}"));

        let err = s
            .search("x", ModelKind::Stt, &hw(8))
            .await
            .expect_err("sem conexão deve falhar");
        assert!(
            matches!(err, SearchError::Network(_)),
            "erro tipado de rede, veio: {err}"
        );
        assert!(
            err.to_string().contains("conexão com o Hugging Face"),
            "mensagem sugere verificar a conexão: {err}"
        );
    }

    #[tokio::test]
    async fn status_nao_2xx_retorna_erro_tipado() {
        let api = MockApi::start(b"rate limited".to_vec(), 429);
        let s = HfSearch::with_base_url(&api.base_url());

        let err = s
            .search("x", ModelKind::Stt, &hw(8))
            .await
            .expect_err("429 deve falhar");
        assert!(
            matches!(
                err,
                SearchError::UnexpectedStatus(StatusCode::TOO_MANY_REQUESTS)
            ),
            "status tipado, veio: {err}"
        );
    }
}
