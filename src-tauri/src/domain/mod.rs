pub mod subtitle;

#[allow(unused_imports)] // API pública do domínio (consumida por 1.4/1.7/1.8)
pub use subtitle::{DomainError, Language, Segment, Subtitle, Timestamp};
