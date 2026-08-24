use crate::domain::{Subtitle, Timestamp};
use crate::format::style::AssStyle;

/// Serializa legendas para o formato ASS com o estilo default profissional.
pub fn to_ass(subtitles: &[Subtitle]) -> String {
    to_ass_styled(subtitles, &AssStyle::default())
}

/// Serializa legendas para o formato ASS com um estilo customizado.
///
/// Estrutura: `[Script Info]`, `[V4+ Styles]` (uma linha `Style:` com os 23
/// campos) e `[Events]` com um `Dialogue:` por `Subtitle`. O tempo do bloco é o
/// menor `start` e o maior `end` dos segmentos (mesmo cálculo do SRT), em
/// `H:MM:SS.cs` (centésimos). As linhas de texto dos segmentos são unidas com
/// `\N` (o escape de quebra de linha do ASS).
pub fn to_ass_styled(subtitles: &[Subtitle], style: &AssStyle) -> String {
    let mut out = ass_header(style);
    for sub in subtitles {
        let (start, end) = block_time(sub);
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
            start.to_ass(),
            end.to_ass(),
            escape_text(sub)
        ));
    }
    out
}

/// Cor da linha original (secundária) na legenda dupla — cinza claro, sem o
/// `&` final (aplicado por [`colour_tag`]).
const DUAL_ORIGINAL_COLOUR: &str = "&H00C0C0C0";

/// Serializa legendas **duplas** (tradução + original) para ASS, com duas
/// linhas por `Dialogue`: a tradução na cor primária do estilo (principal, em
/// cima) e o original em cinza secundário (embaixo), unidas por `\N`.
///
/// O timing de cada evento é o da legenda **original** (skeleton de sync);
/// traduções que sobrepõem o bloco original são empilhadas acima da linha
/// original. Traduções sem correspondência temporal no original (timing novo)
/// entram como eventos avulsos no final, para nunca perder conteúdo.
pub fn to_ass_dual(original: &[Subtitle], translated: &[Subtitle], style: &AssStyle) -> String {
    let mut out = ass_header(style);
    let mut j = 0;
    for orig in original {
        let (start, end) = block_time(orig);
        let mut lines: Vec<String> = Vec::new();
        while j < translated.len() {
            let (ts, te) = block_time(&translated[j]);
            if te <= start {
                j += 1;
                continue;
            }
            if ts >= end {
                break;
            }
            lines.push(format!(
                "{}{}",
                colour_tag(&style.primary_colour),
                escape_text(&translated[j])
            ));
            j += 1;
        }
        lines.push(format!(
            "{}{}",
            colour_tag(DUAL_ORIGINAL_COLOUR),
            escape_text(orig)
        ));
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}\n",
            start.to_ass(),
            end.to_ass(),
            lines.join("\\N")
        ));
    }
    while j < translated.len() {
        let (start, end) = block_time(&translated[j]);
        out.push_str(&format!(
            "Dialogue: 0,{},{},Default,,0,0,0,,{}{}\n",
            start.to_ass(),
            end.to_ass(),
            colour_tag(&style.primary_colour),
            escape_text(&translated[j])
        ));
        j += 1;
    }
    out
}

/// Cabeçalho ASS: `[Script Info]` + `[V4+ Styles]` + abertura de `[Events]`.
fn ass_header(style: &AssStyle) -> String {
    let mut out = String::new();
    out.push_str("[Script Info]\n");
    out.push_str("ScriptType: v4.00+\n");
    out.push_str("WrapStyle: 0\n");
    out.push_str("ScaledBorderAndShadow: yes\n");
    out.push('\n');
    out.push_str("[V4+ Styles]\n");
    out.push_str(&format!("Format: {STYLE_FORMAT}\n"));
    out.push_str(&format!("Style: {}\n", style.to_style_line()));
    out.push('\n');
    out.push_str("[Events]\n");
    out.push_str(&format!("Format: {EVENT_FORMAT}\n"));
    out
}

/// Tag ASS de cor para override inline: `{\c<colour>&}`.
fn colour_tag(colour: &str) -> String {
    format!("{{\\c{colour}&}}")
}

/// Tempo do bloco de uma legenda: menor `start` e maior `end` dos segmentos.
fn block_time(sub: &Subtitle) -> (Timestamp, Timestamp) {
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
    (start, end)
}

/// Campos do `Format:` de `[V4+ Styles]` — 23, na ordem exata da spec.
const STYLE_FORMAT: &str = "Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, \
                            OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, \
                            ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, \
                            Alignment, MarginL, MarginR, MarginV, Encoding";

/// Campos do `Format:` de `[Events]` — 10. O `Text` é o último, então vírgulas
/// no texto são preservadas pelo parser (libass corta só os 9 primeiros).
const EVENT_FORMAT: &str =
    "Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text";

/// Junta os segmentos da legenda numa linha de diálogo. Quebras de linha
/// literais do texto viram `\N` (escape de linha do ASS); o restante do texto —
/// inclusive vírgulas — é preservado como está.
fn escape_text(sub: &Subtitle) -> String {
    sub.segments
        .iter()
        .map(|s| s.text.replace("\r\n", "\\N").replace(['\r', '\n'], "\\N"))
        .collect::<Vec<_>>()
        .join("\\N")
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

    /// Extrai os 10 campos de uma linha `Dialogue:` (split pelos 9 primeiros
    /// separadores — o Text é o último campo e pode conter vírgulas).
    fn dialogue_fields(line: &str) -> Vec<&str> {
        let rest = line.strip_prefix("Dialogue: ").unwrap();
        let mut fields = Vec::new();
        let mut start = 0;
        for _ in 0..9 {
            let comma = rest[start..].find(',').unwrap() + start;
            fields.push(&rest[start..comma]);
            start = comma + 1;
        }
        fields.push(&rest[start..]);
        fields
    }

    fn fixture() -> Vec<Subtitle> {
        vec![
            sub(1, "Olá, mundo!", 1_000, 3_000),
            sub(2, "Segunda legenda.", 4_000, 6_500),
            sub(3, "Terceira, com vírgula.", 7_000, 8_500),
        ]
    }

    #[test]
    fn estrutura_contem_as_tres_secoes() {
        let ass = to_ass(&fixture());
        assert!(ass.starts_with("[Script Info]\nScriptType: v4.00+\n"));
        assert!(ass.contains("\n[V4+ Styles]\n"));
        assert!(ass.contains("\n[Events]\n"));
        assert!(ass.contains("ScaledBorderAndShadow: yes\n"));
    }

    #[test]
    fn style_line_e_dialogue_respeitam_o_numero_de_campos() {
        let ass = to_ass(&fixture());
        let style = ass.lines().find(|l| l.starts_with("Style: ")).unwrap();
        assert_eq!(style.split(',').count(), 23, "Style: com 23 campos");
        for line in ass.lines().filter(|l| l.starts_with("Dialogue: ")) {
            assert_eq!(
                dialogue_fields(line).len(),
                10,
                "Dialogue: com 10 campos: {line}"
            );
        }
    }

    #[test]
    fn timestamps_usam_formato_ass_de_centesimos() {
        let ass = to_ass(&fixture());
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        assert_eq!(fields[1], "0:00:01.00", "start H:MM:SS.cs");
        assert_eq!(fields[2], "0:00:03.00", "end H:MM:SS.cs");
    }

    #[test]
    fn virgula_no_texto_e_preservada() {
        let ass = to_ass(&fixture());
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        assert_eq!(
            fields[9], "Olá, mundo!",
            "vírgula no campo Text é preservada"
        );
    }

    #[test]
    fn quebra_de_linha_literal_vira_backslash_n() {
        let ass = to_ass(&[sub(1, "Primeira\nSegunda", 1_000, 3_000)]);
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        assert_eq!(fields[9], "Primeira\\NSegunda");
    }

    #[test]
    fn bloco_multisegmento_junta_com_backslash_n() {
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
        let ass = to_ass(&[sub]);
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        assert_eq!(fields[1], "0:00:00.00", "menor start do bloco");
        assert_eq!(fields[2], "0:00:02.50", "maior end do bloco");
        assert_eq!(fields[9], "Linha um\\NLinha dois");
    }

    #[test]
    fn round_trip_basico_timestamps_e_texto() {
        let ass = to_ass(&fixture());
        let parsed: Vec<(String, String, String)> = ass
            .lines()
            .filter(|l| l.starts_with("Dialogue: "))
            .map(|l| {
                let f = dialogue_fields(l);
                (f[1].to_string(), f[2].to_string(), f[9].to_string())
            })
            .collect();
        let expected_starts = [1_000u64, 4_000, 7_000];
        let expected_ends = [3_000u64, 6_500, 8_500];
        let expected_texts = ["Olá, mundo!", "Segunda legenda.", "Terceira, com vírgula."];
        assert_eq!(parsed.len(), expected_texts.len());
        for (i, (start, end, text)) in parsed.iter().enumerate() {
            assert_eq!(start, &Timestamp::from_ms(expected_starts[i]).to_ass());
            assert_eq!(end, &Timestamp::from_ms(expected_ends[i]).to_ass());
            assert_eq!(text, expected_texts[i], "texto preservado no round-trip");
        }
    }

    #[test]
    fn estilo_customizado_na_linha_style() {
        let style = AssStyle {
            font_name: "Arial".into(),
            font_size: 56.0,
            primary_colour: "&H00FFFF00".into(),
            ..AssStyle::default()
        };
        let ass = to_ass_styled(&fixture(), &style);
        let style_line = ass.lines().find(|l| l.starts_with("Style: ")).unwrap();
        assert!(style_line.contains("Arial,56,&H00FFFF00"));
        assert!(ass.contains("ScriptType: v4.00+"));
    }

    #[test]
    fn entrada_vazia_gera_estrutura_sem_dialogue() {
        let ass = to_ass(&[]);
        assert!(ass.contains("[Events]\nFormat: Layer, Start, End"));
        assert!(!ass.contains("Dialogue: "));
    }

    fn translated() -> Vec<Subtitle> {
        vec![
            sub(1, "Hello, world!", 1_000, 3_000),
            sub(2, "Second subtitle.", 4_000, 6_500),
        ]
    }

    #[test]
    fn dupla_contem_duas_linguas_no_mesmo_dialogue() {
        let ass = to_ass_dual(&fixture(), &translated(), &AssStyle::default());
        let dialogs: Vec<&str> = ass
            .lines()
            .filter(|l| l.starts_with("Dialogue: "))
            .collect();
        assert_eq!(dialogs.len(), 3, "um Dialogue por legenda original");
        let first = dialogue_fields(dialogs[0]);
        assert!(
            first[9].contains("Hello, world!") && first[9].contains("Olá, mundo!"),
            "tradução e original no mesmo Dialogue: {}",
            first[9]
        );
        let third = dialogue_fields(dialogs[2]);
        assert!(
            third[9].contains("Terceira, com vírgula."),
            "original sem tradução correspondente cai com o texto original"
        );
    }

    #[test]
    fn dupla_aplica_cores_distintas_por_linha() {
        let ass = to_ass_dual(&fixture(), &translated(), &AssStyle::default());
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        assert!(
            fields[9].contains("{\\c&H00FFFFFF&}Hello, world!"),
            "tradução na cor primária (branca)"
        );
        assert!(
            fields[9].contains("{\\c&H00C0C0C0&}Olá, mundo!"),
            "original em cinza secundário"
        );
        assert_ne!(
            fields[9].find("{\\c&H00FFFFFF&}").unwrap(),
            fields[9].find("{\\c&H00C0C0C0&}").unwrap(),
            "cores distintas"
        );
    }

    #[test]
    fn dupla_ordena_traducao_em_cima_e_original_embaixo() {
        let ass = to_ass_dual(&fixture(), &translated(), &AssStyle::default());
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        let lines: Vec<&str> = fields[9].split("\\N").collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("Hello, world!"), "tradução primeiro");
        assert!(lines[1].contains("Olá, mundo!"), "original por último");
    }

    #[test]
    fn dupla_usa_timing_do_original() {
        let ass = to_ass_dual(&fixture(), &translated(), &AssStyle::default());
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        assert_eq!(fields[1], "0:00:01.00", "start do bloco original");
        assert_eq!(fields[2], "0:00:03.00", "end do bloco original");
    }

    #[test]
    fn dupla_empilha_varias_traducoes_que_sobrepoem_o_bloco_original() {
        let orig = vec![sub(1, "Original.", 0, 5_000)];
        let trans = vec![sub(1, "A.", 0, 2_500), sub(2, "B.", 2_000, 5_000)];
        let ass = to_ass_dual(&orig, &trans, &AssStyle::default());
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        let lines: Vec<&str> = fields[9].split("\\N").collect();
        assert_eq!(lines.len(), 3, "2 traduções empilhadas + original");
        assert!(lines[0].contains("A.") && lines[1].contains("B."));
        assert!(lines[2].contains("Original."));
    }

    #[test]
    fn dupla_traducao_sem_timing_correspondente_vira_evento_avulso() {
        let orig = vec![sub(1, "Original.", 0, 2_000)];
        let trans = vec![sub(1, "Sem match.", 5_000, 7_000)];
        let ass = to_ass_dual(&orig, &trans, &AssStyle::default());
        let dialogs: Vec<&str> = ass
            .lines()
            .filter(|l| l.starts_with("Dialogue: "))
            .collect();
        assert_eq!(dialogs.len(), 2, "bloco original + evento avulso");
        let orphan = dialogue_fields(dialogs[1]);
        assert_eq!(orphan[1], "0:00:05.00");
        assert!(orphan[9].contains("Sem match."));
        assert!(!orphan[9].contains("Original."), "avulso só com a tradução");
    }

    #[test]
    fn dupla_escapa_quebra_de_linha_literal() {
        let orig = vec![sub(1, "Primeira\nSegunda", 1_000, 3_000)];
        let trans = vec![sub(1, "First\nSecond", 1_000, 3_000)];
        let ass = to_ass_dual(&orig, &trans, &AssStyle::default());
        let fields = dialogue_fields(ass.lines().find(|l| l.starts_with("Dialogue: ")).unwrap());
        assert!(fields[9].contains("First\\NSecond"));
        assert!(fields[9].contains("Primeira\\NSegunda"));
    }
}
