//! Template de prompt do LLM de tradução (tarefa 3.7).
//!
//! Monta o prompt no template chat do Qwen (ChatML) com: papel, regras de
//! legenda, bloco de contexto anterior, glossário opcional e as linhas
//! numeradas `[N]` do lote. O parser (3.6) espera exatamente o formato
//! `[N] texto` na resposta — o prompt é o contrato que o mantém.
//!
//! O template é experimentação (ajustar prompt é afinar qualidade): o teste de
//! snapshot impede regressão acidental — mudar qualquer texto exige atualizar o
//! snapshot intencionalmente. A instrução anti-thinking é obrigatória e
//! verificada por teste (Qwen3 exigiria também `--no-thinking` no backend, 3.3).

use super::batcher::Batch;
use super::options::{Formality, TranslationOptions};
use crate::domain::Language;

/// Par origem/destino da tradução (`pair` na assinatura do plano).
#[derive(Debug, Clone, Copy)]
pub struct LanguagePair<'a> {
    pub source: &'a Language,
    pub target: &'a Language,
}

/// Entrada do glossário opcional: `term` no idioma de origem → `translation` no
/// idioma de destino. Ausente (slice vazio) → bloco de glossário omitido.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryEntry {
    pub term: String,
    pub translation: String,
}

/// Monta o prompt completo de um lote: sistema (papel+regras+anti-thinking) +
/// usuário (contexto, glossário e linhas `[N]`).
///
/// O bloco de contexto usa o contexto do **primeiro** segmento do lote (os
/// segmentos imediatamente anteriores ao lote — os demais herdam do próprio
/// lote pelas linhas numeradas), limitado ao `context_size` de `options`
/// (tarefa 5.4; `0` omite o bloco). Lote sem contexto e sem glossário → prompt
/// só com as regras e as linhas numeradas. As opções de `options` também
/// ajustam o tom (formalidade) e acrescentam instruções livres do usuário.
pub fn build_prompt(
    batch: &Batch,
    glossary: &[GlossaryEntry],
    pair: &LanguagePair,
    options: &TranslationOptions,
) -> String {
    let src = lang_name(pair.source);
    let tgt = lang_name(pair.target);

    // Regras do sistema: a formalidade (5.4) troca a regra de tom; instruções
    // livres do usuário entram como uma regra extra (omitida quando vazia).
    let formality = match options.formality {
        Formality::Formal => {
            "Traduza de forma formal e cuidada, adequada ao ritmo de leitura de legendas."
        }
        Formality::Colloquial => {
            "Traduza de forma coloquial e natural, adequada ao ritmo de leitura de legendas."
        }
    };
    let mut rules: Vec<String> = vec![
        formality.to_string(),
        "Mantenha nomes próprios, marcas e lugares sem tradução.".into(),
        "Quando possível, mantenha cada linha com no máximo 42 caracteres.".into(),
        "Mantenha o formato [N] antes de cada tradução, uma linha por segmento.".into(),
        "Responda SOMENTE com as linhas numeradas, sem explicação ou raciocínio.".into(),
    ];
    let instructions = options.custom_instructions.trim();
    if !instructions.is_empty() {
        rules.push(format!("Instruções do usuário: {instructions}"));
    }
    let rules_block = rules.iter().map(|r| format!("- {r}\n")).collect::<String>();

    let mut blocks: Vec<String> = Vec::new();
    if let Some(first) = batch.segments.first() {
        // Mantém os `context_size` segmentos anteriores MAIS RECENTES (o
        // contexto do batcher vem em ordem cronológica; o relevante para o
        // próximo segmento é o do fim), preservando a ordem original.
        let context: Vec<&String> = first
            .context
            .iter()
            .rev()
            .take(options.context_size)
            .collect();
        if !context.is_empty() {
            let mut ctx = String::from("Contexto anterior (já traduzido):\n");
            for c in context.into_iter().rev() {
                ctx.push_str(&format!("- {c}\n"));
            }
            blocks.push(ctx);
        }
    }
    if !glossary.is_empty() {
        let mut g = String::from("Glossário (termo → tradução):\n");
        for entry in glossary {
            g.push_str(&format!("- {} → {}\n", entry.term, entry.translation));
        }
        blocks.push(g);
    }
    let mut numbered = String::new();
    for seg in &batch.segments {
        numbered.push_str(&format!("[{id}] {text}\n", id = seg.id, text = seg.text));
    }
    blocks.push(numbered);

    format!(
        "<|im_start|>system\n\
         Você é um tradutor profissional de legendas. Traduza as legendas do {src} para o {tgt}.\n\
         \n\
         Regras:\n\
         {rules_block}\
         <|im_end|>\n\
         <|im_start|>user\n{user}<|im_end|>\n\
         <|im_start|>assistant\n",
        user = blocks.join("\n")
    )
}

/// Nome legível (pt-BR) do idioma para o prompt. Desconhecido → código ISO.
/// `ponytail:` duplica a `lang_name` privada de `llm.rs` (atrás de `--features
/// llama` e fora do escopo 3.7); consolidar num único lugar quando 3.10 mexer no
/// módulo.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::engine::BatchSegment;

    fn batch(segments: Vec<BatchSegment>) -> Batch {
        Batch { segments }
    }

    fn segs() -> Vec<BatchSegment> {
        vec![
            BatchSegment {
                id: 1,
                text: "Olá, como vai você?".into(),
                context: vec!["Fala anterior.".into()],
            },
            BatchSegment {
                id: 2,
                text: "O gato subiu na árvore.".into(),
                context: vec!["Fala anterior.".into(), "Olá, como vai você?".into()],
            },
        ]
    }

    /// Snapshot canônico do template — mudanças são intencionais: alterar qualquer
    /// texto do prompt exige atualizar esta constante junto. Com as opções
    /// **default** (5.4: coloquial, sem instruções, contexto 3) a saída é a
    /// base — ver também [`SNAPSHOT_FORMAL`] para o prompt com opções ativas.
    const SNAPSHOT: &str = "<|im_start|>system\n\
        Você é um tradutor profissional de legendas. Traduza as legendas do português para o inglês.\n\
        \n\
        Regras:\n\
        - Traduza de forma coloquial e natural, adequada ao ritmo de leitura de legendas.\n\
        - Mantenha nomes próprios, marcas e lugares sem tradução.\n\
        - Quando possível, mantenha cada linha com no máximo 42 caracteres.\n\
        - Mantenha o formato [N] antes de cada tradução, uma linha por segmento.\n\
        - Responda SOMENTE com as linhas numeradas, sem explicação ou raciocínio.\n\
        <|im_end|>\n\
        <|im_start|>user\n\
        Contexto anterior (já traduzido):\n\
        - Fala anterior.\n\
        \n\
        Glossário (termo → tradução):\n\
        - Dragon → Dragão\n\
        - Lannister → Lannister\n\
        \n\
        [1] Olá, como vai você?\n\
        [2] O gato subiu na árvore.\n\
        <|im_end|>\n\
        <|im_start|>assistant\n";

    /// Snapshot do prompt com as opções avançadas ativas (5.4): formal + instrução
    /// livre + contexto 1. A formalidade troca a 1ª regra, a instrução vira uma
    /// regra extra e o contexto é limitado a 1 segmento anterior.
    const SNAPSHOT_FORMAL: &str = "<|im_start|>system\n\
        Você é um tradutor profissional de legendas. Traduza as legendas do português para o inglês.\n\
        \n\
        Regras:\n\
        - Traduza de forma formal e cuidada, adequada ao ritmo de leitura de legendas.\n\
        - Mantenha nomes próprios, marcas e lugares sem tradução.\n\
        - Quando possível, mantenha cada linha com no máximo 42 caracteres.\n\
        - Mantenha o formato [N] antes de cada tradução, uma linha por segmento.\n\
        - Responda SOMENTE com as linhas numeradas, sem explicação ou raciocínio.\n\
        - Instruções do usuário: preservar apelidos\n\
        <|im_end|>\n\
        <|im_start|>user\n\
        Contexto anterior (já traduzido):\n\
        - Fala anterior.\n\
        \n\
        Glossário (termo → tradução):\n\
        - Dragon → Dragão\n\
        - Lannister → Lannister\n\
        \n\
        [1] Olá, como vai você?\n\
        [2] O gato subiu na árvore.\n\
        <|im_end|>\n\
        <|im_start|>assistant\n";

    fn default_options() -> TranslationOptions {
        TranslationOptions::default()
    }

    #[test]
    fn snapshot_do_template_nao_regride() {
        let prompt = build_prompt(
            &batch(segs()),
            &[
                GlossaryEntry {
                    term: "Dragon".into(),
                    translation: "Dragão".into(),
                },
                GlossaryEntry {
                    term: "Lannister".into(),
                    translation: "Lannister".into(),
                },
            ],
            &LanguagePair {
                source: &Language::Pt,
                target: &Language::En,
            },
            &default_options(),
        );
        assert_eq!(prompt, SNAPSHOT);
    }

    #[test]
    fn opcoes_alteram_o_prompt_snapshot_formal() {
        let prompt = build_prompt(
            &batch(segs()),
            &[
                GlossaryEntry {
                    term: "Dragon".into(),
                    translation: "Dragão".into(),
                },
                GlossaryEntry {
                    term: "Lannister".into(),
                    translation: "Lannister".into(),
                },
            ],
            &LanguagePair {
                source: &Language::Pt,
                target: &Language::En,
            },
            &TranslationOptions {
                formality: Formality::Formal,
                custom_instructions: "preservar apelidos".into(),
                context_size: 1,
            },
        );
        assert_eq!(prompt, SNAPSHOT_FORMAL);
    }

    #[test]
    fn default_coloquial_sem_instrucoes_nao_alteram_o_prompt() {
        let prompt = build_prompt(
            &batch(segs()),
            &[],
            &LanguagePair {
                source: &Language::Pt,
                target: &Language::En,
            },
            &default_options(),
        );
        assert!(prompt.contains("coloquial e natural"));
        assert!(!prompt.contains("Instruções do usuário"));
        assert!(!prompt.contains("formal"));
    }

    #[test]
    fn contexto_respeita_nivel_de_contexto() {
        let seg = BatchSegment {
            id: 9,
            text: "Linha.".into(),
            context: vec!["primeiro".into(), "segundo".into(), "terceiro".into()],
        };
        let pair = LanguagePair {
            source: &Language::Pt,
            target: &Language::En,
        };
        let p0 = build_prompt(
            &batch(vec![seg.clone()]),
            &[],
            &pair,
            &TranslationOptions {
                context_size: 0,
                ..default_options()
            },
        );
        assert!(!p0.contains("Contexto anterior"));

        let p2 = build_prompt(
            &batch(vec![seg]),
            &[],
            &pair,
            &TranslationOptions {
                context_size: 2,
                ..default_options()
            },
        );
        assert!(p2.contains("Contexto anterior"));
        assert!(p2.contains("- segundo"));
        assert!(p2.contains("- terceiro"));
        assert!(!p2.contains("- primeiro"));
    }

    #[test]
    fn prompt_sempre_contem_anti_thinking_e_formato_numerado() {
        let cases = [
            build_prompt(
                &batch(segs()),
                &[],
                &LanguagePair {
                    source: &Language::En,
                    target: &Language::Pt,
                },
                &default_options(),
            ),
            build_prompt(
                &batch(vec![segs()[0].clone()]),
                &[],
                &LanguagePair {
                    source: &Language::Pt,
                    target: &Language::Ja,
                },
                &default_options(),
            ),
        ];
        for p in &cases {
            assert!(
                p.contains(
                    "Responda SOMENTE com as linhas numeradas, sem explicação ou raciocínio."
                ),
                "anti-thinking sempre presente"
            );
            assert!(p.contains("<|im_start|>system\n"));
            assert!(p.contains("<|im_start|>user\n"));
            assert!(p.contains("<|im_start|>assistant\n"));
        }
        assert!(cases[0].contains("[1] Olá, como vai você?"));
        assert!(cases[0].contains("[2] O gato subiu na árvore."));
    }

    #[test]
    fn lote_sem_contexto_e_sem_glossario_omite_blocos() {
        let seg = BatchSegment {
            id: 7,
            text: "Só esta linha.".into(),
            context: vec![],
        };
        let prompt = build_prompt(
            &batch(vec![seg]),
            &[],
            &LanguagePair {
                source: &Language::Pt,
                target: &Language::En,
            },
            &default_options(),
        );
        assert!(!prompt.contains("Contexto anterior"));
        assert!(!prompt.contains("Glossário"));
        assert!(prompt.contains("[7] Só esta linha."));
    }

    #[test]
    fn contexto_usa_o_do_primeiro_segmento_do_lote() {
        let mut s = segs();
        // 2º segmento tem contexto maior que o 1º — o bloco reflete o 1º (borda do lote)
        let prompt = build_prompt(
            &batch(s.split_off(1)),
            &[],
            &LanguagePair {
                source: &Language::Pt,
                target: &Language::En,
            },
            &default_options(),
        );
        assert!(prompt.contains("- Fala anterior."));
        assert!(!prompt.contains("Contexto anterior (já traduzido):\n- Fala anterior.\n\n- Olá"));
    }

    #[test]
    fn nomes_de_idioma_legiveis_no_prompt() {
        let prompt = build_prompt(
            &batch(vec![segs()[0].clone()]),
            &[],
            &LanguagePair {
                source: &Language::Es,
                target: &Language::Other("ko".into()),
            },
            &default_options(),
        );
        assert!(prompt.contains("do espanhol para o ko"));
    }
}
