//! Serializer de texto puro (`.txt`).
//!
//! Gera uma legenda legível em texto simples, com timestamps opcionais
//! (`HH:MM:SS,mmm --> HH:MM:SS,mmm`) por bloco. Sem timestamps, apenas os
//! textos dos segmentos, um por linha.

use crate::domain::{Subtitle, Timestamp};

/// Serializa legendas para texto puro.
///
/// `with_timestamps = true` prefixa cada bloco com uma linha de tempo
/// `start --> end` (mesmo formato do SRT); `false` produz apenas os textos.
pub fn to_txt(subtitles: &[Subtitle], with_timestamps: bool) -> String {
    let mut out = String::new();
    for sub in subtitles {
        if with_timestamps {
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
            out.push_str(&format!("{start} --> {end}\n"));
        }
        for seg in &sub.segments {
            out.push_str(&seg.text);
            out.push('\n');
        }
        out.push('\n');
    }
    out
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
    fn sem_timestamps_so_texto() {
        assert_eq!(
            to_txt(&fixture(), false),
            "Olá, mundo!\n\nSegunda legenda.\n\n"
        );
    }

    #[test]
    fn com_timestamps_prefixa_cada_bloco() {
        let txt = to_txt(&fixture(), true);
        assert!(txt.contains("00:00:01,000 --> 00:00:03,000\nOlá, mundo!\n"));
        assert!(txt.contains("00:00:04,000 --> 00:00:06,500\nSegunda legenda.\n"));
    }

    #[test]
    fn bloco_multisegmento_junta_textos() {
        let sub = Subtitle {
            index: 1,
            segments: vec![
                Segment::new(
                    "A",
                    Timestamp::from_ms(0),
                    Timestamp::from_ms(1_000),
                    Language::auto(),
                )
                .unwrap(),
                Segment::new(
                    "B",
                    Timestamp::from_ms(1_000),
                    Timestamp::from_ms(2_000),
                    Language::auto(),
                )
                .unwrap(),
            ],
            language: Language::auto(),
        };
        assert_eq!(to_txt(&[sub], false), "A\nB\n\n");
    }

    #[test]
    fn entrada_vazia_retorna_vazio() {
        assert_eq!(to_txt(&[], false), "");
        assert_eq!(to_txt(&[], true), "");
    }
}
