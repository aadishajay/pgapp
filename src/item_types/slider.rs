use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A range slider with a live numeric readout. Demonstrates the
/// extension point this module exists for: it reads its own config
/// keys (`min`/`max`/`step`, each optional) straight out of the field's
/// generic JSON config — no other file needed to know these keys exist.
pub struct Slider;

impl ItemType for Slider {
    fn kind(&self) -> &'static str {
        "slider"
    }

    fn render(&self, args: RenderArgs) -> String {
        let min = config_number(args.config, "min", 0.0);
        let max = config_number(args.config, "max", 100.0);
        let step = config_number(args.config, "step", 1.0);
        let name = escape(args.field_name);
        let default_value = format_number(min);
        let value = escape(if args.value.is_empty() {
            &default_value
        } else {
            args.value
        });

        format!(
            r#"<div class="pgapp-slider">
<input class="pgapp-slider-input" type="range" name="{name}" min="{min}" max="{max}" step="{step}" value="{value}" oninput="this.nextElementSibling.textContent=this.value">
<output class="pgapp-slider-output">{value}</output>
</div>"#,
            min = format_number(min),
            max = format_number(max),
            step = format_number(step),
        )
    }
}

fn config_number(config: &serde_json::Value, key: &str, default: f64) -> f64 {
    config
        .get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Formats without a trailing `.0` for whole numbers, since `min`/`max`
/// attributes read more naturally that way.
fn format_number(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        n.to_string()
    }
}
