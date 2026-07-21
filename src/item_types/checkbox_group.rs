use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A group of checkboxes over `args.choices` — Oracle APEX's "Checkbox
/// Group" item type, storing a comma-separated list of the checked
/// values in one `text` field. The individual `<input>`s carry no
/// `name` of their own (so the browser never submits them directly);
/// a single hidden input under the real field name is what actually
/// submits, kept in sync by `pgapp.syncCheckboxGroup` (runtime.js)
/// every time a box is toggled — the same "one real input, JS keeps it
/// in sync" idiom `popup` uses, chosen so this needed no changes to
/// `server.rs`'s plain `HashMap<String, String>` form parsing (which
/// only ever keeps the *last* value for a repeated key, not a real
/// multi-value list).
pub struct CheckboxGroup;

impl ItemType for CheckboxGroup {
    fn kind(&self) -> &'static str {
        "checkbox_group"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let selected: Vec<&str> = args.value.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();

        let mut html = format!(
            r#"<div class="pgapp-checkbox-group"><input type="hidden" name="{name}" value="{value}">"#,
            value = escape(args.value),
        );
        for (choice_value, choice_label) in args.choices {
            let checked = if selected.contains(&choice_value.as_str()) { " checked" } else { "" };
            html.push_str(&format!(
                r#"<label class="pgapp-checkbox-group-option"><input type="checkbox" value="{cv}"{checked} onchange="pgapp.syncCheckboxGroup(this)"> {cl}</label>"#,
                cv = escape(choice_value),
                cl = escape(choice_label),
            ));
        }
        html.push_str("</div>");
        html
    }
}
