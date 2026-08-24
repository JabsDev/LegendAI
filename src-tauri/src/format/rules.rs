use serde::{Deserialize, Serialize};

use crate::domain::{Language, Subtitle, Timestamp};
use crate::format::line_breaker::break_lines;

/// Máximo de linhas por legenda.
pub const MAX_LINES: usize = 2;
/// Máximo de caracteres por linha.
pub const MAX_CHARS_PER_LINE: usize = 42;
/// Duração mínima (ms): subtítulos mais curtos têm o `end` estendido.
pub const MIN_DURATION_MS: u64 = 1_000;
/// Duração máxima (ms): subtítulos mais longos são re-partidos em mais legendas.
pub const MAX_DURATION_MS: u64 = 7_000;
/// Janela alvo de velocidade de leitura (CPS). O piso não é forçado — nunca se
/// remove tempo de leitura; apenas o teto é garantido estendendo o `end`.
pub const TARGET_CPS_MIN: f64 = 15.0;
pub const TARGET_CPS_MAX: f64 = 25.0;

/// Uma legenda formatada conforme as regras profissionais.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormattedSubtitle {
    pub index: u32,
    pub lines: Vec<String>,
    pub start_ms: Timestamp,
    pub end_ms: Timestamp,
    pub language: Language,
}

impl FormattedSubtitle {
    /// Texto completo (linhas unidas por espaço), como o espectador lê.
    pub fn text(&self) -> String {
        self.lines.join(" ")
    }

    pub fn duration_ms(&self) -> u64 {
        self.end_ms.as_ms().saturating_sub(self.start_ms.as_ms())
    }

    /// Velocidade de leitura em caracteres por segundo.
    pub fn cps(&self) -> f64 {
        cps(&self.text(), self.duration_ms())
    }
}

/// Velocidade de leitura em caracteres por segundo.
pub fn cps(text: &str, duration_ms: u64) -> f64 {
    if duration_ms == 0 {
        return 0.0;
    }
    text.chars().count() as f64 / (duration_ms as f64 / 1000.0)
}

/// Aplica as regras profissionais de formatação a uma lista de legendas.
///
/// Regras (ver constantes acima):
/// * máximo de `MAX_LINES` linhas e `MAX_CHARS_PER_LINE` chars por linha
/// * duração mínima `MIN_DURATION_MS` (end estendido) e máxima `MAX_DURATION_MS`
///   (texto re-partido em mais legendas, tempo proporcional aos caracteres)
/// * velocidade ≤ `TARGET_CPS_MAX` (end estendido)
/// * sem sobreposição entre legendas consecutivas
///
/// O timing original é preservado quando as regras já são satisfeitas. Limites
/// do MVP: palavras isoladas > 42 chars são quebradas por caractere; o
/// re-posicionamento temporal usa proporção por caracteres (não modela pausas
/// reais entre segmentos dentro do mesmo bloco).
pub fn format_subtitles(subtitles: &[Subtitle]) -> Vec<FormattedSubtitle> {
    let mut chunks: Vec<Chunk> = subtitles.iter().flat_map(format_block).collect();
    apply_duration_limits(&mut chunks);
    apply_timing_rules(&mut chunks);
    chunks
        .into_iter()
        .enumerate()
        .map(|(i, c)| FormattedSubtitle {
            index: i as u32 + 1,
            lines: c.lines,
            start_ms: c.start_ms,
            end_ms: c.end_ms,
            language: c.language,
        })
        .collect()
}

/// Unidade de trabalho interna (antes da re-indexação final).
#[derive(Debug, Clone)]
struct Chunk {
    lines: Vec<String>,
    start_ms: Timestamp,
    end_ms: Timestamp,
    language: Language,
}

impl Chunk {
    fn char_count(&self) -> usize {
        self.lines.iter().map(|l| l.chars().count()).sum()
    }

    fn duration_ms(&self) -> u64 {
        self.end_ms.as_ms().saturating_sub(self.start_ms.as_ms())
    }
}

/// Quebra uma `Subtitle` em um ou mais `Chunk`: primeiro aplica a regra de
/// linhas (max 2 linhas por chunk, re-partindo texto longo), depois distribui o
/// tempo do bloco proporcionalmente aos caracteres de cada chunk.
fn format_block(sub: &Subtitle) -> Vec<Chunk> {
    let text = sub
        .segments
        .iter()
        .map(|s| s.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if text.is_empty() {
        return Vec::new();
    }
    let start = sub
        .segments
        .iter()
        .map(|s| s.start_ms)
        .min()
        .unwrap_or(Timestamp::ZERO);
    let end = sub
        .segments
        .iter()
        .map(|s| s.end_ms)
        .max()
        .unwrap_or(Timestamp::ZERO);
    let span_ms = end.as_ms().saturating_sub(start.as_ms());
    if span_ms == 0 {
        return Vec::new();
    }

    let lines = break_lines(&text, MAX_CHARS_PER_LINE);
    let groups: Vec<Vec<String>> = lines.chunks(MAX_LINES).map(|c| c.to_vec()).collect();
    let weights: Vec<usize> = groups
        .iter()
        .map(|g| g.iter().map(|l| l.chars().count()).sum())
        .collect();
    let total: usize = weights.iter().sum();
    if total == 0 {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut cursor = start;
    for (group, weight) in groups.into_iter().zip(weights) {
        let dur = span_ms * weight as u64 / total as u64;
        let chunk_end = cursor.saturating_add_ms(dur);
        chunks.push(Chunk {
            lines: group,
            start_ms: cursor,
            end_ms: chunk_end,
            language: sub.language.clone(),
        });
        cursor = chunk_end;
    }
    if let Some(last) = chunks.last_mut() {
        last.end_ms = end;
    }
    chunks
}

/// Re-partição de chunks mais longos que `MAX_DURATION_MS`, recursivamente.
fn apply_duration_limits(chunks: &mut Vec<Chunk>) {
    let mut out = Vec::new();
    for chunk in chunks.drain(..) {
        out.extend(split_long(chunk, 0));
    }
    *chunks = out;
}

fn split_long(chunk: Chunk, depth: usize) -> Vec<Chunk> {
    if chunk.duration_ms() <= MAX_DURATION_MS || depth >= 8 {
        return vec![chunk];
    }
    let parts = if chunk.lines.len() == 2 {
        (vec![chunk.lines[0].clone()], vec![chunk.lines[1].clone()])
    } else {
        match split_text_in_half(&chunk.lines[0]) {
            Some((a, b)) => (vec![a], vec![b]),
            None => return vec![chunk],
        }
    };
    split_pair(chunk, parts.0, parts.1, depth)
}

/// Divide o span de `chunk` em dois chunks contíguos, tempo proporcional aos
/// caracteres de cada parte, e recursa se ainda houver pedaço > `MAX_DURATION_MS`.
fn split_pair(
    chunk: Chunk,
    a_lines: Vec<String>,
    b_lines: Vec<String>,
    depth: usize,
) -> Vec<Chunk> {
    let w_a: usize = a_lines.iter().map(|l| l.chars().count()).sum();
    let w_b: usize = b_lines.iter().map(|l| l.chars().count()).sum();
    let total = w_a + w_b;
    let mid = if total == 0 {
        chunk.end_ms
    } else {
        chunk
            .start_ms
            .saturating_add_ms(chunk.duration_ms() * w_a as u64 / total as u64)
    };
    let a = Chunk {
        lines: a_lines,
        start_ms: chunk.start_ms,
        end_ms: mid,
        language: chunk.language.clone(),
    };
    let b = Chunk {
        lines: b_lines,
        start_ms: mid,
        end_ms: chunk.end_ms,
        language: chunk.language,
    };
    let mut out = split_long(a, depth + 1);
    out.extend(split_long(b, depth + 1));
    out
}

/// Divide um texto em duas partes em fronteira de palavra próxima ao meio.
/// Retorna `None` se não houver fronteira (texto de palavra única).
fn split_text_in_half(text: &str) -> Option<(String, String)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let half = text.chars().count() / 2;
    let mut count = 0;
    let mut cut = None;
    for (i, w) in words.iter().enumerate() {
        count += w.chars().count() + 1;
        if count > half {
            cut = Some(i + 1);
            break;
        }
    }
    let cut = cut?;
    if cut == 0 || cut >= words.len() {
        return None;
    }
    Some((words[..cut].join(" "), words[cut..].join(" ")))
}

/// Ajustes finais de timing por chunk: duração mínima, teto de CPS e corte de
/// sobreposição com a próxima legenda. Os `start` são monotônicos (vêm da
/// construção por cursor), então apenas o `end` é ajustado.
fn apply_timing_rules(chunks: &mut [Chunk]) {
    for i in 0..chunks.len() {
        let next_start = chunks
            .get(i + 1)
            .map(|c| c.start_ms.as_ms())
            .unwrap_or(u64::MAX);
        let start = chunks[i].start_ms.as_ms();
        let mut end = chunks[i].end_ms.as_ms();

        let min_end = start.saturating_add(MIN_DURATION_MS);
        if min_end > end {
            end = min_end;
        }
        let cps_end = start.saturating_add(
            (chunks[i].char_count() as f64 / TARGET_CPS_MAX * 1000.0).ceil() as u64,
        );
        if cps_end > end {
            end = cps_end;
        }
        if end > next_start {
            end = next_start;
        }
        if end <= start {
            end = start.saturating_add(1);
        }
        chunks[i].end_ms = Timestamp::from_ms(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Segment;

    fn seg(text: &str, s: u64, e: u64) -> Segment {
        Segment::new(
            text,
            Timestamp::from_ms(s),
            Timestamp::from_ms(e),
            Language::Pt,
        )
        .unwrap()
    }

    fn sub(segments: Vec<Segment>) -> Subtitle {
        Subtitle {
            index: 1,
            segments,
            language: Language::Pt,
        }
    }

    #[test]
    fn cps_calculator() {
        assert_eq!(cps("abcde", 1000), 5.0);
        assert_eq!(cps("abcde", 500), 10.0);
        assert_eq!(cps("abcde", 0), 0.0);
        assert_eq!(cps("áéí", 1000), 3.0);
    }

    #[test]
    fn timing_original_preservado_quando_regras_satisfeitas() {
        let out = format_subtitles(&[sub(vec![seg("Primeira linha", 1000, 2500)])]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_ms.as_ms(), 1000);
        assert_eq!(out[0].end_ms.as_ms(), 2500);
        assert_eq!(out[0].lines, vec!["Primeira linha"]);
    }

    #[test]
    fn duracao_curta_estendida_para_1s() {
        let out = format_subtitles(&[sub(vec![seg("Sim.", 1000, 1400)])]);
        assert_eq!(out[0].start_ms.as_ms(), 1000);
        assert_eq!(out[0].end_ms.as_ms(), 2000);
        assert!(out[0].duration_ms() >= 1000);
    }

    #[test]
    fn extensao_respeita_inicio_da_proxima_legenda() {
        let out = format_subtitles(&[
            sub(vec![seg("Sim.", 1000, 1200)]),
            sub(vec![seg("Claro que sim.", 1300, 2500)]),
        ]);
        assert_eq!(
            out[0].end_ms.as_ms(),
            1300,
            "end cortado no início da próxima"
        );
        assert!(out[0].end_ms.as_ms() > out[0].start_ms.as_ms());
        assert!(out[0].end_ms <= out[1].start_ms, "sem sobreposição");
    }

    #[test]
    fn duracao_longa_de_duas_linhas_repartida() {
        let out = format_subtitles(&[sub(vec![seg(
            "Esta é a primeira linha deste bloco de exemplo, e aqui vai a segunda",
            1000,
            11000,
        )])]);
        assert!(out.len() >= 2, "esperava re-partição em 2+ legendas");
        for f in &out {
            assert!(f.lines.len() <= 2);
            assert!(
                f.duration_ms() <= MAX_DURATION_MS,
                "chunk {} com {}ms",
                f.index,
                f.duration_ms()
            );
        }
    }

    #[test]
    fn linha_unica_longa_no_tempo_repartida() {
        let out = format_subtitles(&[sub(vec![seg("Frase curta", 1000, 12000)])]);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].lines, vec!["Frase"]);
        assert_eq!(out[1].lines, vec!["curta"]);
        assert_eq!(out[0].start_ms.as_ms(), 1000);
        assert_eq!(out[1].end_ms.as_ms(), 12000);
        for f in &out {
            assert!(f.duration_ms() <= MAX_DURATION_MS);
        }
    }

    #[test]
    fn texto_longo_gera_varias_legendas_de_ate_2_linhas() {
        let long = "palavra ".repeat(30);
        let out = format_subtitles(&[sub(vec![seg(long.trim(), 0, 12000)])]);
        assert_eq!(out.len(), 3, "240 chars → 6 linhas → 3 blocos de 2");
        for f in &out {
            assert!(f.lines.len() <= 2);
        }
        let joined: Vec<String> = out.iter().flat_map(|f| f.lines.iter().cloned()).collect();
        assert_eq!(joined.join(" "), long.trim());
    }

    #[test]
    fn cps_no_teto_estende_end() {
        let out = format_subtitles(&[sub(vec![seg("ab cdefghijklmnopqrstuvwxyz", 1000, 2000)])]);
        assert_eq!(out[0].end_ms.as_ms(), 2080);
        assert!(out[0].cps() <= TARGET_CPS_MAX);
    }

    #[test]
    fn cps_no_range_para_segmentos_padrao() {
        let corpus = vec![
            sub(vec![seg("Olá, tudo bem com você hoje?", 0, 1800)]),
            sub(vec![seg(
                "Vamos caminhar pela praia ao entardecer",
                2000,
                4000,
            )]),
            sub(vec![seg("Sim, com certeza", 5000, 5700)]),
        ];
        let out = format_subtitles(&corpus);
        assert_eq!(out.len(), 3);
        for f in &out {
            assert!(
                (TARGET_CPS_MIN..=TARGET_CPS_MAX).contains(&f.cps()),
                "legenda {} com {:.1} cps fora da janela",
                f.index,
                f.cps()
            );
        }
    }

    #[test]
    fn sem_overlap_no_corpus() {
        let corpus = vec![
            sub(vec![seg("Olá, mundo!", 0, 1500)]),
            sub(vec![seg(
                "Um texto bem longo que precisa ser quebrado em várias linhas para caber na tela, com no máximo quarenta e dois caracteres por linha",
                2000,
                9000,
            )]),
            sub(vec![seg("Tudo bem?", 10000, 10400)]),
            sub(vec![seg(
                "Frase longa que fica na tela por bastante tempo sem quebrar",
                12000,
                21000,
            )]),
            sub(vec![
                seg("Primeira linha de um bloco", 22000, 25000),
                seg("segunda linha do mesmo bloco", 25000, 29000),
            ]),
        ];
        let out = format_subtitles(&corpus);
        assert!(!out.is_empty());
        for f in &out {
            assert!(
                f.lines.len() <= MAX_LINES,
                "legenda {} com {} linhas",
                f.index,
                f.lines.len()
            );
            for line in &f.lines {
                assert!(
                    line.chars().count() <= MAX_CHARS_PER_LINE,
                    "linha longa na legenda {}: {line:?}",
                    f.index
                );
            }
        }
        for w in out.windows(2) {
            assert!(
                w[0].end_ms <= w[1].start_ms,
                "overlap entre legendas {} e {}",
                w[0].index,
                w[1].index
            );
        }
    }

    #[test]
    fn indices_sequenciais_1_based() {
        let out = format_subtitles(&[
            sub(vec![seg("Primeira", 0, 2000)]),
            sub(vec![seg("Segunda", 3000, 5000)]),
        ]);
        let indexes: Vec<u32> = out.iter().map(|f| f.index).collect();
        assert_eq!(indexes, vec![1, 2]);
    }

    #[test]
    fn idioma_preservado() {
        let out = format_subtitles(&[sub(vec![seg("Olá", 0, 1500)])]);
        assert_eq!(out[0].language, Language::Pt);
    }

    #[test]
    fn entrada_vazia_ou_sem_segmentos() {
        assert_eq!(format_subtitles(&[]), Vec::new());
        assert_eq!(format_subtitles(&[sub(vec![])]), Vec::new());
    }
}
