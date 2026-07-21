use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A dual-listbox multi-select — Oracle APEX's "Shuttle" item type.
/// Stores a comma-separated, order-preserving list of the values moved
/// into the right-hand ("selected") list, same convention as
/// `checkbox_group`/`list_manager`. Both `<select multiple>`s carry no
/// `name` of their own; the one real hidden input is rebuilt from the
/// selected list's current option order by `pgapp.shuttleMove`
/// (runtime.js) every time something moves between the two.
pub struct Shuttle;

impl ItemType for Shuttle {
    fn kind(&self) -> &'static str {
        "shuttle"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let selected: Vec<&str> = args.value.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();

        let mut available_options = String::new();
        let mut selected_options = String::new();
        for (choice_value, choice_label) in args.choices {
            let option = format!(r#"<option value="{cv}">{cl}</option>"#, cv = escape(choice_value), cl = escape(choice_label));
            if selected.contains(&choice_value.as_str()) {
                selected_options.push_str(&option);
            } else {
                available_options.push_str(&option);
            }
        }

        format!(
            r#"<div class="pgapp-shuttle">
<input type="hidden" name="{name}" value="{value}">
<select multiple size="8" class="pgapp-select pgapp-shuttle-list pgapp-shuttle-available">{available_options}</select>
<div class="pgapp-shuttle-controls">
<button type="button" class="pgapp-btn pgapp-btn-secondary" onclick="pgapp.shuttleMove(this, true)">&rarr;</button>
<button type="button" class="pgapp-btn pgapp-btn-secondary" onclick="pgapp.shuttleMove(this, false)">&larr;</button>
</div>
<select multiple size="8" class="pgapp-select pgapp-shuttle-list pgapp-shuttle-selected">{selected_options}</select>
</div>"#,
            value = escape(args.value),
        )
    }
}
