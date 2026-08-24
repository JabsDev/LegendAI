pub mod ass;
pub mod srt;
pub mod txt;
pub mod vtt;

#[allow(unused_imports)] // API pública consumida por 1.8/1.9, Fase 3 e 5.7
pub use ass::{to_ass, to_ass_dual, to_ass_styled};
#[allow(unused_imports)]
pub use srt::{parse_srt, to_srt, SrtError};
#[allow(unused_imports)] // consumido pelo comando de exportação (5.7)
pub use txt::to_txt;
#[allow(unused_imports)]
pub use vtt::to_vtt;
