//! Serializer WebVTT (`.vtt`).
//!
//! Estrutura idêntica ao SRT, com duas diferenças: cabeçalho `WEBVTT` e
//! timestamps com `.` como separador de milissegundos (`HH:MM:SS.mmm`). O
//! tempo de cada bloco é o menor `start` e o maior `end` dos segmentos (mesmo
//! cálculo do SRT/ASS). Texto de múltiplos segmentos vai em linhas separadas.

use crate::domain::{Subtitle, Timestamp};

/// Serializa legendas para o formato WebVTT.
pub fn to_vtt(subtitles: &[Subtitle]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for sub in subtitles {
        let start = block_start(sub);
        let end = block_end(sub);
        out.push_str(&format!(
            "{}\n{} --> {}\n",
            sub.index,
            vtt_time(start),
            vtt_time(end)
        ));
        for seg in &sub.segments {
            out.push_str(&seg.text);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

fn block_start(sub: &Subtitle) -> Timestamp {
    sub.segments
        .iter()
        .map(|s| s.start_ms)
        .min()
        .unwrap_or(Timestamp::ZERO)
}

fn block_end(sub: &Subtitle) -> Timestamp {
    sub.segments
        .iter()
        .map(|s| s.end_ms)
        .max()
        .unwrap_or(Timestamp::ZERO)
}

/// Timestamp WebVTT: `HH:MM:SS.mmm` (ponto, não vírgula como no SRT).
fn vtt_time(ts: Timestamp) -> String {
    let ms = ts.as_ms();
    let h = ms / 3_600_000;
    let m = (ms / 60_000) % 60;
    let s = (ms / 1000) % 60;
    let frac = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}.{frac:03}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Language, Segment};

    fn sub(index: u32, text: &str, start_ms: u64, end_ms: u64) -> Subtitle {
        Subtitle {
            index,
            segments: vec![Segment::new(
                text,
                Timestamp::from_ms(start_ms),
                Timestamp::from_ms(end_ms),
                Language::auto(),
            )
            .unwrap()],
            language: Language::auto(),
        }
    }

    fn fixture() -> Vec<Subtitle> {
        vec![
            sub(1, "Olá, mundo!", 1_000, 3_000),
            sub(2, "Segunda legenda.", 4_000, 6_500),
        ]
    }

    #[test]
    fn cabecalho_webvtt_e_blocos() {
        let vtt = to_vtt(&fixture());
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("\n1\n00:00:01.000 --> 00:00:03.000\nOlá, mundo!\n"));
        assert!(vtt.contains("\n2\n00:00:04.000 --> 00:00:06.500\nSegunda legenda.\n"));
    }

    #[test]
    fn timestamps_usam_ponto_nao_virgula() {
        let vtt = to_vtt(&fixture());
        for line in vtt.lines().filter(|l| l.contains("-->")) {
            assert!(!line.contains(','), "timestamp VTT usa `.`: {line}");
        }
        assert!(vtt.contains("00:00:01.000 --> 00:00:03.000"));
    }

    #[test]
    fn bloco_multisegmento_usa_min_max_e_multiplas_linhas() {
        let sub = Subtitle {
            index: 1,
            segments: vec![
                Segment::new(
                    "Linha um",
                    Timestamp::from_ms(0),
                    Timestamp::from_ms(2_000),
                    Language::auto(),
                )
                .unwrap(),
                Segment::new(
                    "Linha dois",
                    Timestamp::from_ms(1_500),
                    Timestamp::from_ms(2_500),
                    Language::auto(),
                )
                .unwrap(),
            ],
            language: Language::auto(),
        };
        let vtt = to_vtt(&[sub]);
        assert!(vtt.contains("00:00:00.000 --> 00:00:02.500"));
        assert!(vtt.contains("Linha um\nLinha dois"));
    }

    #[test]
    fn entrada_vazia_tem_so_o_cabecalho() {
        assert_eq!(to_vtt(&[]), "WEBVTT\n\n");
    }
}
