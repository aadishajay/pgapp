use super::{ItemType, RenderArgs};
use crate::html::{escape, js_escape};

/// A "Pop Up LOV": a button showing the current choice, opening a
/// native `<dialog>` listing every (value, label) pair in `args.choices`
/// (fixed list or named query, same as Radio) behind a search box that
/// filters the list client-side as you type (see `pgapp.filterPopup`/
/// `pgapp.openPopup` in `/runtime.js` — every choice is already in the
/// DOM, so filtering never needs a server round trip). Picking one
/// calls the pgapp runtime's `setItem(name, value)` instead of touching
/// the DOM directly, so any custom action code can capture/set the
/// same item the same way.
pub struct Popup;

impl ItemType for Popup {
    fn kind(&self) -> &'static str {
        "popup"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let dialog_id = format!("pgapp-popup-dialog-{name}");
        let display_id = format!("pgapp-popup-display-{name}");
        let search_id = format!("pgapp-popup-search-{name}");

        let mut html = format!(
            r#"<div class="pgapp-popup">
<input type="hidden" name="{name}" value="{value}">
<button type="button" class="pgapp-btn pgapp-btn-secondary" onclick="pgapp.openPopup('{dialog_id}', '{search_id}')">
<span id="{display_id}">{display}</span>
</button>
<dialog id="{dialog_id}" class="pgapp-popup-dialog">
<input type="search" id="{search_id}" class="pgapp-input pgapp-popup-search" placeholder="Search…" autocomplete="off" oninput="pgapp.filterPopup('{dialog_id}', this.value)">
<ul class="pgapp-popup-list">"#,
            value = escape(args.value),
            display = if args.value.is_empty() {
                "Choose\u{2026}".to_string()
            } else {
                escape(args.value)
            },
        );

        for (choice_value, choice_label) in args.choices {
            // JS-escape first (protects the single-quoted JS string),
            // then HTML-escape the result (protects the attribute).
            let js_value = escape(&js_escape(choice_value));
            html.push_str(&format!(
                r#"<li><button type="button" onclick="pgapp.setItem('{name}', '{js_value}'); document.getElementById('{dialog_id}').close();">{label}</button></li>"#,
                label = escape(choice_label),
            ));
        }
        html.push_str(r#"<li class="pgapp-popup-empty" style="display:none">No matches</li>"#);

        html.push_str(&format!(
            r#"</ul><button type="button" class="pgapp-btn" onclick="document.getElementById('{dialog_id}').close()">Cancel</button></dialog></div>"#
        ));
        html
    }
}
