/// Quebrador de linhas para legendas.
///
/// Algoritmo guloso: enche cada linha até `max_chars`, mas prefere quebrar após
/// pontuação de cláusula (`,`, `;`, `:`, `.`, `!`, `?`, `…`, `—`) quando isso
/// não deixa a próxima linha vazia. Palavras isoladas maiores que `max_chars`
/// são quebradas por caracteres (sem hifenização — MVP).
pub fn break_lines(text: &str, max_chars: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() || max_chars == 0 {
        return Vec::new();
    }
    if char_count(text) <= max_chars {
        return vec![text.to_string()];
    }

    let words: Vec<&str> = text.split_whitespace().collect();
    let mut lines = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if char_count(words[i]) > max_chars {
            lines.extend(split_long_word(words[i], max_chars));
            i += 1;
            continue;
        }
        let mut line = String::new();
        let mut len = 0usize;
        let mut j = i;
        while j < words.len() {
            let wlen = char_count(words[j]);
            let sep = usize::from(len > 0);
            if len + sep + wlen > max_chars {
                break;
            }
            if sep > 0 {
                line.push(' ');
            }
            line.push_str(words[j]);
            len += sep + wlen;
            j += 1;
        }
        if j < words.len() {
            if let Some(best) = (i..j).rev().find(|&t| ends_with_clause_punct(words[t])) {
                let best = best + 1;
                if best > i {
                    line = words[i..best].join(" ");
                    j = best;
                }
            }
        }
        lines.push(line);
        i = j;
    }
    lines
}

fn char_count(s: &str) -> usize {
    s.chars().count()
}

/// Quebra uma palavra isolada maior que `max_chars` em pedaços de até `max_chars`
/// caracteres.
fn split_long_word(word: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rem = word.to_string();
    while char_count(&rem) > max_chars {
        out.push(rem.chars().take(max_chars).collect());
        rem = rem.chars().skip(max_chars).collect();
    }
    if !rem.is_empty() {
        out.push(rem);
    }
    out
}

/// `true` se a palavra termina com pontuação que encerra cláusula (bom ponto de
/// quebra de linha em legendas).
fn ends_with_clause_punct(word: &str) -> bool {
    word.chars()
        .last()
        .is_some_and(|c| matches!(c, ',' | ';' | ':' | '.' | '!' | '?' | '…' | '—'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linha_unica_quando_cabe() {
        assert_eq!(break_lines("Olá mundo", 42), vec!["Olá mundo"]);
        assert_eq!(break_lines("", 42), Vec::<String>::new());
        assert_eq!(break_lines("   ", 42), Vec::<String>::new());
    }

    #[test]
    fn quebra_em_fronteira_de_palavra() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbb";
        assert_eq!(
            break_lines(text, 42),
            vec!["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "bbbbb"]
        );
    }

    #[test]
    fn preferencia_por_quebra_apos_pontuacao() {
        let text = "Primeira parte da frase, segunda parte continua aqui";
        let lines = break_lines(text, 42);
        assert_eq!(lines[0], "Primeira parte da frase,");
        assert_eq!(lines[1], "segunda parte continua aqui");
    }

    #[test]
    fn sem_pontuacao_usa_fronteira_gulosa() {
        let lines = break_lines("um dois tres quatro cinco seis", 8);
        assert_eq!(lines, vec!["um dois", "tres", "quatro", "cinco", "seis"]);
    }

    #[test]
    fn palavra_maior_que_max_chars_quebrada_por_caracteres() {
        let lines = break_lines("abcdefghij", 4);
        assert_eq!(lines, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn nunca_excede_max_chars_no_corpus() {
        let long = "e".repeat(120);
        let corpus = [
            "Olá, mundo!",
            "Primeira parte da frase, segunda parte continua aqui",
            "um dois tres quatro cinco seis sete oito nove dez",
            "palavra palavra palavra palavra palavra palavra palavra palavra",
            long.as_str(),
        ];
        for text in corpus {
            for line in break_lines(text, 42) {
                assert!(
                    line.chars().count() <= 42,
                    "linha quebrada longa para `{text}`: {line:?}"
                );
            }
        }
    }
}
