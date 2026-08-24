use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

use crate::domain::Language;

use super::engine::{
    BatchRequest, BatchResult, TranslateError, TranslatedSegment, TranslationEngine,
    TranslationStatus,
};

/// Parâmetros fixos da arquitetura NLLB-200-distilled-600M (M2M100).
const N_LAYERS: usize = 12;
const N_HEADS: usize = 16;
const HEAD_DIM: usize = 64;
const HIDDEN_DIM: usize = 1024;

const BOS: i64 = 0; // `<s>`
const EOS: i64 = 2; // `</s>`
const PAD: i64 = 1; // `<pad>`
const MAX_NEW_TOKENS: usize = 60;

/// Mapeia código ISO 639-1 → token de idioma do NLLB-200 (formato `xxx_XXXX`).
/// Cobre os idiomas do catálogo (2.1) e os mais comuns do domínio.
fn lang_to_nllb(lang: &Language) -> Option<&'static str> {
    match lang.as_code() {
        "pt" => Some("por_Latn"),
        "en" => Some("eng_Latn"),
        "es" => Some("spa_Latn"),
        "fr" => Some("fra_Latn"),
        "de" => Some("deu_Latn"),
        "it" => Some("ita_Latn"),
        "ja" => Some("jpn_Jpan"),
        "zh" => Some("zho_Hans"),
        "ar" => Some("arb_Arab"),
        "ru" => Some("rus_Cyrl"),
        "nl" => Some("nld_Latn"),
        "pl" => Some("pol_Latn"),
        "tr" => Some("tur_Latn"),
        "hi" => Some("hin_Deva"),
        "ko" => Some("kor_Hang"),
        "vi" => Some("vie_Latn"),
        "th" => Some("tha_Thai"),
        "id" => Some("ind_Latn"),
        "sv" => Some("swe_Latn"),
        "da" => Some("dan_Latn"),
        "fi" => Some("fin_Latn"),
        "cs" => Some("ces_Latn"),
        "el" => Some("ell_Grek"),
        "hu" => Some("hun_Latn"),
        "ro" => Some("ron_Latn"),
        "uk" => Some("ukr_Cyrl"),
        "bg" => Some("bul_Cyrl"),
        "hr" => Some("hrv_Latn"),
        "sk" => Some("slk_Latn"),
        "sl" => Some("slv_Latn"),
        "he" => Some("heb_Hebr"),
        "ur" => Some("urd_Arab"),
        "fa" => Some("pes_Arab"),
        "ms" => Some("zsm_Latn"),
        "sw" => Some("swh_Latn"),
        _ => None,
    }
}

/// Engine de tradução NLLB-200 via ONNX Runtime (`ort`).
///
/// Carrega três arquivos (encoder, decoder merge e tokenizer) e traduz por
/// geração greedy com `decoder_start_token_id = </s>` + `forced_bos = <idioma>`.
///
/// # `ponytail:` geração recomputa o decoder do zero a cada token (`use_cache=false`)
/// em vez de usar o cache (`use_cache=true` / ramo `then` do `If`): o ramo com cache
/// falha no execution provider de CPU deste export (`encoder_attn/Reshape` com past
/// de tamanho > 2). Recomputar é O(n²) mas correto e suficiente para linhas curtas de
/// legenda (~0.5s/frase em fp32 nesta máquina). Trocar pelo cache quando o EP de CPU
/// suportar o ramo `then` ou se o throughput de legendas longas exigir.
#[derive(Debug)]
pub struct NllbEngine {
    encoder: Session,
    decoder: Session,
    tokenizer: Tokenizer,
}

impl NllbEngine {
    /// Carrega o modelo ONNX (encoder + decoder merged) e o tokenizer.
    /// `use_cuda` tenta CUDA EP quando o binário foi compilado com `--features cuda` e GPU detectada.
    pub fn load(
        encoder_path: impl AsRef<Path>,
        decoder_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
        threads: usize,
    ) -> Result<Self, TranslateError> {
        Self::load_with_gpu(encoder_path, decoder_path, tokenizer_path, threads, false)
    }

    /// Variante que permite forçar CUDA EP (factory passa `hw.gpu == Some(Cuda)`).
    pub fn load_with_gpu(
        encoder_path: impl AsRef<Path>,
        decoder_path: impl AsRef<Path>,
        tokenizer_path: impl AsRef<Path>,
        threads: usize,
        use_cuda: bool,
    ) -> Result<Self, TranslateError> {
        let threads = threads.max(1);
        let encoder = load_session(encoder_path.as_ref(), threads, use_cuda)?;
        let decoder = load_session(decoder_path.as_ref(), threads, use_cuda)?;
        let tokenizer = Tokenizer::from_file(tokenizer_path.as_ref())
            .map_err(|e| TranslateError::Backend(format!("tokenizer NLLB: {e}")))?;
        Ok(Self {
            encoder,
            decoder,
            tokenizer,
        })
    }

    /// Token id do marcador de idioma NLLB (`por_Latn`, `eng_Latn`, ...).
    fn lang_token_id(&self, lang: &Language) -> Result<i64, TranslateError> {
        let nllb = lang_to_nllb(lang).ok_or_else(|| {
            TranslateError::Backend(format!(
                "idioma `{}` não suportado pelo NLLB",
                lang.as_code()
            ))
        })?;
        self.tokenizer
            .token_to_id(nllb)
            .map(|id| id as i64)
            .ok_or_else(|| {
                TranslateError::Backend(format!("token `{nllb}` ausente do tokenizer NLLB"))
            })
    }

    /// Traduz um segmento de texto (pt→en, etc.) com geração greedy.
    fn translate_text(
        &mut self,
        text: &str,
        source: &Language,
        target: &Language,
    ) -> Result<String, TranslateError> {
        let src_token = self.lang_token_id(source)?;
        let tgt_token = self.lang_token_id(target)?;

        // O post-processor do tokenizer NLLB prefixa `eng_Latn` por padrão; removemos
        // esse prefixo e o `</s>` final e remontamos com o idioma de origem correto:
        // `[<src_lang> <tokens> </s>]` (mesmo formato do transformers.js/_build_translation_inputs).
        let mut toks: Vec<i64> = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| TranslateError::Backend(format!("tokenização: {e}")))?
            .get_ids()
            .iter()
            .map(|&t| t as i64)
            .collect();
        if toks.first()
            == Some(
                &self
                    .tokenizer
                    .token_to_id("eng_Latn")
                    .map(|i| i as i64)
                    .unwrap_or(-1),
            )
        {
            toks.remove(0);
        }
        if toks.last() == Some(&EOS) {
            toks.pop();
        }
        let mut src_ids = vec![src_token];
        src_ids.extend(toks);
        src_ids.push(EOS);

        // Encoder → hidden states.
        let enc_len = src_ids.len() as i64;
        let enc_input = Tensor::from_array((vec![1i64, enc_len], src_ids)).map_err(backend)?;
        let enc_mask = Tensor::from_array((vec![1i64, enc_len], vec![1i64; enc_len as usize]))
            .map_err(backend)?;
        let enc_out = self
            .encoder
            .run(ort::inputs![
                "input_ids" => &enc_input,
                "attention_mask" => &enc_mask
            ])
            .map_err(backend)?;
        let (enc_shape, enc_hidden) = enc_out["last_hidden_state"]
            .try_extract_tensor::<f32>()
            .map_err(backend)?;
        let enc_len = enc_shape[1];
        let enc_hidden: Vec<f32> = enc_hidden.to_vec();

        // Decoder greedy: input começa com `[</s>, <tgt_lang>]` (decoder_start + forced_bos)
        // e, por `ponytail:` (recompute), sempre roda com past vazio e `use_cache=false`.
        let mut dec_ids: Vec<i64> = vec![EOS, tgt_token];
        let mut generated: Vec<u32> = Vec::new();

        for _ in 0..MAX_NEW_TOKENS {
            let dlen = dec_ids.len() as i64;
            let dec_input =
                Tensor::from_array((vec![1i64, dlen], dec_ids.clone())).map_err(backend)?;
            let enc_hidden_t =
                Tensor::from_array((vec![1i64, enc_len, HIDDEN_DIM as i64], enc_hidden.clone()))
                    .map_err(backend)?;
            let enc_mask_t =
                Tensor::from_array((vec![1i64, enc_len], vec![1i64; enc_len as usize]))
                    .map_err(backend)?;
            let use_cache = Tensor::from_array((vec![1i64], vec![false])).map_err(backend)?;

            let mut feeds = ort::inputs![
                "input_ids" => &dec_input,
                "encoder_attention_mask" => &enc_mask_t,
                "encoder_hidden_states" => &enc_hidden_t,
                "use_cache_branch" => &use_cache,
            ];
            for layer in 0..N_LAYERS {
                for suffix in [
                    "decoder.key",
                    "decoder.value",
                    "encoder.key",
                    "encoder.value",
                ] {
                    let name = format!("past_key_values.{layer}.{suffix}");
                    let empty = Tensor::<f32>::from_array((
                        vec![1i64, N_HEADS as i64, 0, HEAD_DIM as i64],
                        Vec::<f32>::new(),
                    ))
                    .map_err(backend)?;
                    feeds.push((name.into(), empty.into()));
                }
            }

            let out = self.decoder.run(feeds).map_err(backend)?;
            let (logits_shape, logits) =
                out["logits"].try_extract_tensor::<f32>().map_err(backend)?;
            let dec_len = logits_shape[1] as usize;
            let vocab = logits_shape[2] as usize;
            let start = (dec_len - 1) * vocab;
            let last = &logits[start..start + vocab];
            let next = argmax(last) as i64;

            if next == EOS || next == PAD {
                break;
            }
            generated.push(next as u32);
            dec_ids.push(next);
        }

        let text = self
            .tokenizer
            .decode(&generated, true)
            .map_err(|e| TranslateError::Backend(format!("decodificação: {e}")))?;
        Ok(text.trim().to_string())
    }
}

fn load_session(path: &Path, threads: usize, use_cuda: bool) -> Result<Session, TranslateError> {
    if !path.exists() {
        return Err(TranslateError::Backend(format!(
            "modelo ONNX NLLB não encontrado em {}",
            path.display()
        )));
    }
    // Quando compilado com `cuda` e GPU disponível, tenta CUDA EP primeiro.
    #[cfg(feature = "cuda")]
    if use_cuda {
        let cuda_res: Result<Session, ort::Error> = (|| {
            let b = Session::builder()?;
            let b = b.with_execution_providers([ort::ep::CUDA::default().build()])?;
            let mut b = b.with_intra_threads(threads)?;
            Ok(b.commit_from_file(path)?)
        })();
        match cuda_res {
            Ok(s) => {
                tracing::info!("NLLB: CUDA EP habilitado para {}", path.display());
                return Ok(s);
            }
            Err(e) => tracing::warn!("NLLB: CUDA falhou ({e}), fallback CPU"),
        }
    }
    #[cfg(not(feature = "cuda"))]
    if use_cuda {
        tracing::warn!(
            "NLLB: GPU detectada mas binário sem feature `cuda` — usando CPU. Recompile com `--features cuda`"
        );
    }
    Session::builder()
        .map_err(backend)?
        .with_intra_threads(threads)
        .map_err(backend)?
        .commit_from_file(path)
        .map_err(backend)
}

fn argmax(slice: &[f32]) -> usize {
    slice
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

fn backend(e: impl std::fmt::Display) -> TranslateError {
    TranslateError::Backend(e.to_string())
}

impl TranslationEngine for NllbEngine {
    fn translate_batch(&mut self, req: &BatchRequest) -> Result<BatchResult, TranslateError> {
        let mut translations = Vec::with_capacity(req.segments.len());
        for seg in &req.segments {
            // NLLB é stateless por segmento: o contexto anterior (3.5) não é injetado,
            // pois o NLLB não aceita prompt/contexto (limitação documentada no ADR-001).
            let text = match self.translate_text(&seg.text, &req.source_lang, &req.target_lang) {
                Ok(t) => TranslatedSegment {
                    id: seg.id,
                    text: t,
                    status: TranslationStatus::Ok,
                },
                Err(e) => {
                    tracing::error!("segmento {} falhou no NLLB: {e}", seg.id);
                    TranslatedSegment {
                        id: seg.id,
                        text: seg.text.clone(),
                        status: TranslationStatus::KeptOriginal,
                    }
                }
            };
            translations.push(text);
        }
        Ok(BatchResult { translations })
    }

    fn supported_pair(&self, source: &Language, target: &Language) -> bool {
        lang_to_nllb(source).is_some() && lang_to_nllb(target).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::engine::BatchSegment;

    #[test]
    fn lang_to_nllb_mapeia_iso_comuns() {
        assert_eq!(lang_to_nllb(&Language::Pt), Some("por_Latn"));
        assert_eq!(lang_to_nllb(&Language::En), Some("eng_Latn"));
        assert_eq!(lang_to_nllb(&Language::Es), Some("spa_Latn"));
        assert_eq!(lang_to_nllb(&Language::Ja), Some("jpn_Jpan"));
        assert_eq!(lang_to_nllb(&Language::Zh), Some("zho_Hans"));
        assert_eq!(lang_to_nllb(&Language::Other("xx".into())), None);
        assert_eq!(lang_to_nllb(&Language::auto()), None);
    }

    #[test]
    fn suporta_pares_conhecidos_rejeita_desconhecidos() {
        assert!(lang_to_nllb(&Language::Pt).is_some());
        assert!(lang_to_nllb(&Language::En).is_some());
        assert!(lang_to_nllb(&Language::Other("pt".into())).is_some());
        assert!(lang_to_nllb(&Language::Other("xx".into())).is_none());
    }

    #[test]
    fn argmax_pega_indice_do_maior_valor() {
        assert_eq!(argmax(&[1.0, 3.0, 2.0]), 1);
        assert_eq!(argmax(&[0.0, -1.0, 5.0]), 2);
        assert_eq!(argmax(&[2.0]), 0);
    }

    #[test]
    fn load_model_ausente_retorna_erro_claro() {
        let err = NllbEngine::load(
            "/nao/existe/enc.onnx",
            "/nao/existe/dec.onnx",
            "/nao/existe/tok.json",
            4,
        )
        .unwrap_err();
        assert!(matches!(err, TranslateError::Backend(_)));
    }

    /// Teste manual (não roda em CI): traduz uma frase curta pt→en com modelo real.
    /// Exige modelos ONNX NLLB baixados em cache; aponta via env:
    ///   LEGENDAI_NLLB_ENC, LEGENDAI_NLLB_DEC, LEGENDAI_NLLB_TOK
    /// Rodar: cargo test --features ort -- --ignored nllb_manual_traduz_pt_en
    #[test]
    #[ignore]
    fn nllb_manual_traduz_pt_en() {
        let enc = std::env::var("LEGENDAI_NLLB_ENC").expect("set LEGENDAI_NLLB_ENC");
        let dec = std::env::var("LEGENDAI_NLLB_DEC").expect("set LEGENDAI_NLLB_DEC");
        let tok = std::env::var("LEGENDAI_NLLB_TOK").expect("set LEGENDAI_NLLB_TOK");
        let mut engine = NllbEngine::load(enc, dec, tok, 4).expect("carregar engine NLLB");
        assert!(engine.supported_pair(&Language::Pt, &Language::En));

        let req = BatchRequest {
            source_lang: Language::Pt,
            target_lang: Language::En,
            options: Default::default(),
            segments: vec![
                BatchSegment {
                    id: 1,
                    text: "Olá mundo, como vai você?".into(),
                    context: vec![],
                },
                BatchSegment {
                    id: 2,
                    text: "O gato subiu na árvore.".into(),
                    context: vec![],
                },
            ],
        };
        let res = engine.translate_batch(&req).expect("traduzir lote");
        for t in &res.translations {
            println!("id {}: {:?}", t.id, t.text);
        }
        assert_eq!(res.translations.len(), 2);
        assert!(res.translations[0].text.to_lowercase().contains("world"));
        assert!(res.translations[1].text.to_lowercase().contains("cat"));
        assert!(res
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::Ok));
    }
}
