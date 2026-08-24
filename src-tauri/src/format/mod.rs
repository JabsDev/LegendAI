pub mod line_breaker;
pub mod rules;
pub mod style;

#[allow(unused_imports)] // API pública consumida por 1.9 e Fase 3
pub use line_breaker::break_lines;
#[allow(unused_imports)]
pub use rules::{cps, format_subtitles, FormattedSubtitle};
#[allow(unused_imports)] // AssStyle consumido pelo serializer ASS (5.1) e export 5.7
pub use style::AssStyle;
