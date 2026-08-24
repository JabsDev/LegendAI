//! Debug Hy-MT2: geração bruta com o template oficial.
//! Uso: cargo run --features full,cuda --example bench_hy

use std::time::Instant;

use legendai_lib::domain::Language;
use legendai_lib::translate::llm::LlmEngine;

fn main() {
    let path =
        "/home/jabs/.cache/legendai/models/translation/hy-mt2-1.8b-q8_0/Hy-MT2-1.8B-Q8_0.gguf";
    println!("carregando...");
    let start = Instant::now();
    let mut engine = match LlmEngine::load(path, 8, 256) {
        Ok(e) => e,
        Err(e) => {
            println!("LOAD FALHOU: {e}");
            return;
        }
    };
    println!("load: {}ms", start.elapsed().as_millis());

    // template oficial do HF: begin + User + content + Assistant
    let p1 = "<｜hy_begin▁of▁sentence｜><｜hy_User｜>Translate the following text into English. Note that you should only output the translated result without any additional explanation:\n\nOlá, como vai você?<｜hy_Assistant｜>";
    let t0 = Instant::now();
    match engine.raw_generate(p1, 128, 2048) {
        Ok(s) => {
            println!(
                "[P1 com begin] em {:.1}s -> {:?}",
                t0.elapsed().as_secs_f64(),
                s
            );
        }
        Err(e) => println!("[P1] erro: {e}"),
    }

    let p2 = "<｜hy_User｜>Translate the following text into English. Note that you should only output the translated result without any additional explanation:\n\nOlá, como vai você?<｜hy_Assistant｜>";
    let t1 = Instant::now();
    match engine.raw_generate(p2, 128, 2048) {
        Ok(s) => {
            println!(
                "[P2 sem begin] em {:.1}s -> {:?}",
                t1.elapsed().as_secs_f64(),
                s
            );
        }
        Err(e) => println!("[P2] erro: {e}"),
    }

    // formato numerado
    let p3 = "<｜hy_begin▁of▁sentence｜><｜hy_User｜>Translate the following texts into English. Note that you should only output the translated results, one per line with the same [N] prefix:\n\n[1] Olá, como vai você?\n[2] O gato subiu na árvore.<｜hy_Assistant｜>";
    let t2 = Instant::now();
    match engine.raw_generate(p3, 256, 4096) {
        Ok(s) => {
            println!(
                "[P3 numerado] em {:.1}s -> {:?}",
                t2.elapsed().as_secs_f64(),
                s
            );
        }
        Err(e) => println!("[P3] erro: {e}"),
    }

    // prompt EXATO do build_hy_batched_prompt com os textos sintéticos da integração
    let p4 = "<｜hy_begin▁of▁sentence｜><｜hy_User｜>Translate the following texts into English. Note that you should only output the translated results without any additional explanation, one per line with the same [N] prefix:\n\n[1] Este é o segmento de teste número 1 para legenda.\n[2] Este é o segmento de teste número 2 para legenda.<｜hy_Assistant｜>";
    let t4 = Instant::now();
    match engine.raw_generate(p4, 256, 4096) {
        Ok(s) => {
            println!(
                "[P4 textos sintéticos] em {:.1}s -> {:?}",
                t4.elapsed().as_secs_f64(),
                s
            );
        }
        Err(e) => println!("[P4] erro: {e}"),
    }

    // mesmo prompt, mas com max_tokens=1024 (igual translate_batched)
    let t5 = Instant::now();
    match engine.raw_generate(p4, 1024, 4096) {
        Ok(s) => {
            println!(
                "[P5 max1024] em {:.1}s -> {:?}",
                t5.elapsed().as_secs_f64(),
                s
            );
        }
        Err(e) => println!("[P5] erro: {e}"),
    }

    // fluxo completo de integração (translate_batch via run_translate)
    use legendai_lib::config::AppConfig;
    use legendai_lib::domain::{Segment, Subtitle, Timestamp};
    let segs: Vec<String> = (0..4)
        .map(|i| format!("Este é o segmento de teste número {} para legenda.", i + 1))
        .collect();
    let segments: Vec<Segment> = segs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            Segment::new(
                t.clone(),
                Timestamp::from_ms(i as u64 * 2000),
                Timestamp::from_ms(i as u64 * 2000 + 1500),
                Language::Pt,
            )
            .unwrap()
        })
        .collect();
    let subtitles: Vec<Subtitle> = segments
        .chunks(2)
        .enumerate()
        .map(|(i, chunk)| Subtitle {
            index: i as u32 + 1,
            segments: chunk.to_vec(),
            language: Language::Pt,
        })
        .collect();
    let cfg = AppConfig {
        target_lang: "en".into(),
        source_lang: "pt".into(),
        ..Default::default()
    };
    let t3 = Instant::now();
    match legendai_lib::pipeline::translate_pipeline::run_translate_with_engine(
        &mut engine,
        &subtitles,
        &cfg,
    ) {
        Ok(res) => {
            let secs = t3.elapsed().as_secs_f64();
            println!(
                "[INTEGRAÇÃO] 4 segs em {:.1}s -> {:.2} seg/s, kept {}",
                secs,
                4.0 / secs,
                res.kept_original_count
            );
            for s in &res.subtitles {
                println!("  -> {}", s.segments[0].text);
            }
        }
        Err(e) => println!("[INTEGRAÇÃO] erro: {e}"),
    }
}
