//! Batcher de segmentos numerados (tarefa 3.5).
//!
//! Divide a lista de segmentos em lotes de até 10, preservando ordem. Cada
//! lote forma a entrada de um `BatchRequest`, com `context` = os últimos
//! `context_size` segmentos **anteriores** (nunca futuros) para coerência de
//! nomes/pronomes (parte central do template 3.7).
//!
//! Os ids são sequenciais globais (1-based) da legenda inteira — reconstruir a
//! ordem original é trivial: concatenar os lotes na ordem (ou ordenar por id,
//! como o parser 3.6 faz ao mapear a resposta `[N]` do LLM).

use super::engine::BatchSegment;
use crate::domain::Segment;

/// Lote de segmentos a traduzir junto — `segments` alimenta um `BatchRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub segments: Vec<BatchSegment>,
}

/// Tamanho de lote padrão (nota do plano: ~10 segmentos/lote).
pub const DEFAULT_BATCH_SIZE: usize = 10;
/// Quantidade de segmentos anteriores incluídos como contexto (nota: 2-3).
pub const DEFAULT_CONTEXT_SIZE: usize = 3;

/// Divide `segments` em lotes de até `batch_size`, preservando ordem.
///
/// O contexto do segmento i são os textos dos segmentos `[i-context_size, i)`
/// — só anteriores, cortado nas bordas (primeiros segmentos têm contexto
/// menor, o primeiro tem contexto vazio). `batch_size == 0` é tratado como 1
/// para não produzir lotes infinitos/vazios.
pub fn chunk_segments(segments: &[Segment], batch_size: usize, context_size: usize) -> Vec<Batch> {
    let batch_size = batch_size.max(1);
    let mut batches: Vec<Batch> = Vec::new();
    for (idx, segment) in segments.iter().enumerate() {
        if batches
            .last()
            .is_none_or(|b| b.segments.len() >= batch_size)
        {
            batches.push(Batch {
                segments: Vec::new(),
            });
        }
        let start = idx.saturating_sub(context_size);
        let context = segments[start..idx]
            .iter()
            .map(|s| s.text.clone())
            .collect();
        batches
            .last_mut()
            .expect("lote recém-inserido")
            .segments
            .push(BatchSegment {
                id: (idx + 1) as u32,
                text: segment.text.clone(),
                context,
            });
    }
    batches
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Language, Timestamp};

    fn segs(n: usize) -> Vec<Segment> {
        (0..n)
            .map(|i| {
                let ms = (i as u64) * 1000;
                Segment::new(
                    format!("seg {i}"),
                    Timestamp::from_ms(ms),
                    Timestamp::from_ms(ms + 500),
                    Language::Pt,
                )
                .unwrap()
            })
            .collect()
    }

    fn texts(batches: &[Batch]) -> Vec<&str> {
        batches
            .iter()
            .flat_map(|b| b.segments.iter())
            .map(|s| s.text.as_str())
            .collect()
    }

    fn ids(batches: &[Batch]) -> Vec<u32> {
        batches
            .iter()
            .flat_map(|b| b.segments.iter())
            .map(|s| s.id)
            .collect()
    }

    #[test]
    fn vazio_retorna_zero_lotes() {
        assert!(chunk_segments(&[], 10, 3).is_empty());
    }

    #[test]
    fn menos_que_um_lote_vira_lote_unico_preservando_ordem() {
        let batches = chunk_segments(&segs(3), 10, 3);
        assert_eq!(batches.len(), 1);
        assert_eq!(texts(&batches), vec!["seg 0", "seg 1", "seg 2"]);
        assert_eq!(ids(&batches), vec![1, 2, 3]);
    }

    #[test]
    fn lote_nunca_ultrapassa_10_segmentos() {
        let batches = chunk_segments(&segs(25), 10, 3);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].segments.len(), 10);
        assert_eq!(batches[1].segments.len(), 10);
        assert_eq!(batches[2].segments.len(), 5);
        for b in &batches {
            assert!(b.segments.len() <= 10);
        }
    }

    #[test]
    fn ids_globais_sequenciais_preservam_ordem_original() {
        let batches = chunk_segments(&segs(25), 10, 3);
        assert_eq!(ids(&batches), (1..=25).collect::<Vec<u32>>());
    }

    #[test]
    fn contexto_so_de_segmentos_anteriores() {
        let batches = chunk_segments(&segs(5), 10, 3);
        let contexts: Vec<Vec<String>> = batches[0]
            .segments
            .iter()
            .map(|s| s.context.clone())
            .collect();
        assert_eq!(contexts[0], Vec::<String>::new()); // primeiro: sem contexto
        assert_eq!(contexts[1], vec!["seg 0"]);
        assert_eq!(contexts[2], vec!["seg 0", "seg 1"]);
        assert_eq!(contexts[3], vec!["seg 0", "seg 1", "seg 2"]);
        assert_eq!(contexts[4], vec!["seg 1", "seg 2", "seg 3"]);
    }

    #[test]
    fn contexto_na_borda_entre_lotes() {
        let batches = chunk_segments(&segs(12), 10, 3);
        // 1º segmento do 2º lote ("seg 10", id 11): contexto = últimos 3 do lote anterior
        assert_eq!(
            batches[1].segments[0].context,
            vec!["seg 7", "seg 8", "seg 9"]
        );
    }

    #[test]
    fn contexto_zero_nao_inclui_anterior() {
        let batches = chunk_segments(&segs(4), 10, 0);
        assert!(batches[0].segments.iter().all(|s| s.context.is_empty()));
    }

    #[test]
    fn batch_size_zero_e_tratado_como_1() {
        let batches = chunk_segments(&segs(3), 0, 1);
        assert_eq!(batches.len(), 3);
        assert_eq!(ids(&batches), vec![1, 2, 3]);
    }
}
