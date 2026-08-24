#![cfg(feature = "stt")]
//! Teste E2E do pipeline de tradução (tarefa 3.10) com a engine mock (3.1).
//!
//! Valida a orquestração completa 3.5→3.4→3.6→1.8→1.7 sem exigir modelo real
//! (estratégia de testes: "Pipeline traduzido (3.10) com mock engine"). Roda em
//! CI: `cargo test --features stt --test e2e_translate`.
//!
//! Critérios de aceitação:
//! 1. mock engine gera SRT traduzido válido (regras 1.8);
//! 2. timing preservado (mesmos timestamps de entrada);
//! 3. formatação reaplicada ao texto traduzido.

use legendai_lib::config::AppConfig;
use legendai_lib::domain::{Language, Segment, Subtitle, Timestamp};
use legendai_lib::format::{
    cps,
    rules::{MAX_CHARS_PER_LINE, MAX_LINES, TARGET_CPS_MAX},
};
use legendai_lib::pipeline::run_translate_with_engine;
use legendai_lib::subtitles::srt::parse_srt;
use legendai_lib::translate::MockEngine;

fn cfg_pt_en() -> AppConfig {
    AppConfig {
        source_lang: "pt".into(),
        target_lang: "en".into(),
        ..Default::default()
    }
}

fn block(index: u32, text: &str, start: u64, end: u64) -> Subtitle {
    Subtitle {
        index,
        segments: vec![Segment::new(
            text,
            Timestamp::from_ms(start),
            Timestamp::from_ms(end),
            Language::Pt,
        )
        .unwrap()],
        language: Language::Pt,
    }
}

#[test]
fn e2e_mock_gera_srt_traduzido_valido_com_timing_preservado() {
    let input = vec![
        block(1, "Olá, mundo.", 1000, 3000),
        block(2, "Como você está?", 4000, 6500),
        block(3, "Estou muito bem, obrigado.", 7000, 10000),
    ];
    let mut engine = MockEngine::default();
    let result = run_translate_with_engine(&mut engine, &input, &cfg_pt_en()).unwrap();

    // 1. SRT final válido, com texto traduzido (prefixo do mock).
    let parsed = parse_srt(&result.srt).unwrap();
    assert_eq!(parsed.len(), input.len());
    for (out, src) in parsed.iter().zip(&input) {
        assert_eq!(out.segments[0].text, format!("TR {}", src.segments[0].text));
    }

    // 2. Timing preservado (mesmos timestamps de entrada).
    for (out, src) in parsed.iter().zip(&input) {
        assert_eq!(out.segments[0].start_ms, src.segments[0].start_ms);
        assert_eq!(out.segments[0].end_ms, src.segments[0].end_ms);
    }

    // 3. Regras 1.8: ≤2 linhas, ≤42 chars/linha, CPS ≤ 25, sem overlap.
    for sub in &parsed {
        assert!(
            sub.segments.len() <= MAX_LINES,
            "bloco {} com {} linhas (max {MAX_LINES})",
            sub.index,
            sub.segments.len()
        );
        for seg in &sub.segments {
            assert!(
                seg.text.chars().count() <= MAX_CHARS_PER_LINE,
                "linha com {} chars (max {MAX_CHARS_PER_LINE})",
                seg.text.chars().count()
            );
        }
        let text: String = sub
            .segments
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join(" ");
        let start = sub.segments[0].start_ms.as_ms();
        let end = sub.segments.last().unwrap().end_ms.as_ms();
        let speed = cps(&text, end - start);
        assert!(
            speed <= TARGET_CPS_MAX + 0.001,
            "legenda {} com {speed:.1} cps acima do teto de {TARGET_CPS_MAX}",
            sub.index
        );
    }
    for w in parsed.windows(2) {
        assert!(
            w[0].segments.last().unwrap().end_ms <= w[1].segments[0].start_ms,
            "overlap entre legendas {} e {}",
            w[0].index,
            w[1].index
        );
    }
}

#[test]
fn e2e_texto_longo_reaplica_formatacao() {
    let long =
        "uma frase bem longa que deveria ser quebrada em mais de uma linha para caber na tela de forma legível e confortável";
    let input = vec![block(1, long, 1000, 12000)];
    let mut engine = MockEngine::default();
    let result = run_translate_with_engine(&mut engine, &input, &cfg_pt_en()).unwrap();

    let parsed = parse_srt(&result.srt).unwrap();
    assert!(
        parsed.len() >= 2,
        "texto longo traduzido deve ser re-partido pelo formatter 1.8"
    );
    for sub in &parsed {
        assert!(sub.segments.len() <= MAX_LINES);
        for seg in &sub.segments {
            assert!(seg.text.chars().count() <= MAX_CHARS_PER_LINE);
        }
    }
}
