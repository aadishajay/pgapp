use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A radio button group over `args.choices` — a fixed markup-declared
/// list, or a named query's rows; this component doesn't need to know
/// which (see `server::query_engine::resolve_field_choices`).
pub struct Radio;

impl ItemType for Radio {
    fn kind(&self) -> &'static str {
        "radio"
    }

    fn render(&self, args: RenderArgs) -> String {
        let mut html = String::from(r#"<div class="pgapp-radio-group">"#);
        for (choice_value, choice_label) in args.choices {
            let checked = if choice_value == args.value { " checked" } else { "" };
            html.push_str(&format!(
                r#"<label class="pgapp-radio-option"><input type="radio" name="{name}" value="{cv}"{checked}> {cl}</label>"#,
                name = escape(args.field_name),
                cv = escape(choice_value),
                cl = escape(choice_label),
            ));
        }
        html.push_str("</div>");
        html
    }
}
