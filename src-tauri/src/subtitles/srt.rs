use thiserror::Error;

use crate::domain::{Language, Segment, Subtitle, Timestamp};

/// Serializa legendas para o formato SRT.
///
/// Cada `Subtitle` vira um bloco: índice, linha de tempo `start --> end` e uma
/// linha de texto por `Segment`. O tempo do bloco é o menor `start` e o maior
/// `end` dos segmentos (segmentos dentro de um bloco compartilham o mesmo tempo
/// no SRT). Limitação: timestamps por segmento dentro de um bloco são perdidos
/// no round-trip — é a limitação natural do formato.
pub fn to_srt(subtitles: &[Subtitle]) -> String {
    let mut out = String::new();
    for sub in subtitles {
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
        out.push_str(&format!("{}\n{start} --> {end}\n", sub.index));
        for seg in &sub.segments {
            out.push_str(&seg.text);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Faz o parse de texto SRT em `Vec<Subtitle>`.
///
/// Tolerante a BOM, CRLF/LF e índices ausentes (re-numerados sequencialmente).
/// Blocos sem linha de tempo ou com timestamp malformado geram `SrtError` com o
/// número da linha (1-based) do arquivo original.
pub fn parse_srt(input: &str) -> Result<Vec<Subtitle>, SrtError> {
    let input = input.strip_prefix('\u{FEFF}').unwrap_or(input);
    let input = input.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = input.lines().collect();

    let mut subtitles = Vec::new();
    let mut next_index: u32 = 1;
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let mut block = Vec::new();
        while i < lines.len() && !lines[i].trim().is_empty() {
            block.push((i + 1, lines[i]));
            i += 1;
        }
        let timing_pos = block
            .iter()
            .position(|(_, l)| l.contains("-->"))
            .ok_or(SrtError::MissingTimingLine { line: block[0].0 })?;
        let (timing_line_no, timing_line) = block[timing_pos];
        let mut parts = timing_line.split("-->");
        let start = parts.next().unwrap().trim();
        let end = parts.next().map(str::trim).unwrap_or("");
        if end.is_empty() {
            return Err(SrtError::InvalidTimestamp {
                line: timing_line_no,
                value: timing_line.to_string(),
            });
        }
        let start_ts = Timestamp::from_srt(start).map_err(|_| SrtError::InvalidTimestamp {
            line: timing_line_no,
            value: start.to_string(),
        })?;
        let end_ts = Timestamp::from_srt(end).map_err(|_| SrtError::InvalidTimestamp {
            line: timing_line_no,
            value: end.to_string(),
        })?;
        if end_ts <= start_ts {
            return Err(SrtError::InvalidTiming {
                line: timing_line_no,
                start: start_ts.as_ms(),
                end: end_ts.as_ms(),
            });
        }
        let index = block
            .first()
            .and_then(|(_, l)| l.trim().parse::<u32>().ok())
            .unwrap_or(next_index);
        next_index = index + 1;
        let segments: Vec<Segment> = block[timing_pos + 1..]
            .iter()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(_, l)| Segment {
                text: l.to_string(),
                start_ms: start_ts,
                end_ms: end_ts,
                lang: Language::auto(),
            })
            .collect();
        subtitles.push(Subtitle {
            index,
            segments,
            language: Language::auto(),
        });
    }
    Ok(subtitles)
}

/// Erros do parser SRT. `line` é sempre 1-based no arquivo original.
#[derive(Debug, Error)]
pub enum SrtError {
    #[error("linha {line}: timestamp inválido: `{value}`")]
    InvalidTimestamp { line: usize, value: String },
    #[error("linha {line}: bloco sem linha de tempo `start --> end`")]
    MissingTimingLine { line: usize },
    #[error("linha {line}: end ({end}ms) deve ser maior que start ({start}ms)")]
    InvalidTiming { line: usize, start: u64, end: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Timestamp;

    fn subtitle(index: u32, text: &str, start_ms: u64, end_ms: u64) -> Subtitle {
        Subtitle {
            index,
            segments: vec![Segment {
                text: text.to_string(),
                start_ms: Timestamp::from_ms(start_ms),
                end_ms: Timestamp::from_ms(end_ms),
                lang: Language::auto(),
            }],
            language: Language::auto(),
        }
    }

    fn fixture() -> Vec<Subtitle> {
        vec![
            subtitle(1, "Olá, mundo!", 1000, 3000),
            subtitle(2, "Segunda legenda.", 4000, 6500),
            subtitle(3, "Terceira, com acento.", 7000, 8500),
        ]
    }

    #[test]
    fn round_trip_parse_to_srt() {
        let subs = fixture();
        let srt = to_srt(&subs);
        assert_eq!(
            parse_srt(&srt).unwrap(),
            subs,
            "round-trip deve preservar índice, texto e tempo"
        );
    }

    #[test]
    fn writer_formato_blocos() {
        let srt = to_srt(&fixture());
        assert_eq!(
            srt,
            "1\n00:00:01,000 --> 00:00:03,000\nOlá, mundo!\n\
             \n2\n00:00:04,000 --> 00:00:06,500\nSegunda legenda.\n\
             \n3\n00:00:07,000 --> 00:00:08,500\nTerceira, com acento.\n\n"
        );
    }

    #[test]
    fn writer_bloco_multilinha_usa_min_max() {
        let sub = Subtitle {
            index: 1,
            segments: vec![
                Segment {
                    text: "Linha um".into(),
                    start_ms: Timestamp::from_ms(0),
                    end_ms: Timestamp::from_ms(2000),
                    lang: Language::auto(),
                },
                Segment {
                    text: "Linha dois".into(),
                    start_ms: Timestamp::from_ms(1500),
                    end_ms: Timestamp::from_ms(2500),
                    lang: Language::auto(),
                },
            ],
            language: Language::auto(),
        };
        assert_eq!(
            to_srt(&[sub]),
            "1\n00:00:00,000 --> 00:00:02,500\nLinha um\nLinha dois\n\n"
        );
    }

    #[test]
    fn parser_aceita_crlf() {
        let srt = to_srt(&fixture()).replace('\n', "\r\n");
        assert_eq!(parse_srt(&srt).unwrap(), fixture());
    }

    #[test]
    fn parser_aceita_bom() {
        let srt = format!("\u{FEFF}{}", to_srt(&fixture()));
        assert_eq!(parse_srt(&srt).unwrap(), fixture());
    }

    #[test]
    fn parser_aceita_bom_e_crlf_juntos() {
        let srt = format!("\u{FEFF}{}", to_srt(&fixture()).replace('\n', "\r\n"));
        assert_eq!(parse_srt(&srt).unwrap(), fixture());
    }

    #[test]
    fn parser_tolera_indices_ausentes() {
        let srt =
            "00:00:01,000 --> 00:00:03,000\nPrimeira\n\n00:00:04,000 --> 00:00:06,000\nSegunda\n";
        let subs = parse_srt(srt).unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].index, 1);
        assert_eq!(subs[0].segments[0].text, "Primeira");
        assert_eq!(subs[1].index, 2);
        assert_eq!(subs[1].segments[0].text, "Segunda");
    }

    #[test]
    fn parser_ignora_titulo_antes_do_indice() {
        let srt =
            "Meu Filme\n1\n00:00:01,000 --> 00:00:03,000\nTexto\n\n2\n00:00:04,000 --> 00:00:06,000\nOutro\n";
        let subs = parse_srt(srt).unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0].index, 1);
        assert_eq!(subs[0].segments[0].text, "Texto");
        assert_eq!(subs[1].segments[0].text, "Outro");
    }

    #[test]
    fn parser_erro_aponta_linha_de_timestamp_malformado() {
        let srt =
            "1\n00:00:01,000 --> 00:00:03,000\nBom\n\n2\n00:00:xx,000 --> 00:00:06,000\nRuim\n";
        let err = parse_srt(srt).unwrap_err();
        match err {
            SrtError::InvalidTimestamp { line, value } => {
                assert_eq!(line, 6, "linha 6 é a do timestamp inválido");
                assert_eq!(value, "00:00:xx,000");
            }
            other => panic!("esperava InvalidTimestamp, veio {other:?}"),
        }
    }

    #[test]
    fn parser_erro_em_timestamp_sem_fim() {
        let srt = "1\n00:00:01,000 -->\nTexto\n";
        assert!(matches!(
            parse_srt(srt).unwrap_err(),
            SrtError::InvalidTimestamp { .. }
        ));
    }

    #[test]
    fn parser_erro_bloco_sem_linha_de_tempo() {
        let srt = "1\nsó texto, sem timing\n";
        assert!(matches!(
            parse_srt(srt).unwrap_err(),
            SrtError::MissingTimingLine { line: 1 }
        ));
    }

    #[test]
    fn parser_erro_quando_end_menor_que_start() {
        let srt = "1\n00:00:05,000 --> 00:00:01,000\nTexto\n";
        assert!(matches!(
            parse_srt(srt).unwrap_err(),
            SrtError::InvalidTiming { .. }
        ));
    }

    #[test]
    fn parser_vazio_retorna_lista_vazia() {
        assert_eq!(parse_srt("").unwrap(), Vec::new());
        assert_eq!(parse_srt("\n\n\r\n").unwrap(), Vec::new());
    }
}
