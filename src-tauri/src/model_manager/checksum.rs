//! Verificação de checksum SHA256 (tarefa 2.3).
//!
//! Calcula o hash de um arquivo em streaming (sem carregar o modelo inteiro em
//! RAM) e o compara com o `sha256` do manifesto do catálogo (2.1) quando
//! disponível. Modelos sem checksum declarado (`sha256: null`) passam com aviso
//! em vez de falha — hashes de GGUF mudam se o repo HF atualizar o arquivo.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::catalog::ModelInfo;

/// Erros de verificação de checksum.
#[derive(Debug, Error)]
pub enum ChecksumError {
    #[error("erro de I/O ao ler `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("checksum inválido para `{path}`: esperado `{expected}`, calculado `{actual}`")]
    Mismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("checksum malformado no catálogo: `{0}` (esperado 64 hex minúsculos)")]
    Malformed(String),
}

/// Calcula o SHA256 (hex minúsculo) de um arquivo em streaming, em blocos de
/// 64KB — modelos têm GB e não cabem em RAM.
pub fn sha256_hex(path: &Path) -> Result<String, ChecksumError> {
    let file = std::fs::File::open(path).map_err(|source| ChecksumError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|source| ChecksumError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

/// Verifica o SHA256 de um arquivo contra o esperado (comparação case-insensitive).
pub fn verify_sha256(path: &Path, expected: &str) -> Result<(), ChecksumError> {
    if !is_valid_sha256(expected) {
        return Err(ChecksumError::Malformed(expected.to_string()));
    }
    let actual = sha256_hex(path)?;
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(ChecksumError::Mismatch {
            path: path.to_path_buf(),
            expected: expected.to_string(),
            actual,
        })
    }
}

/// Checksum no formato esperado: 64 dígitos hex.
pub fn is_valid_sha256(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Verifica o arquivo principal de um modelo baixado (`dest_dir/<file>`) contra
/// o `sha256` do catálogo. Checksum ausente → aviso e passa (modelos podem ter
/// `sha256: null`). Mismatch → apaga o arquivo (e `.part` residual) para não
/// deixar órfão no cache e retorna erro.
pub fn verify_model(model: &ModelInfo, dest_dir: &Path) -> Result<(), ChecksumError> {
    let path = dest_dir.join(&model.file);
    match &model.sha256 {
        None => {
            tracing::warn!(
                "modelo `{}` sem sha256 no catálogo — verificação de integridade pulada",
                model.id
            );
            Ok(())
        }
        Some(expected) => match verify_sha256(&path, expected) {
            Ok(()) => Ok(()),
            Err(e @ ChecksumError::Mismatch { .. }) => {
                tracing::error!("modelo `{}` corrompido: {e} — removendo do cache", model.id);
                let part = part_path(&path);
                for p in [path, part] {
                    let _ = std::fs::remove_file(&p);
                }
                Err(e)
            }
            Err(e) => Err(e),
        },
    }
}

/// Caminho do arquivo parcial de download: `<dest>.part`.
fn part_path(dest: &Path) -> PathBuf {
    let mut os = dest.as_os_str().to_os_string();
    os.push(".part");
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;

    // sha256 conhecido de "hello world" (sem newline).
    const HELLO: &str = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("legendai-csum-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn model(id: &str, file: &str, sha256: Option<&str>) -> ModelInfo {
        let mut v = serde_json::json!({
            "id": id, "kind": "stt", "name": "Teste", "repo_id": "o/r",
            "file": file, "backend": "whisper", "quantization": "q5",
            "size_mb": 1, "min_ram_gb": 1, "quality": 3, "speed": 3,
            "threads_supported": true,
        });
        if let Some(h) = sha256 {
            v["sha256"] = serde_json::json!(h);
        }
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn sha256_hex_confere_com_valor_conhecido() {
        let dir = temp_dir("hex");
        let path = dir.join("data.bin");
        std::fs::write(&path, b"hello world").unwrap();
        assert_eq!(sha256_hex(&path).unwrap(), HELLO);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verifica_sha256_correto_passa() {
        let dir = temp_dir("ok");
        let path = dir.join("data.bin");
        std::fs::write(&path, b"hello world").unwrap();
        assert!(verify_sha256(&path, HELLO).is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn verifica_sha256_errado_falha_com_mismatch() {
        let dir = temp_dir("bad");
        let path = dir.join("data.bin");
        std::fs::write(&path, b"outro conteudo").unwrap();
        let err = verify_sha256(&path, HELLO).unwrap_err();
        assert!(
            matches!(&err, ChecksumError::Mismatch { expected, .. } if expected == HELLO),
            "esperava Mismatch, veio: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sha256_malformado_retorna_erro() {
        let dir = temp_dir("mal");
        let path = dir.join("data.bin");
        std::fs::write(&path, b"hello world").unwrap();
        assert!(matches!(
            verify_sha256(&path, "xyz").unwrap_err(),
            ChecksumError::Malformed(_)
        ));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn modelo_sem_sha256_passa_com_aviso_e_mantem_arquivo() {
        let dir = temp_dir("none");
        let path = dir.join("model.bin");
        std::fs::write(&path, b"dados").unwrap();
        let m = model("m", "model.bin", None);
        assert!(verify_model(&m, &dir).is_ok());
        assert!(path.exists(), "sem checksum não deve remover o arquivo");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn modelo_com_sha256_correto_mantem_arquivo() {
        let dir = temp_dir("good");
        let path = dir.join("model.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let m = model("m", "model.bin", Some(HELLO));
        assert!(verify_model(&m, &dir).is_ok());
        assert!(path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn modelo_com_sha256_errado_remove_arquivo_sem_orfao() {
        let dir = temp_dir("corrupt");
        let path = dir.join("model.bin");
        std::fs::write(&path, b"conteudo corrompido").unwrap();
        std::fs::write(dir.join("model.bin.part"), b"residuo").unwrap();
        let m = model("m", "model.bin", Some(HELLO));

        let err = verify_model(&m, &dir).unwrap_err();
        assert!(
            matches!(&err, ChecksumError::Mismatch { expected, .. } if expected == HELLO),
            "esperava Mismatch, veio: {err}"
        );
        assert!(
            !path.exists() && !dir.join("model.bin.part").exists(),
            "checksum errado não deve deixar arquivo órfão no cache"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
