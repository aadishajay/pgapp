use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A row of clickable stars — Oracle APEX's "Star Rating" item type.
/// Stores a plain integer (as text, same "everything is text/integer/
/// boolean/timestamp" convention as every other item type) in 1..=max.
/// The stars themselves are plain `<span>`s with no `name` of their
/// own — same "one real hidden input, JS keeps it in sync" idiom
/// `checkbox_group`/`popup` use — clicking one calls
/// `pgapp.setStarRating` (runtime.js), which sets the hidden input and
/// restyles every star up to the clicked position via `.pgapp-star-on`.
/// `max` config key (optional, default 5).
pub struct StarRating;

impl ItemType for StarRating {
    fn kind(&self) -> &'static str {
        "star_rating"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let max: u32 = args.config.get("max").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(5);
        let current: u32 = args.value.parse().unwrap_or(0);

        let mut html = format!(
            r#"<div class="pgapp-star-rating"><input type="hidden" name="{name}" value="{value}">"#,
            value = escape(args.value),
        );
        for i in 1..=max {
            let on = if i <= current { " pgapp-star-on" } else { "" };
            html.push_str(&format!(
                r#"<span class="pgapp-star{on}" data-value="{i}" onclick="pgapp.setStarRating(this, {i})">&#9733;</span>"#
            ));
        }
        html.push_str("</div>");
        html
    }
}
