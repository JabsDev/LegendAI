//! Comandos IPC de salvamento de legendas editadas (tarefa 4.6).
//!
//! `save_subtitles` recebe a grade editada do editor (uma linha = uma legenda
//! SRT: texto + timing in/out em ms) e grava um SRT válido. O backend é a
//! fonte de verdade das regras (critério "Save produz SRT válido"): valida
//! texto não vazio, ≤ `MAX_LINES` linhas, ≤ `MAX_CHARS_PER_LINE` chars por
//! linha, `end > start` e ausência de sobreposição entre legendas
//! consecutivas — o mesmo guarda do pipeline de formatação 1.8.

use serde::{Deserialize, Serialize};

use crate::domain::{Language, Segment, Subtitle, Timestamp};
use crate::format::rules::{MAX_CHARS_PER_LINE, MAX_LINES};
use crate::subtitles::srt::to_srt;

/// Uma linha editável do editor: texto + timing (in/out) em milissegundos.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

/// Grava as legendas editadas como SRT em `path`. Valida cada linha e a
/// ausência de sobreposição; retorna o conteúdo do SRT gravado (para o
/// frontend atualizar o preview).
#[tauri::command(rename_all = "snake_case")]
pub fn save_subtitles(path: String, cues: Vec<Cue>) -> Result<String, String> {
    let srt = build_srt(&cues)?;
    std::fs::write(&path, &srt).map_err(|e| format!("falha ao gravar a legenda: {e}"))?;
    Ok(srt)
}

/// Núcleo testável: valida `cues` e monta o SRT. Mensagens de erro estáveis e
/// acionáveis (padrão 4.8), com o índice (1-based) da legenda problemática.
fn build_srt(cues: &[Cue]) -> Result<String, String> {
    if cues.is_empty() {
        return Err("a legenda não contém nenhuma linha".into());
    }
    let mut subs = Vec::with_capacity(cues.len());
    for (i, cue) in cues.iter().enumerate() {
        let n = i + 1;
        let text = cue.text.trim();
        if text.is_empty() {
            return Err(format!("linha {n}: o texto não pode ser vazio"));
        }
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > MAX_LINES {
            return Err(format!("linha {n}: mais de {MAX_LINES} linhas"));
        }
        for line in &lines {
            if line.chars().count() > MAX_CHARS_PER_LINE {
                return Err(format!(
                    "linha {n}: linha com mais de {MAX_CHARS_PER_LINE} caracteres"
                ));
            }
        }
        if cue.end_ms <= cue.start_ms {
            return Err(format!(
                "linha {n}: o fim deve ser maior que o início ({} → {})",
                Timestamp::from_ms(cue.start_ms),
                Timestamp::from_ms(cue.end_ms)
            ));
        }
        if i > 0 && cue.start_ms < cues[i - 1].end_ms {
            return Err(format!("linha {n}: sobreposição com a linha anterior"));
        }
        subs.push(Subtitle {
            index: n as u32,
            segments: vec![Segment::new(
                text,
                Timestamp::from_ms(cue.start_ms),
                Timestamp::from_ms(cue.end_ms),
                Language::auto(),
            )
            .map_err(|_| format!("linha {n}: timing inválido"))?],
            language: Language::auto(),
        });
    }
    Ok(to_srt(&subs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtitles::srt::parse_srt;

    fn cue(s: u64, e: u64, text: &str) -> Cue {
        Cue {
            start_ms: s,
            end_ms: e,
            text: text.into(),
        }
    }

    #[test]
    fn srt_gerado_e_valido_em_round_trip() {
        let cues = vec![
            cue(1000, 2500, "Primeira legenda"),
            cue(3000, 5000, "Segunda linha que aparece depois"),
            cue(5200, 7000, "Terceira, com texto em duas linhas\nassim"),
        ];
        let srt = build_srt(&cues).unwrap();
        let parsed = parse_srt(&srt).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].segments[0].text, "Primeira legenda");
        assert_eq!(
            parsed[1].segments[0].start_ms.as_ms(),
            3000,
            "timing preservado no round-trip"
        );
        assert_eq!(
            parsed[2].segments[0].text, "Terceira, com texto em duas linhas",
            "texto multilinha preservado (1ª linha)"
        );
        assert_eq!(
            parsed[2].segments[1].text, "assim",
            "2ª linha vira segmento separado no round-trip"
        );
    }

    #[test]
    fn texto_com_mais_de_2_linhas_rejeitado() {
        let err = build_srt(&[cue(0, 1000, "a\nb\nc")]).unwrap_err();
        assert!(err.contains("2 linhas"), "{err}");
    }

    #[test]
    fn linha_acima_de_42_chars_rejeitada() {
        let long = "a".repeat(43);
        let err = build_srt(&[cue(0, 1000, &long)]).unwrap_err();
        assert!(err.contains("42"), "{err}");
    }

    #[test]
    fn texto_vazio_rejeitado() {
        let err = build_srt(&[cue(0, 1000, "   ")]).unwrap_err();
        assert!(err.contains("vazio"), "{err}");
    }

    #[test]
    fn end_menor_ou_igual_a_start_rejeitado() {
        let err = build_srt(&[cue(1000, 1000, "x")]).unwrap_err();
        assert!(err.contains("fim"), "{err}");
        let err = build_srt(&[cue(2000, 1000, "x")]).unwrap_err();
        assert!(err.contains("fim"), "{err}");
    }

    #[test]
    fn sobreposicao_entre_linhas_rejeitada() {
        let err = build_srt(&[cue(0, 2000, "a"), cue(1500, 3000, "b")]).unwrap_err();
        assert!(err.contains("sobreposição"), "{err}");
        // Contíguo (start == end anterior) é válido.
        assert!(build_srt(&[cue(0, 2000, "a"), cue(2000, 3000, "b")]).is_ok());
    }

    #[test]
    fn lista_vazia_rejeitada() {
        let err = build_srt(&[]).unwrap_err();
        assert!(err.contains("nenhuma linha"), "{err}");
    }
}
