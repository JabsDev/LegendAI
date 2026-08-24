//! Parser estrito da resposta numerada do LLM + fallback por linha (tarefa 3.6).
//!
//! O template de prompt (3.7) pede a tradução no formato `[N] texto`, uma linha
//! por segmento. O formato numerado torna o parser robusto a respostas fora de
//! ordem: a ordem final é reconstruída pelo `id`, nunca pela posição da linha.
//! NLLB não usa este parser (não tem prompt) — só o caminho LLM.

use std::collections::HashMap;

use super::engine::{
    BatchResult, BatchSegment, TranslateError, TranslatedSegment, TranslationStatus,
};

/// Parseia uma linha `[N] texto` → `(id, texto)`. `None` se a linha não casar
/// `^\s*\[(\d+)\]\s*(.+)$` (malformada, id não-numérico ou texto vazio).
/// `ponytail:` parse manual em vez de regex — o padrão é simples o suficiente
/// e evita depender do crate `regex` (hoje transitivo, não direto).
fn parse_numbered_line(line: &str) -> Option<(u32, String)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('[')?;
    let close = rest.find(']')?;
    let digits = &rest[..close];
    if digits.is_empty() {
        return None;
    }
    let id = digits.parse::<u32>().ok()?;
    let text = rest[close + 1..].trim();
    if text.is_empty() {
        return None;
    }
    Some((id, text.to_string()))
}

/// Converte a resposta bruta do LLM num `BatchResult` para `segments`.
///
/// Para cada segmento, na ordem do lote: linha `[N]` com N == id do segmento →
/// `Ok` com o texto traduzido; ausente/malformada → `Retry` **preservando o
/// texto original** em `text` (o caller re-tenta com `translate_with_retry`).
/// Linhas com id fora do lote, duplicadas ou malformadas são ignoradas (logadas
/// em debug) e nunca viram segmentos — o formato `[N]` é o único contrato.
pub fn parse_batch_response(response: &str, segments: &[BatchSegment]) -> BatchResult {
    let mut by_id: HashMap<u32, String> = HashMap::new();
    for (lineno, line) in response.lines().enumerate() {
        match parse_numbered_line(line) {
            Some((id, text)) => {
                if let std::collections::hash_map::Entry::Vacant(e) = by_id.entry(id) {
                    e.insert(text);
                } else {
                    tracing::debug!(
                        "id {id} duplicado (linha {}); mantendo 1ª ocorrência",
                        lineno + 1
                    );
                }
            }
            None => {
                if !line.trim().is_empty() {
                    tracing::debug!(
                        "linha {} da resposta do LLM não casa [N]: {:?}",
                        lineno + 1,
                        line
                    );
                }
            }
        }
    }

    let translations = segments
        .iter()
        .map(|seg| match by_id.remove(&seg.id) {
            Some(text) => TranslatedSegment {
                id: seg.id,
                text,
                status: TranslationStatus::Ok,
            },
            None => TranslatedSegment {
                id: seg.id,
                text: seg.text.clone(),
                status: TranslationStatus::Retry,
            },
        })
        .collect();
    BatchResult { translations }
}

/// Traduz `segments` com fallback por linha (até `max_attempts` tentativas).
///
/// A cada tentativa chama `respond(&pending)` — que monta o prompt (3.7) e chama
/// a engine (3.3) — e parseia a resposta com `parse_batch_response`. Segmentos
/// que saem `Ok` são coletados; os marcados `Retry` (linha corrompida) vão para
/// a próxima tentativa num lote menor contendo só eles. Persistindo a falha
/// após `max_attempts`, o segmento vira `KeptOriginal` **mantendo o texto
/// original** (nunca descarta texto). Um `Err` de `respond` (falha de backend)
/// propaga para o caller.
pub fn translate_with_retry(
    segments: &[BatchSegment],
    max_attempts: usize,
    mut respond: impl FnMut(&[BatchSegment]) -> Result<String, TranslateError>,
) -> Result<BatchResult, TranslateError> {
    let max_attempts = max_attempts.max(1);
    let mut pending: Vec<BatchSegment> = segments.to_vec();
    let mut done: Vec<TranslatedSegment> = Vec::new();

    for attempt in 0..max_attempts {
        if pending.is_empty() {
            break;
        }
        let response = respond(&pending)?;
        let parsed = parse_batch_response(&response, &pending);
        let last_attempt = attempt + 1 == max_attempts;

        let mut next: Vec<BatchSegment> = Vec::new();
        for seg in pending {
            let entry = parsed
                .translations
                .iter()
                .find(|t| t.id == seg.id)
                .expect("parse_batch_response cobre todos os ids do lote");
            match entry.status {
                TranslationStatus::Ok => done.push(entry.clone()),
                TranslationStatus::Retry if !last_attempt => next.push(seg),
                _ => {
                    tracing::warn!(
                        "segmento {} não traduzido após {} tentativas; mantendo original",
                        seg.id,
                        attempt + 1
                    );
                    done.push(TranslatedSegment {
                        id: seg.id,
                        text: seg.text,
                        status: TranslationStatus::KeptOriginal,
                    });
                }
            }
        }
        pending = next;
    }

    done.sort_by_key(|t| t.id);
    Ok(BatchResult { translations: done })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(n: usize) -> Vec<BatchSegment> {
        (1..=n as u32)
            .map(|id| BatchSegment {
                id,
                text: format!("original {id}"),
                context: vec![],
            })
            .collect()
    }

    /// Monta uma resposta `[N] texto` para os segmentos pendentes; ids onde `ok`
    /// retorna `false` ganham uma linha malformada (simula corrupção do LLM).
    fn ok_response(pending: &[BatchSegment], ok: impl Fn(u32) -> bool) -> String {
        pending
            .iter()
            .map(|s| {
                if ok(s.id) {
                    format!("[{}] tradução {}", s.id, s.id)
                } else {
                    "linha quebrada sem número".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn resposta_perfeita_todas_ok_fora_de_ordem() {
        let result = parse_batch_response("[3] três\n[1] um\n[2] dois\n", &segs(3));
        assert_eq!(result.translations.len(), 3);
        assert!(result
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::Ok));
        assert_eq!(result.translations[0].text, "um");
        assert_eq!(result.translations[1].text, "dois");
        assert_eq!(result.translations[2].text, "três");
    }

    #[test]
    fn linha_malformada_marca_retry_com_texto_original() {
        let result = parse_batch_response("[1] ok\nlinha sem número\n", &segs(2));
        assert_eq!(result.translations[0].status, TranslationStatus::Ok);
        assert_eq!(result.translations[1].status, TranslationStatus::Retry);
        assert_eq!(result.translations[1].text, "original 2");
    }

    #[test]
    fn id_fora_do_range_e_ignorado() {
        let result = parse_batch_response("[99] alien\n[1] ok\n", &segs(2));
        assert_eq!(result.translations[0].status, TranslationStatus::Ok);
        assert_eq!(result.translations[1].status, TranslationStatus::Retry);
    }

    #[test]
    fn id_duplicado_usa_primeira_ocorrencia() {
        let result = parse_batch_response("[1] primeira\n[1] segunda\n", &segs(1));
        assert_eq!(result.translations[0].text, "primeira");
        assert_eq!(result.translations[0].status, TranslationStatus::Ok);
    }

    #[test]
    fn resposta_vazia_marca_todos_retry() {
        let result = parse_batch_response("", &segs(3));
        assert!(result
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::Retry));
    }

    #[test]
    fn segmentos_vazios_retorna_resultado_vazio() {
        let result = parse_batch_response("[1] x", &[]);
        assert!(result.translations.is_empty());
    }

    #[test]
    fn linha_sem_texto_ou_id_invalido_e_malformada() {
        let result = parse_batch_response("[1]\n[abc] x\n[ ] y\n", &segs(1));
        assert_eq!(result.translations[0].status, TranslationStatus::Retry);
    }

    #[test]
    fn crlf_espacos_e_texto_com_colchetes_tolerados() {
        let result = parse_batch_response("  [1]   texto [entre] colchetes  \r\n", &segs(1));
        assert_eq!(result.translations[0].text, "texto [entre] colchetes");
        assert_eq!(result.translations[0].status, TranslationStatus::Ok);
    }

    #[test]
    fn saida_perfeita_sem_retentativas() {
        let mut calls = 0;
        let result = translate_with_retry(&segs(3), 2, |pending| {
            calls += 1;
            Ok(ok_response(pending, |_| true))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(result
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::Ok));
    }

    #[test]
    fn linhas_corrompidas_retentadas_ate_ok() {
        let mut calls = 0;
        let mut second_pending: Vec<u32> = Vec::new();
        let result = translate_with_retry(&segs(10), 2, |pending| {
            calls += 1;
            if calls == 2 {
                second_pending = pending.iter().map(|s| s.id).collect();
            }
            Ok(ok_response(pending, |id| {
                !(calls == 1 && (id == 3 || id == 7))
            }))
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(second_pending, vec![3, 7]); // só os retry vão ao 2º lote
        assert_eq!(result.translations.len(), 10);
        assert!(result
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::Ok));
        let text3 = result
            .translations
            .iter()
            .find(|t| t.id == 3)
            .map(|t| t.text.clone())
            .unwrap();
        assert_eq!(text3, "tradução 3");
    }

    #[test]
    fn falha_persistente_vira_kept_original_sem_descartar_texto() {
        let mut calls = 0;
        let result = translate_with_retry(&segs(4), 2, |pending| {
            calls += 1;
            Ok(ok_response(pending, |id| id != 2))
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(result.translations.len(), 4);
        for t in &result.translations {
            if t.id == 2 {
                assert_eq!(t.status, TranslationStatus::KeptOriginal);
                assert_eq!(t.text, "original 2"); // nunca descarta texto
            } else {
                assert_eq!(t.status, TranslationStatus::Ok);
            }
        }
    }

    #[test]
    fn max_attempts_zero_tratado_como_um() {
        let mut calls = 0;
        let result = translate_with_retry(&segs(2), 0, |pending| {
            calls += 1;
            Ok(ok_response(pending, |_| false))
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(result
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::KeptOriginal));
    }

    #[test]
    fn segmentos_vazios_nao_chama_respond() {
        let mut calls = 0;
        let result = translate_with_retry(&[], 2, |_| {
            calls += 1;
            Ok(String::new())
        })
        .unwrap();
        assert_eq!(calls, 0);
        assert!(result.translations.is_empty());
    }

    #[test]
    fn erro_do_backend_propaga() {
        let result = translate_with_retry(&segs(2), 2, |_| {
            Err(TranslateError::Backend("modelo quebrou".into()))
        });
        assert!(result.is_err());
    }
}
