use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

/// Timestamp em milissegundos. `Display` usa o formato SRT (`HH:MM:SS,mmm`);
/// `to_ass()`/`from_ass()` cobrem o formato ASS (`H:MM:SS.cs`).
/// Serializa como `u64` (ms) para IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(u64);

impl Timestamp {
    pub const ZERO: Timestamp = Timestamp(0);

    pub const fn from_ms(ms: u64) -> Self {
        Self(ms)
    }

    pub fn from_secs_f64(secs: f64) -> Self {
        Self((secs * 1000.0).round() as u64)
    }

    pub const fn as_ms(self) -> u64 {
        self.0
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1000.0
    }

    pub const fn saturating_add_ms(self, ms: u64) -> Self {
        Self(self.0.saturating_add(ms))
    }

    pub const fn saturating_sub_ms(self, ms: u64) -> Self {
        Self(self.0.saturating_sub(ms))
    }

    /// Formato ASS: `H:MM:SS.cs` (horas sem padding, cs = centésimos de segundo).
    pub fn to_ass(self) -> String {
        let (h, m, s, ms) = self.hms();
        format!("{h}:{m:02}:{s:02}.{:02}", ms / 10)
    }

    /// Parseia timestamp SRT (`HH:MM:SS,mmm` ou `HH:MM:SS.mmm`).
    pub fn from_srt(s: &str) -> Result<Self, DomainError> {
        Self::from_fractional(s, 1)
    }

    /// Parseia timestamp ASS (`H:MM:SS.cs`).
    pub fn from_ass(s: &str) -> Result<Self, DomainError> {
        Self::from_fractional(s, 10)
    }

    fn hms(self) -> (u64, u64, u64, u64) {
        let ms = self.0;
        (
            ms / 3_600_000,
            (ms / 60_000) % 60,
            (ms / 1000) % 60,
            ms % 1000,
        )
    }

    /// `fraction`: divisor da parte fracionária (1 para ms SRT, 10 para cs ASS).
    fn from_fractional(s: &str, fraction: u32) -> Result<Self, DomainError> {
        let invalid = || DomainError::InvalidTimestamp(s.trim().to_string());
        let s = s.trim();
        let (time, frac) = s
            .split_once(',')
            .or_else(|| s.split_once('.'))
            .ok_or_else(invalid)?;
        let frac: u32 = frac.parse().map_err(|_| invalid())?;
        if frac >= 1000 / fraction {
            return Err(invalid());
        }
        let sub = frac * fraction;
        let parts: Vec<&str> = time.split(':').collect();
        if parts.len() != 3 {
            return Err(invalid());
        }
        let h: u64 = parts[0].trim().parse().map_err(|_| invalid())?;
        let m: u64 = parts[1].trim().parse().map_err(|_| invalid())?;
        let s: u64 = parts[2].trim().parse().map_err(|_| invalid())?;
        if m >= 60 || s >= 60 {
            return Err(invalid());
        }
        Ok(Timestamp(
            h * 3_600_000 + m * 60_000 + s * 1000 + u64::from(sub),
        ))
    }
}

impl fmt::Display for Timestamp {
    /// SRT: `HH:MM:SS,mmm`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (h, m, s, ms) = self.hms();
        write!(f, "{h:02}:{m:02}:{s:02},{ms:03}")
    }
}

impl FromStr for Timestamp {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_srt(s)
    }
}

/// Idioma de uma legenda/segmento (código ISO 639-1 em minúsculas).
/// Variantes nomeadas cobrem os idiomas mais comuns; `Other` cobre o restante
/// do espaço suportado por Whisper/NLLB sem inflar o enum.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Language {
    Pt,
    En,
    Es,
    Fr,
    De,
    It,
    Ja,
    Zh,
    Ar,
    Ru,
    Other(String),
}

impl Language {
    pub fn as_code(&self) -> &str {
        match self {
            Language::Pt => "pt",
            Language::En => "en",
            Language::Es => "es",
            Language::Fr => "fr",
            Language::De => "de",
            Language::It => "it",
            Language::Ja => "ja",
            Language::Zh => "zh",
            Language::Ar => "ar",
            Language::Ru => "ru",
            Language::Other(c) => c,
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "pt" | "por" | "pt-br" | "pt_br" => Language::Pt,
            "en" | "eng" => Language::En,
            "es" | "spa" => Language::Es,
            "fr" | "fra" | "fre" => Language::Fr,
            "de" | "deu" | "ger" => Language::De,
            "it" | "ita" => Language::It,
            "ja" | "jpn" | "jp" => Language::Ja,
            "zh" | "zho" | "chi" | "cmn" => Language::Zh,
            "ar" | "ara" => Language::Ar,
            "ru" | "rus" => Language::Ru,
            "ko" | "kor" => Language::Other("ko".into()),
            "hi" | "hin" => Language::Other("hi".into()),
            "nl" | "nld" | "dut" => Language::Other("nl".into()),
            "pl" | "pol" => Language::Other("pl".into()),
            "tr" | "tur" => Language::Other("tr".into()),
            "vi" | "vie" => Language::Other("vi".into()),
            "th" | "tha" => Language::Other("th".into()),
            "id" | "ind" => Language::Other("id".into()),
            "yue" | "zh-yue" => Language::Other("yue".into()),
            "und" => Language::auto(),
            "" => Language::auto(),
            other => Language::Other(other.to_string()),
        }
    }

    /// Auto-detecção (default do Whisper) — nenhum idioma concreto forçado.
    pub fn auto() -> Self {
        Language::Other("auto".into())
    }

    /// `true` se representa auto-detecção (não um idioma concreto).
    pub fn is_auto(&self) -> bool {
        matches!(self, Language::Other(c) if c == "auto")
    }
}

impl Serialize for Language {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_code())
    }
}

impl<'de> Deserialize<'de> for Language {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Language::from_code(&String::deserialize(deserializer)?))
    }
}

/// Erros do domínio de legendas.
#[derive(Debug, Error)]
pub enum DomainError {
    #[error("timestamp inválido: `{0}`")]
    InvalidTimestamp(String),
    #[error("segmento inválido: end ({end}ms) deve ser maior que start ({start}ms)")]
    InvalidTiming { start: u64, end: u64 },
}

/// Um trecho de fala com tempo e idioma.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub text: String,
    pub start_ms: Timestamp,
    pub end_ms: Timestamp,
    pub lang: Language,
}

impl Segment {
    /// Constrói um segmento validando `end > start`.
    pub fn new(
        text: impl Into<String>,
        start_ms: Timestamp,
        end_ms: Timestamp,
        lang: Language,
    ) -> Result<Self, DomainError> {
        if end_ms <= start_ms {
            return Err(DomainError::InvalidTiming {
                start: start_ms.as_ms(),
                end: end_ms.as_ms(),
            });
        }
        Ok(Self {
            text: text.into(),
            start_ms,
            end_ms,
            lang,
        })
    }
}

/// Uma legenda (lista ordenada de segmentos) com índice 1-based e idioma.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subtitle {
    pub index: u32,
    pub segments: Vec<Segment>,
    pub language: Language,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_format() {
        assert_eq!(Timestamp::from_ms(0).to_string(), "00:00:00,000");
        assert_eq!(Timestamp::from_ms(999).to_string(), "00:00:00,999");
        assert_eq!(Timestamp::from_ms(3_661_250).to_string(), "01:01:01,250");
        assert_eq!(Timestamp::from_secs_f64(90.25).to_string(), "00:01:30,250");
    }

    #[test]
    fn srt_parse_round_trip() {
        for ms in [0u64, 1, 999, 1_000, 61_500, 3_661_250, 99_999_999] {
            let ts = Timestamp::from_ms(ms);
            let s = ts.to_string();
            assert_eq!(Timestamp::from_str(&s).unwrap(), ts, "SRT {s}");
        }
    }

    #[test]
    fn srt_parse_aceita_ponto_e_fracao_menor() {
        assert_eq!(
            Timestamp::from_srt("00:00:01.500").unwrap(),
            Timestamp::from_ms(1500)
        );
        assert_eq!(
            Timestamp::from_srt("00:00:01,5").unwrap(),
            Timestamp::from_ms(1005)
        );
    }

    #[test]
    fn ass_format() {
        assert_eq!(Timestamp::from_ms(0).to_ass(), "0:00:00.00");
        assert_eq!(Timestamp::from_ms(3_661_250).to_ass(), "1:01:01.25");
        assert_eq!(Timestamp::from_ms(90250).to_ass(), "0:01:30.25");
    }

    #[test]
    fn ass_parse_round_trip() {
        // ASS tem precisão de 10ms (centésimos) — só testar valores múltiplos de 10.
        for ms in [0u64, 1_500, 61_500, 3_661_250, 99_999_990] {
            let ts = Timestamp::from_ms(ms);
            let s = ts.to_ass();
            assert_eq!(Timestamp::from_ass(&s).unwrap(), ts, "ASS {s}");
        }
    }

    #[test]
    fn timestamp_invalido_rejeitado() {
        assert!(Timestamp::from_srt("abc").is_err());
        assert!(Timestamp::from_srt("00:60:00,000").is_err());
        assert!(Timestamp::from_srt("00:00:00,1000").is_err());
        assert!(Timestamp::from_srt("00:00:00").is_err());
        assert!(Timestamp::from_ass("0:00:00").is_err());
        assert!(Timestamp::from_ass("0:00:00.100").is_err());
    }

    #[test]
    fn segment_invalido_rejeitado() {
        for (start, end) in [(1000u64, 1000u64), (2000, 1000)] {
            let err = Segment::new(
                "x",
                Timestamp::from_ms(start),
                Timestamp::from_ms(end),
                Language::Pt,
            )
            .unwrap_err();
            assert!(matches!(err, DomainError::InvalidTiming { .. }));
        }
    }

    #[test]
    fn segment_valido_aceito() {
        let seg = Segment::new(
            "olá",
            Timestamp::from_ms(0),
            Timestamp::from_ms(1500),
            Language::Pt,
        )
        .unwrap();
        assert_eq!(seg.start_ms.as_ms(), 0);
        assert_eq!(seg.end_ms.as_ms(), 1500);
    }

    #[test]
    fn language_code_round_trip() {
        assert_eq!(Language::Pt.as_code(), "pt");
        assert_eq!(Language::from_code("EN"), Language::En);
        assert_eq!(Language::from_code("xx"), Language::Other("xx".into()));
        assert_eq!(Language::Other("xx".into()).as_code(), "xx");

        assert_eq!(
            serde_json::to_value(Language::Pt).unwrap(),
            serde_json::json!("pt")
        );
        assert_eq!(
            serde_json::from_value::<Language>(serde_json::json!("pt")).unwrap(),
            Language::Pt
        );
        assert_eq!(
            serde_json::from_value::<Language>(serde_json::json!("xx")).unwrap(),
            Language::Other("xx".into())
        );
    }

    #[test]
    fn language_auto_e_is_auto() {
        assert!(Language::auto().is_auto());
        assert!(!Language::Pt.is_auto());
        assert!(!Language::from_code("pt").is_auto());
        assert!(Language::from_code("auto").is_auto());
    }
}
