use std::num::NonZeroU32;
use std::path::Path;
use std::sync::OnceLock;

use llama_cpp_4::context::params::LlamaContextParams;
use llama_cpp_4::llama_backend::LlamaBackend;
use llama_cpp_4::llama_batch::LlamaBatch;
use llama_cpp_4::model::params::LlamaModelParams;
use llama_cpp_4::model::LlamaModel;
use llama_cpp_4::prelude::{AddBos, Special};
use llama_cpp_4::sampling::LlamaSampler;

use crate::domain::Language;

use super::engine::{
    BatchRequest, BatchResult, TranslateError, TranslatedSegment, TranslationEngine,
    TranslationStatus,
};

/// Número de layers a descarregar na GPU por tier (`ponytail:` fixo — ajustar só com
/// evidência de perda de qualidade; ver nota 3.3).
pub const NGL_TIER2: u32 = 256;
pub const NGL_TIER3: u32 = 512;

/// Tamanho de contexto do prompt (suficiente para prompt curto de legenda + geração).
const N_CTX: u32 = 2048;
const N_CTX_BATCH: u32 = 4096;
const MAX_NEW_TOKENS: usize = 256;
const MAX_NEW_TOKENS_BATCH: usize = 1024;

/// Inicializa o backend llama.cpp uma vez por processo (CUDA/Metal/etc.).
fn backend() -> &'static LlamaBackend {
    static BACKEND: OnceLock<LlamaBackend> = OnceLock::new();
    BACKEND.get_or_init(|| LlamaBackend::init().expect("falha ao inicializar o backend llama.cpp"))
}

/// Engine de tradução para LLMs GGUF (TowerInstruct 7B, Hy-MT2, Qwen) via `llama.cpp`.
///
/// Traduz um lote por chamada (template 3.7 com linhas `[N]` + parser 3.6), com
/// fallback por segmento. `llama-cpp-4` é o bindings do llama.cpp atual (suporta
/// arquiteturas novas como `hunyuan-dense` do Hy-MT2, que o crate antigo não lia).
///
/// # Anti-thinking
/// TowerInstruct/Hy-MT2 não têm modo thinking. O prompt pede "só a tradução" e
/// `strip_thinking` descarta qualquer bloco de reasoning que porventura apareça.
pub struct LlmEngine {
    model: LlamaModel,
    n_threads: u32,
    gpu_layers: u32,
    is_hy: bool,
}

impl LlmEngine {
    /// Carrega um GGUF de LLM. `threads` e `gpu_layers` (NGL) vêm do tier (2.5/2.6).
    pub fn load(
        model_path: impl AsRef<Path>,
        threads: usize,
        gpu_layers: u32,
    ) -> Result<Self, TranslateError> {
        let model_path = model_path.as_ref();
        if !model_path.exists() {
            return Err(TranslateError::Backend(format!(
                "modelo GGUF de LLM não encontrado em {} — baixe-o na aba Modelos",
                model_path.display()
            )));
        }
        let params = LlamaModelParams::default().with_n_gpu_layers(gpu_layers);
        let is_hy = model_path
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains("hy-mt2");
        let model = LlamaModel::load_from_file(backend(), model_path, &params)
            .map_err(|e| TranslateError::Backend(format!("falha ao carregar GGUF: {e}")))?;
        Ok(Self {
            model,
            n_threads: threads.max(1) as u32,
            gpu_layers,
            is_hy,
        })
    }

    /// Geração bruta (debug/bench) do `prompt`.
    #[doc(hidden)]
    pub fn raw_generate(
        &self,
        prompt: &str,
        max_tokens: usize,
        n_ctx: u32,
    ) -> Result<String, TranslateError> {
        self.generate(prompt, max_tokens, n_ctx)
    }

    /// Geração greedy do `prompt` com contexto `n_ctx` e teto de `max_tokens`.
    fn generate(
        &self,
        prompt: &str,
        max_tokens: usize,
        n_ctx: u32,
    ) -> Result<String, TranslateError> {
        let tokens = self
            .model
            .str_to_token(prompt, AddBos::Always)
            .map_err(|e| TranslateError::Backend(format!("tokenização LLM: {e}")))?;
        if tokens.is_empty() {
            return Ok(String::new());
        }
        let batch_size = n_ctx.min(1024) as usize;
        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(n_ctx))
            .with_n_batch(batch_size as u32)
            .with_n_threads(self.n_threads as i32)
            .with_n_threads_batch(self.n_threads as i32);
        let mut ctx = self
            .model
            .new_context(backend(), ctx_params)
            .map_err(|e| TranslateError::Backend(format!("falha ao criar contexto LLM: {e}")))?;

        let mut batch = LlamaBatch::new(batch_size, 1);
        for (i, &tok) in tokens.iter().enumerate() {
            batch
                .add(tok, i as i32, &[0], i == tokens.len() - 1)
                .map_err(|e| TranslateError::Backend(format!("falha ao prefill do prompt: {e}")))?;
        }
        ctx.decode(&mut batch)
            .map_err(|e| TranslateError::Backend(format!("falha ao decodificar: {e}")))?;

        // Amostragem determinística (greedy): evita divagações de reasoning na saída.
        let sampler = LlamaSampler::chain_simple([LlamaSampler::greedy()]);
        let mut out: Vec<u8> = Vec::new();
        for pos in (tokens.len() as i32..).take(max_tokens) {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            if self.model.is_eog_token(token) {
                break;
            }
            let bytes = self
                .model
                .token_to_bytes(token, Special::Plaintext)
                .map_err(|e| TranslateError::Backend(format!("falha ao decodificar token: {e}")))?;
            out.extend(bytes);
            batch.clear();
            batch
                .add(token, pos, &[0], true)
                .map_err(|e| TranslateError::Backend(format!("falha ao gerar: {e}")))?;
            ctx.decode(&mut batch)
                .map_err(|e| TranslateError::Backend(format!("falha ao decodificar: {e}")))?;
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Traduz um único segmento (com contexto anterior) para texto final.
    fn translate_one(
        &self,
        text: &str,
        source: &Language,
        target: &Language,
        context: &[String],
    ) -> Result<String, TranslateError> {
        let prompt = if self.is_hy {
            build_hy_prompt(text, target, context)
        } else {
            build_prompt(text, source, target, context)
        };
        let out = self.generate(&prompt, MAX_NEW_TOKENS, N_CTX)?;
        Ok(strip_thinking(&out).trim().to_string())
    }

    /// Lote em 1 chamada: monta o prompt 3.7 e gera todas as linhas `[N]` de uma vez.
    /// Retorna `None` se a geração falhar — o caller faz fallback por segmento.
    fn translate_batched(&self, req: &BatchRequest) -> Option<Result<BatchResult, TranslateError>> {
        let prompt = if self.is_hy {
            let batch = crate::translate::batcher::Batch {
                segments: req.segments.clone(),
            };
            build_hy_batched_prompt(&batch, &req.target_lang)
        } else {
            // Opções e glossário reais da config (best-effort: defaults se falhar)
            let config = crate::config::AppConfig::load_or_default();
            let options = config.translation_options.clone();
            let glossary = crate::translate::Glossary::load().to_prompt_entries();
            let batch = crate::translate::batcher::Batch {
                segments: req.segments.clone(),
            };
            let pair = crate::translate::prompt::LanguagePair {
                source: &req.source_lang,
                target: &req.target_lang,
            };
            crate::translate::prompt::build_prompt(&batch, &glossary, &pair, &options)
        };

        let raw = match self.generate(&prompt, MAX_NEW_TOKENS_BATCH, N_CTX_BATCH) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("LLM batched: geração falhou, fallback por segmento: {e}");
                return None;
            }
        };
        let cleaned = strip_thinking(&raw);
        // Parser 3.6 converte as linhas `[N] text` em BatchResult (ordem por id)
        let parsed = crate::translate::parser::parse_batch_response(&cleaned, &req.segments);
        // Se nada parseou (modelo fugiu do formato), deixa o retry externo tentar por segmento
        let ok_count = parsed
            .translations
            .iter()
            .filter(|t| t.status == TranslationStatus::Ok)
            .count();
        if ok_count == 0 && req.segments.len() > 1 {
            tracing::warn!(
                "LLM batched: 0/{} parseou OK (resposta: {:?}), fallback por segmento",
                req.segments.len(),
                cleaned.lines().take(5).collect::<Vec<_>>()
            );
            return None;
        }
        Some(Ok(parsed))
    }
}

fn build_hy_prompt(text: &str, target: &Language, context: &[String]) -> String {
    // Hy-MT2 foi treinado com nomes de idioma EM INGLÊS (README oficial) — usar
    // "English"/"Portuguese", não "inglês"/"português", senão o modelo não
    // reconhece o alvo e ecoa o input.
    let tgt = lang_name_en(target);
    let mut body = String::new();
    if !context.is_empty() {
        body.push_str("Contexto anterior (já traduzido):\n");
        for c in context {
            body.push_str(&format!("- {c}\n"));
        }
        body.push('\n');
    }
    body.push_str(&format!(
        "Translate the following text into {}. Note that you should only output the translated result without any additional explanation:\n\n{}",
        tgt, text
    ));
    // Template chat oficial do Hy-MT2 (HF chat_template.jinja): begin token obrigatório.
    format!(
        "<｜hy_begin▁of▁sentence｜><｜hy_User｜>{}<｜hy_Assistant｜>",
        body
    )
}

fn build_hy_batched_prompt(batch: &crate::translate::batcher::Batch, target: &Language) -> String {
    let tgt = lang_name_en(target);
    let mut numbered = String::new();
    for seg in &batch.segments {
        numbered.push_str(&format!("[{}] {}\n", seg.id, seg.text));
    }
    let body = format!(
        "Translate the following texts into {}. Note that you should only output the translated results without any additional explanation, one per line with the same [N] prefix:\n\n{}",
        tgt, numbered
    );
    format!(
        "<｜hy_begin▁of▁sentence｜><｜hy_User｜>{}<｜hy_Assistant｜>",
        body
    )
}

/// Monta o prompt no template chat do Tower (ChatML, compatível com Qwen). O template refinado com glossário
/// e múltiplas linhas numeradas é da tarefa 3.7 — aqui apenas o mínimo anti-thinking.
fn build_prompt(text: &str, source: &Language, target: &Language, context: &[String]) -> String {
    let src = lang_name(source);
    let tgt = lang_name(target);

    let mut user = String::new();
    if !context.is_empty() {
        user.push_str("Contexto anterior (já traduzido):\n");
        for c in context {
            user.push_str(&format!("- {c}\n"));
        }
        user.push('\n');
    }
    user.push_str(text);

    format!(
        "<|im_start|>system\nVocê é um tradutor profissional de legendas. Traduza o texto abaixo \
         do {src} para o {tgt}. Responda SOMENTE com a tradução final, sem explicação, raciocínio \
         ou comentários.<|im_end|>\n\
         <|im_start|>user\n{user}<|im_end|>\n\
         <|im_start|>assistant\n"
    )
}

/// LLM traduz praticamente qualquer par; auto (detecção do Whisper) não serve como
/// idioma origem/destino de tradução.
fn supports_pair(source: &Language, target: &Language) -> bool {
    !source.is_auto() && !target.is_auto()
}

/// Nome legível (pt-BR) do idioma para o prompt. Desconhecido → código ISO.
fn lang_name(lang: &Language) -> String {
    match lang.as_code() {
        "pt" => "português".into(),
        "en" => "inglês".into(),
        "es" => "espanhol".into(),
        "fr" => "francês".into(),
        "de" => "alemão".into(),
        "it" => "italiano".into(),
        "ja" => "japonês".into(),
        "zh" => "chinês".into(),
        "ar" => "árabe".into(),
        "ru" => "russo".into(),
        other => other.to_string(),
    }
}

/// Nome do idioma EM INGLÊS para o Hy-MT2 (README: nomes completos em inglês).
fn lang_name_en(lang: &Language) -> String {
    match lang.as_code() {
        "pt" => "Portuguese".into(),
        "en" => "English".into(),
        "es" => "Spanish".into(),
        "fr" => "French".into(),
        "de" => "German".into(),
        "it" => "Italian".into(),
        "ja" => "Japanese".into(),
        "zh" => "Chinese".into(),
        "ar" => "Arabic".into(),
        "ru" => "Russian".into(),
        "ko" => "Korean".into(),
        "nl" => "Dutch".into(),
        "pl" => "Polish".into(),
        "tr" => "Turkish".into(),
        "vi" => "Vietnamese".into(),
        "th" => "Thai".into(),
        other => other.to_string(),
    }
}

/// Descarta blocos de reasoning/thinking comuns da saída, caso o modelo emita algum.
/// (`ponytail:` cobre os marcadores mais frequentes; a fonte canônica é a instrução do
/// prompt — Qwen2.5 sem modo thinking — não uma garantia de parser.)
fn strip_thinking(text: &str) -> String {
    for (open, close) in [("<think>", "</think>"), ("[thinking]", "[/thinking]")] {
        if let (Some(start), Some(end)) = (text.find(open), text.find(close)) {
            if end > start {
                let mut out = String::with_capacity(text.len());
                out.push_str(&text[..start]);
                out.push_str(&text[end + close.len()..]);
                return out;
            }
        }
    }
    // `think` de abertura sem fechamento: descarta do marcador até a 1ª linha em branco
    // (parágrafo de raciocínio), mantendo o que vier depois.
    if let Some(start) = text.find("think\n") {
        let rest = &text[start + "think\n".len()..];
        let after = rest.find("\n\n").map(|i| i + 2).unwrap_or(0);
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start]);
        out.push_str(&rest[after..]);
        return out;
    }
    text.to_string()
}

impl TranslationEngine for LlmEngine {
    fn translate_batch(&mut self, req: &BatchRequest) -> Result<BatchResult, TranslateError> {
        if req.segments.is_empty() {
            return Ok(BatchResult {
                translations: vec![],
            });
        }
        // ponytail: 10x speedup — 1 chamada por lote (10 segs) em vez de 1 por segmento.
        // Usa o template 3.7 com contexto/glossário/opções reais; single-seg cai aqui tb.
        if req.segments.len() > 1 {
            if let Some(batched) = self.translate_batched(req) {
                return batched;
            }
            // fallback: por segmento se o lote falhar
        }
        let mut translations = Vec::with_capacity(req.segments.len());
        for seg in &req.segments {
            let t = match self.translate_one(
                &seg.text,
                &req.source_lang,
                &req.target_lang,
                &seg.context,
            ) {
                Ok(text) => TranslatedSegment {
                    id: seg.id,
                    text,
                    status: TranslationStatus::Ok,
                },
                Err(e) => {
                    tracing::error!("segmento {} falhou no LLM: {e}", seg.id);
                    TranslatedSegment {
                        id: seg.id,
                        text: seg.text.clone(),
                        status: TranslationStatus::KeptOriginal,
                    }
                }
            };
            translations.push(t);
        }
        Ok(BatchResult { translations })
    }

    fn supported_pair(&self, source: &Language, target: &Language) -> bool {
        supports_pair(source, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::engine::BatchSegment;

    #[test]
    fn strip_thinking_remove_blocos_xml() {
        assert_eq!(strip_thinking("a<think>vou refletir</think>b"), "ab");
        assert_eq!(
            strip_thinking("Olá [thinking]hmm[/thinking] mundo"),
            "Olá  mundo"
        );
    }

    #[test]
    fn strip_thinking_remove_paragrafo_think() {
        assert_eq!(
            strip_thinking("think\nraciocínio aqui\n\nTradução"),
            "Tradução"
        );
        assert_eq!(strip_thinking("só texto"), "só texto");
    }

    #[test]
    fn prompt_contem_anti_thinking_e_idiomas() {
        let p = build_prompt("Olá", &Language::Pt, &Language::En, &[]);
        assert!(p.contains("sem explicação, raciocínio"));
        assert!(p.contains("português"));
        assert!(p.contains("inglês"));
        assert!(p.contains("<|im_start|>assistant\n"));
    }

    #[test]
    fn prompt_inclui_contexto_anterior() {
        let p = build_prompt("Fala 2", &Language::Pt, &Language::En, &["Fala 1".into()]);
        assert!(p.contains("Contexto anterior"));
        assert!(p.contains("Fala 1"));
    }

    #[test]
    fn load_model_ausente_retorna_erro_claro() {
        match LlmEngine::load("/nao/existe/modelo.gguf", 4, 0) {
            Ok(_) => panic!("esperava erro para modelo ausente"),
            Err(e) => {
                assert!(matches!(e, TranslateError::Backend(_)));
                assert!(e.to_string().contains("GGUF"));
            }
        }
    }

    #[test]
    fn suporta_pares_sem_auto_rejeita_auto() {
        assert!(supports_pair(&Language::Pt, &Language::En));
        assert!(!supports_pair(&Language::auto(), &Language::En));
        assert!(!supports_pair(&Language::Pt, &Language::auto()));
    }

    /// Teste manual (não roda em CI): traduz um lote com GGUF real. Exige modelo em cache;
    /// aponta via env `LEGENDAI_LLM_PATH` (ex: qwen2.5-3b-instruct-q4_k_m.gguf).
    /// Rodar: cargo test --features llama -- --ignored llm_manual_traduz_lote
    #[test]
    #[ignore]
    fn llm_manual_traduz_lote() {
        let path = std::env::var("LEGENDAI_LLM_PATH").expect("set LEGENDAI_LLM_PATH");
        let mut engine = LlmEngine::load(&path, 4, 0).expect("carregar engine LLM");
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
                    context: vec!["Olá mundo, como vai você?".into()],
                },
            ],
        };
        let res = engine.translate_batch(&req).expect("traduzir lote");
        assert_eq!(res.translations.len(), 2);
        for t in &res.translations {
            println!("id {}: {:?}", t.id, t.text);
        }
        assert!(res.translations[0].text.to_lowercase().contains("world"));
        assert!(res.translations[1].text.to_lowercase().contains("cat"));
        assert!(res
            .translations
            .iter()
            .all(|t| t.status == TranslationStatus::Ok));
    }
}
