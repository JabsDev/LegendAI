//! Fila de processamento de vídeos com pool de workers (tarefas 4.9 e 5.3).
//!
//! A fila guarda itens (vídeo + origem + opções) com estado por item
//! (`pending`/`running`/`done`/`error`/`cancelled`) e um **pool de workers**
//! (`std::thread` dedicados — nunca bloqueiam a thread principal do Tauri nem o
//! runtime async) que processa **até N itens em paralelo**, onde N é o limite
//! de concorrência por tier (5.3): Tier 1 = 1 worker, Tier 2 = 2, Tier 3 = 3 —
//! paralelismo multiplica o consumo de RAM, então o semáforo por tier é o
//! guarda (um modelo STT + uma engine por job simultâneo já pesam).
//!
//! Cada item é executado por [`crate::commands::pipeline::execute_job`] (o
//! mesmo pipeline da 4.3: extrair → transcrever → traduzir → formatar →
//! exportar), que já emite `pipeline-progress` por etapa — filtrados pelo `id`
//! do item — e retorna o [`PipelineFinished`] para o worker gravar o estado.
//!
//! **Isolamento entre jobs paralelos:** o config e o hardware são carregados
//! por job dentro de `execute_job`/`run_job` (`AppConfig::load_or_default()` +
//! `detect()` por chamada) — cada worker roda com cache/sessão próprios, sem
//! estado compartilhado além da própria fila; diretórios temporários e ids de
//! job são únicos por item (`job-<ms>-<n>`), então resultados não se misturam.
//!
//! Eventos emitidos:
//! - `queue-updated` → lista completa de itens (após enfileirar/remover/
//!   iniciar/concluir/cancelar um item) — fonte de verdade da UI;
//! - `pipeline-progress` / `pipeline-finished` → progresso em tempo real do
//!   item em execução (payloads da 4.3, `job_id` = `item.id`).
//!
//! Comandos IPC: `queue_list` (poll), `queue_enqueue`, `queue_cancel`,
//! `queue_remove`. Cancelamento é por item (token cooperativo do item em
//! execução) — cancelar um job não afeta os demais, nem os `pending`.
//!
//! `ponytail:` o pool é fixo em `MAX_WORKER_THREADS` e a concorrência real é
//! o semáforo (permissões = `max_workers_for_tier`) — guarda explícito do
//! limite por tier; itens reclamados por threads sem permissão ficam
//! marcados `running` aguardando a vez (cancelamento ainda funciona, o token
//! já existe). "Reuso de modelo carregado entre jobs do mesmo vídeo (worker
//! keep-alive)" não implementado: segurar modelos na RAM entre jobs
//! multiplicaria o consumo permanente e contraria o guarda de RAM por tier
//! (cada job já faz o swap 3.8 e dropa os modelos ao terminar).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

use crate::commands::pipeline::{execute_job, PipelineOptions, PipelineSource};
use crate::errors::ErrorDetail;
use crate::hardware::detect::detect;
use crate::hardware::tier::{tier_for, Tier};
use crate::pipeline::steps::{PipelineFinished, PipelineStep, PipelineSummary};

/// Estado de um item da fila.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState {
    /// Aguardando um worker (na ordem da fila).
    Pending,
    /// Em processamento (até N simultâneos, N = limite do tier).
    Running,
    /// Concluído com sucesso (`summary` preenchido).
    Done,
    /// Falhou (`error` traz o [`ErrorDetail`] com código estável 4.8).
    Error,
    /// Cancelado pelo usuário (estado limpo, não erro).
    Cancelled,
}

/// Item da fila serializado para a UI (evento `queue-updated` e `queue_list`).
/// `source`/`options` não cruzam IPC — são usados só pelo worker.
#[derive(Debug, Clone, Serialize)]
pub struct QueueItem {
    pub id: String,
    pub input_path: String,
    pub state: QueueState,
    /// Etapa atual (item em execução).
    pub step: Option<PipelineStep>,
    /// Porcentagem da etapa atual (0-100).
    pub pct: u8,
    pub detail: Option<String>,
    pub summary: Option<PipelineSummary>,
    pub error: Option<ErrorDetail>,
    #[serde(skip)]
    pub source: PipelineSource,
    #[serde(skip)]
    pub options: PipelineOptions,
}

impl QueueItem {
    fn new(
        id: String,
        input_path: String,
        source: PipelineSource,
        options: PipelineOptions,
    ) -> Self {
        Self {
            id,
            input_path,
            state: QueueState::Pending,
            step: None,
            pct: 0,
            detail: None,
            summary: None,
            error: None,
            source,
            options,
        }
    }
}

/// Estado global da fila: itens em ordem de enfileiramento + os itens em
/// execução (id + token de cancelamento cooperativo, um por worker ativo) +
/// condvar de wake compartilhada pelos workers do pool.
struct JobQueue {
    items: Mutex<Vec<QueueItem>>,
    running: Mutex<Vec<(String, CancellationToken)>>,
    wake: Condvar,
}

static QUEUE: OnceLock<JobQueue> = OnceLock::new();
/// Nº de workers já iniciados (lazy na primeira enqueue).
static WORKERS: OnceLock<usize> = OnceLock::new();

fn job_queue() -> &'static JobQueue {
    QUEUE.get_or_init(|| JobQueue {
        items: Mutex::new(Vec::new()),
        running: Mutex::new(Vec::new()),
        wake: Condvar::new(),
    })
}

/// Nº de workers simultâneos por tier (tarefa 5.3): Tier 1 = 1 worker (RAM
/// apertada — um modelo STT + uma engine de tradução por job já pesam);
/// Tier 2/3 = 2-3. É o nº de permissões do semáforo do pool — o guarda do
/// limite de concorrência por tier.
pub fn max_workers_for_tier(tier: Tier) -> usize {
    match tier {
        Tier::Tier1 => 1,
        Tier::Tier2 => 2,
        Tier::Tier3 => 3,
    }
}

/// Semáforo de contagem mínimo (`Mutex` + `Condvar`). O `std::sync::Semaphore`
/// ainda não é estável no toolchain e o do tokio só expõe acquire async — este
/// é o guarda de concorrência do pool (permissões = limite do tier).
struct Semaphore {
    permits: Mutex<usize>,
    cv: Condvar,
}

impl Semaphore {
    fn new(permits: usize) -> Self {
        Self {
            permits: Mutex::new(permits),
            cv: Condvar::new(),
        }
    }

    /// Bloqueia o thread corrente até haver permissão; o guard devolve a
    /// permissão no drop (fim do job no worker).
    fn acquire(&self) -> SemaphoreGuard<'_> {
        let mut permits = self.permits.lock().unwrap();
        while *permits == 0 {
            permits = self.cv.wait(permits).unwrap();
        }
        *permits -= 1;
        SemaphoreGuard { semaphore: self }
    }
}

/// Guard que devolve a permissão ao semáforo ao sair de escopo.
struct SemaphoreGuard<'a> {
    semaphore: &'a Semaphore,
}

impl Drop for SemaphoreGuard<'_> {
    fn drop(&mut self) {
        let mut permits = self.semaphore.permits.lock().unwrap();
        *permits += 1;
        self.semaphore.cv.notify_one();
    }
}

/// Tamanho fixo do pool de threads (o maior dos tiers). A concorrência real é
/// o semáforo (permissões = `max_workers_for_tier`), não o nº de threads —
/// o pool é fixo para que uma mudança futura de limite não respawne threads.
const MAX_WORKER_THREADS: usize = 3;

/// Serializa a lista completa de itens no evento `queue-updated`.
fn emit_queue_updated(app: &tauri::AppHandle) {
    let items = job_queue().items.lock().unwrap();
    let _ = app.emit("queue-updated", &*items);
}

/// Atualiza o progresso de um item pelo id (chamado por `emit_progress` da 4.3
/// a cada avanço real de etapa). No-op se o job não for da fila.
pub(crate) fn update_progress(id: &str, step: PipelineStep, pct: u8, detail: Option<&str>) {
    let mut items = job_queue().items.lock().unwrap();
    if let Some(item) = items.iter_mut().find(|i| i.id == id) {
        item.step = Some(step);
        item.pct = pct.clamp(0, 100);
        item.detail = detail.map(String::from);
    }
}

/// Aplica o resultado do pipeline ao item (transição de estado final).
fn apply_finished(item: &mut QueueItem, finished: &PipelineFinished) {
    item.step = None;
    item.detail = None;
    if finished.ok {
        item.state = QueueState::Done;
        item.summary = finished.summary.clone();
        item.error = None;
    } else if finished.cancelled {
        item.state = QueueState::Cancelled;
        item.summary = None;
        item.error = None;
    } else {
        item.state = QueueState::Error;
        item.summary = None;
        item.error = finished.error.clone();
    }
}

/// Testável: índice do próximo item `pending` na ordem da fila.
fn next_pending_index(items: &[QueueItem]) -> Option<usize> {
    items.iter().position(|i| i.state == QueueState::Pending)
}

/// Reclama o próximo item `pending` (marca `Running`) sob o lock já adquirido.
fn claim_next_locked(items: &mut [QueueItem]) -> Option<QueueItem> {
    let idx = next_pending_index(items)?;
    let mut item = items[idx].clone();
    item.state = QueueState::Running;
    item.step = None;
    item.pct = 0;
    item.detail = None;
    items[idx] = item.clone();
    Some(item)
}

/// Token de cancelamento do item em execução (se for o caso).
fn running_token(id: &str) -> Option<CancellationToken> {
    job_queue()
        .running
        .lock()
        .unwrap()
        .iter()
        .find(|(running_id, _)| running_id == id)
        .map(|(_, token)| token.clone())
}

/// Inicia o pool de workers na primeira enqueue (lazy): `MAX_WORKER_THREADS`
/// threads compartilhando um `Arc<Semaphore>` com `max_workers_for_tier`
/// permissões — o semáforo é o guarda real do limite de concorrência por tier
/// (RAM). Tier 1 (1 permissão): as 3 threads rodam, mas só 1 job por vez.
fn ensure_workers(app: &tauri::AppHandle) -> usize {
    *WORKERS.get_or_init(|| {
        let tier = tier_for(&detect());
        let permits = max_workers_for_tier(tier);
        let semaphore = Arc::new(Semaphore::new(permits));
        for _ in 0..MAX_WORKER_THREADS {
            let handle = app.clone();
            let sem = semaphore.clone();
            std::thread::Builder::new()
                .name("legendai-queue".into())
                .spawn(move || worker_loop(handle, sem))
                .expect("não foi possível iniciar o worker da fila");
        }
        permits
    })
}

/// Loop de um worker do pool: espera um item `pending` (condvar — o wait é
/// atômico com o lock, sem wake perdido), adquire a permissão do semáforo do
/// pool (bloqueia até um slot do tier liberar — o item já marcado `running`
/// aguarda a vez) e executa o pipeline. Sem itens, bloqueia até um
/// `queue_enqueue` sinalizar com `notify_all`.
fn worker_loop(app: tauri::AppHandle, semaphore: Arc<Semaphore>) {
    loop {
        let item = {
            let queue = job_queue();
            let mut items = queue.items.lock().unwrap();
            loop {
                if let Some(item) = claim_next_locked(&mut items) {
                    break item;
                }
                items = queue.wake.wait(items).unwrap();
            }
        };

        let _permit = semaphore.acquire();
        let token = CancellationToken::new();
        {
            let mut running = job_queue().running.lock().unwrap();
            running.push((item.id.clone(), token.clone()));
        }
        emit_queue_updated(&app);

        let finished = execute_job(
            &app,
            &item.id,
            &item.input_path,
            &item.source,
            Some(item.options),
            &token,
        );

        {
            let mut running = job_queue().running.lock().unwrap();
            running.retain(|(id, _)| id != &item.id);
        }
        {
            let mut items = job_queue().items.lock().unwrap();
            if let Some(entry) = items.iter_mut().find(|i| i.id == item.id) {
                apply_finished(entry, &finished);
            }
        }

        let _ = app.emit("pipeline-finished", &finished);
        emit_queue_updated(&app);
    }
}

/// Ids sequenciais únicos (arquivo-safe — viram nome do dir temp do job).
fn next_job_id() -> String {
    let n = JOB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("job-{ms}-{n}")
}

static JOB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Lista os itens da fila (poll inicial da UI).
#[tauri::command(rename_all = "snake_case")]
pub fn queue_list() -> Vec<QueueItem> {
    job_queue().items.lock().unwrap().clone()
}

/// Adiciona um vídeo à fila. Retorna na hora (UI não bloqueia); um worker do
/// pool processa quando chegar a vez, emitindo `queue-updated` e `pipeline-*`.
#[tauri::command(rename_all = "snake_case")]
pub fn queue_enqueue(
    app: tauri::AppHandle,
    input_path: String,
    source: PipelineSource,
    options: Option<PipelineOptions>,
) -> Result<QueueItem, String> {
    if !Path::new(&input_path).exists() {
        return Err(format!("arquivo não encontrado: `{input_path}`"));
    }
    let item = QueueItem::new(
        next_job_id(),
        input_path,
        source,
        options.unwrap_or_default(),
    );
    job_queue().items.lock().unwrap().push(item.clone());
    ensure_workers(&app);
    job_queue().wake.notify_all();
    emit_queue_updated(&app);
    Ok(item)
}

/// Cancela o item em execução (cooperativo: para na próxima checagem do token
/// — abort do whisper ou entre lotes de tradução). Itens `pending` não são
/// afetados — cancelar um job não cancela os demais.
#[tauri::command(rename_all = "snake_case")]
pub fn queue_cancel(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let token = running_token(&id)
        .ok_or_else(|| format!("nenhum processamento em andamento para o item `{id}`"))?;
    token.cancel();
    let _ = app;
    Ok(())
}

/// Remove um item da fila. Itens em execução exigem cancelamento antes.
#[tauri::command(rename_all = "snake_case")]
pub fn queue_remove(app: tauri::AppHandle, id: String) -> Result<(), String> {
    {
        let running = job_queue().running.lock().unwrap();
        if running.iter().any(|(running_id, _)| running_id == &id) {
            return Err("o item está em processamento — cancele antes de remover".into());
        }
    }
    let mut items = job_queue().items.lock().unwrap();
    match items.iter().position(|i| i.id == id) {
        Some(idx) if items[idx].state != QueueState::Running => {
            items.remove(idx);
        }
        Some(_) => return Err("o item está em processamento — cancele antes de remover".into()),
        None => return Err(format!("item `{id}` não está na fila")),
    }
    drop(items);
    emit_queue_updated(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    fn sample_item(id: &str, state: QueueState) -> QueueItem {
        let mut item = QueueItem::new(
            id.into(),
            format!("/videos/{id}.mp4"),
            PipelineSource::Audio { track_index: 0 },
            PipelineOptions::default(),
        );
        item.state = state;
        item
    }

    fn clear_queue() {
        job_queue().items.lock().unwrap().clear();
        job_queue().running.lock().unwrap().clear();
    }

    #[test]
    fn next_job_id_gera_ids_unicos_sequenciais() {
        let a = next_job_id();
        let b = next_job_id();
        assert_ne!(a, b);
        assert!(a.starts_with("job-"));
    }

    #[test]
    fn queue_item_serializa_para_ipc() {
        let mut item = sample_item("j1", QueueState::Running);
        item.step = Some(PipelineStep::Transcribe);
        item.pct = 50;
        let value = serde_json::to_value(&item).unwrap();
        assert_eq!(value["id"], "j1");
        assert_eq!(value["state"], "running");
        assert_eq!(value["step"], "transcribe");
        assert_eq!(value["pct"], 50);
        // Campos internos do worker não cruzam IPC.
        assert!(value.get("source").is_none());
        assert!(value.get("options").is_none());
    }

    #[test]
    fn apply_finished_mapeia_estados_finais() {
        let mut item = sample_item("j1", QueueState::Running);

        // ok → done + summary, sem erro.
        let finished = PipelineFinished {
            job_id: "j1".into(),
            ok: true,
            cancelled: false,
            error: None,
            summary: Some(PipelineSummary {
                output_path: "/tmp/out.srt".into(),
                duration_secs: 90.0,
                segments: 42,
                source_lang: "pt".into(),
                target_lang: "en".into(),
                kept_original: 0,
                stats: Default::default(),
            }),
        };
        apply_finished(&mut item, &finished);
        assert_eq!(item.state, QueueState::Done);
        assert!(item.summary.is_some());
        assert!(item.error.is_none());
        assert!(item.step.is_none());

        // cancelado → cancelled, sem erro e sem resumo.
        let finished = PipelineFinished {
            job_id: "j2".into(),
            ok: false,
            cancelled: true,
            error: None,
            summary: None,
        };
        apply_finished(&mut item, &finished);
        assert_eq!(item.state, QueueState::Cancelled);
        assert!(item.summary.is_none());
        assert!(item.error.is_none());

        // erro → error + ErrorDetail (código estável).
        let finished = PipelineFinished {
            job_id: "j3".into(),
            ok: false,
            cancelled: false,
            error: Some(ErrorDetail {
                code: "no_speech",
                message: "nenhuma fala detectada".into(),
                hint: None,
            }),
            summary: None,
        };
        apply_finished(&mut item, &finished);
        assert_eq!(item.state, QueueState::Error);
        assert_eq!(item.error.as_ref().unwrap().code, "no_speech");
        assert!(item.summary.is_none());
    }

    #[test]
    fn next_pending_respeita_ordem_da_fila() {
        let items = vec![
            sample_item("done", QueueState::Done),
            sample_item("pending1", QueueState::Pending),
            sample_item("pending2", QueueState::Pending),
        ];
        // Apenas `pending` concorrem, na ordem de enfileiramento.
        assert_eq!(next_pending_index(&items), Some(1));
    }

    #[test]
    fn next_pending_vazio_sem_itens_pendentes() {
        let items = vec![
            sample_item("a", QueueState::Running),
            sample_item("b", QueueState::Done),
        ];
        assert_eq!(next_pending_index(&items), None);
        assert_eq!(next_pending_index(&[]), None);
    }

    #[test]
    fn max_workers_por_tier_respeita_limite_de_ram() {
        // Tier 1 (RAM apertada) = 1 worker; Tier 2/3 = 2-3 (tarefa 5.3).
        assert_eq!(max_workers_for_tier(Tier::Tier1), 1);
        assert_eq!(max_workers_for_tier(Tier::Tier2), 2);
        assert_eq!(max_workers_for_tier(Tier::Tier3), 3);
    }

    #[test]
    fn claim_entrega_itens_distintos_a_workers_concorrentes() {
        clear_queue();
        {
            let mut items = job_queue().items.lock().unwrap();
            items.push(sample_item("j1", QueueState::Pending));
            items.push(sample_item("j2", QueueState::Pending));
            items.push(sample_item("j3", QueueState::Pending));
        }

        // 3 workers reclamando ao mesmo tempo → cada um pega um item distinto
        // (o lock da fila impede dois workers de reclamar o mesmo item).
        let handles: Vec<_> = (0..3)
            .map(|_| {
                std::thread::spawn(|| {
                    claim_next_locked(&mut job_queue().items.lock().unwrap()).map(|i| i.id)
                })
            })
            .collect();
        let mut ids: Vec<String> = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Option<Vec<_>>>()
            .unwrap();
        ids.sort();
        assert_eq!(
            ids,
            vec!["j1".to_string(), "j2".to_string(), "j3".to_string()]
        );
        // Nenhum item pendente sobrou.
        assert_eq!(next_pending_index(&job_queue().items.lock().unwrap()), None);

        clear_queue();
    }

    #[test]
    fn semaforo_limita_concorrencia_ao_tier() {
        // 4 "jobs" com limite 2 (Tier 2): pico de concorrência ≤ 2 e tempo
        // total ≈ 2× o de um job (execução em paralelo, não serial).
        let sem = Arc::new(Semaphore::new(2));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let start = Instant::now();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let sem = sem.clone();
                let active = active.clone();
                let peak = peak.clone();
                std::thread::spawn(move || {
                    let _permit = sem.acquire();
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(150));
                    active.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(peak.load(Ordering::SeqCst) <= 2, "pico > limite do tier");
        // 4 jobs em 2 slots → pelo menos 2 batches (≈2× o tempo de um job).
        assert!(start.elapsed() >= Duration::from_millis(250));
    }

    #[test]
    fn cancel_so_atinge_item_em_execucao() {
        let token_a = CancellationToken::new();
        let token_b = CancellationToken::new();
        {
            let mut running = job_queue().running.lock().unwrap();
            // Dois jobs em execução (pool 5.3) — um não afeta o outro.
            running.push(("job-a".into(), token_a.clone()));
            running.push(("job-b".into(), token_b.clone()));
        }
        // Item que não está rodando → erro claro (nenhum token é cancelado).
        assert!(running_token("job-parada").is_none());
        // Item em execução → token retornado e cancelado (só ele).
        let t = running_token("job-a").unwrap();
        t.cancel();
        assert!(token_a.is_cancelled());
        assert!(!token_b.is_cancelled());
        // Limpeza para não vazar estado entre testes.
        clear_queue();
    }
}
