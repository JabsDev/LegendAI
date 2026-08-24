/// Estilo de legenda ASS (Advanced SubStation Alpha) com defaults profissionais.
///
/// Os campos seguem exatamente a linha `Style:` de `[V4+ Styles]`. Cores no
/// formato ASS `&HAABBGGRR` (canal alpha + BGR — ex: branco `&H00FFFFFF`,
/// preto semi-transparente `&H80000000`). `alignment` usa o keypad numérico
/// (2 = centro-inferior), `border_style` 1 = outline + shadow, 3 = caixa opaca.
#[derive(Debug, Clone, PartialEq)]
pub struct AssStyle {
    pub name: String,
    pub font_name: String,
    pub font_size: f64,
    pub primary_colour: String,
    pub secondary_colour: String,
    pub outline_colour: String,
    pub back_colour: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike_out: bool,
    pub scale_x: f64,
    pub scale_y: f64,
    pub spacing: f64,
    pub angle: f64,
    pub border_style: u8,
    pub outline: f64,
    pub shadow: f64,
    pub alignment: u8,
    pub margin_l: u32,
    pub margin_r: u32,
    pub margin_v: u32,
    pub encoding: u32,
}

impl Default for AssStyle {
    fn default() -> Self {
        Self {
            name: "Default".into(),
            font_name: "Sans-serif".into(),
            font_size: 48.0,
            primary_colour: "&H00FFFFFF".into(),
            secondary_colour: "&H0000FFFF".into(),
            outline_colour: "&H00000000".into(),
            back_colour: "&H80000000".into(),
            bold: false,
            italic: false,
            underline: false,
            strike_out: false,
            scale_x: 100.0,
            scale_y: 100.0,
            spacing: 0.0,
            angle: 0.0,
            border_style: 1,
            outline: 2.0,
            shadow: 1.0,
            alignment: 2,
            margin_l: 20,
            margin_r: 20,
            margin_v: 10,
            encoding: 1,
        }
    }
}

impl AssStyle {
    /// Linha `Style:` completa (sem o prefixo `Style: `), com os 23 campos do
    /// `Format:` de `[V4+ Styles]` na mesma ordem.
    pub fn to_style_line(&self) -> String {
        [
            self.name.clone(),
            self.font_name.clone(),
            self.font_size.to_string(),
            self.primary_colour.clone(),
            self.secondary_colour.clone(),
            self.outline_colour.clone(),
            self.back_colour.clone(),
            bool_flag(self.bold).to_string(),
            bool_flag(self.italic).to_string(),
            bool_flag(self.underline).to_string(),
            bool_flag(self.strike_out).to_string(),
            self.scale_x.to_string(),
            self.scale_y.to_string(),
            self.spacing.to_string(),
            self.angle.to_string(),
            self.border_style.to_string(),
            self.outline.to_string(),
            self.shadow.to_string(),
            self.alignment.to_string(),
            self.margin_l.to_string(),
            self.margin_r.to_string(),
            self.margin_v.to_string(),
            self.encoding.to_string(),
        ]
        .join(",")
    }
}

/// Flags bool do ASS usam `-1` (verdadeiro) / `0` (falso).
fn bool_flag(v: bool) -> i8 {
    if v {
        -1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_line_tem_23_campos_na_ordem_do_format() {
        let line = AssStyle::default().to_style_line();
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(fields.len(), 23, "Style: precisa de 23 campos");
        assert_eq!(fields[0], "Default");
        assert_eq!(fields[1], "Sans-serif");
        assert_eq!(fields[2], "48");
        assert_eq!(fields[3], "&H00FFFFFF");
        assert_eq!(fields[7], "0", "bold default false");
        assert_eq!(fields[15], "1", "border_style outline+shadow");
        assert_eq!(fields[18], "2", "alignment centro-inferior");
    }

    #[test]
    fn flags_bool_viram_menos_um_ou_zero() {
        let style = AssStyle {
            bold: true,
            italic: true,
            ..AssStyle::default()
        };
        let line = style.to_style_line();
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(fields[7], "-1");
        assert_eq!(fields[8], "-1");
        assert_eq!(fields[9], "0");
        assert_eq!(fields[10], "0");
    }

    #[test]
    fn estilo_customizado_aparece_na_linha() {
        let style = AssStyle {
            name: "Legenda".into(),
            font_name: "Arial".into(),
            font_size: 56.0,
            primary_colour: "&H00FFFF00".into(),
            outline: 3.0,
            margin_v: 40,
            ..AssStyle::default()
        };
        let line = style.to_style_line();
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(fields[0], "Legenda");
        assert_eq!(fields[1], "Arial");
        assert_eq!(fields[2], "56");
        assert_eq!(fields[3], "&H00FFFF00");
        assert_eq!(fields[16], "3");
        assert_eq!(fields[21], "40", "margin_v é o 22º campo (índice 21)");
    }
}
