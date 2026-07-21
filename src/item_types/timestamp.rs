use super::{ItemType, RenderArgs};
use crate::html::escape;

/// A native `<input type=datetime-local>` — Oracle APEX's "Timestamp
/// Picker" item type, the `timestamp`-field counterpart to `date`'s
/// text-field one. `FieldType::Timestamp` columns already store
/// `YYYY-MM-DD HH:MM:SS`-shaped text (see `server.rs`'s handling of
/// that type); the browser control needs a `T` separator instead of a
/// space, so the value is translated both ways at the render boundary
/// rather than changing what's stored.
pub struct TimestampPicker;

impl ItemType for TimestampPicker {
    fn kind(&self) -> &'static str {
        "timestamp"
    }

    fn render(&self, args: RenderArgs) -> String {
        let name = escape(args.field_name);
        let value = escape(&to_datetime_local(args.value));
        let required = if args.required { " required" } else { "" };
        format!(r#"<input class="pgapp-input" type="datetime-local" name="{name}" value="{value}" step="1"{required}>"#)
    }
}

/// A stored `timestamptz` value reads back as `YYYY-MM-DD
/// HH:MM:SS+TZ` (see `server.rs`'s `select_columns`, which casts every
/// column to `::text`) — the `datetime-local` input needs the `T`
/// separator this has a space instead of, and rejects the trailing
/// timezone offset outright, so both get fixed up here. Passes
/// anything that doesn't look like that shape through unchanged rather
/// than guessing.
fn to_datetime_local(raw: &str) -> String {
    let Some((date, time)) = raw.split_once(' ') else {
        return raw.to_string();
    };
    // Safe to split the *time* portion (already separated from the
    // date above) on '+'/'-'/'Z' — only a timezone offset or the "Z"
    // UTC suffix can introduce one here, never the date's own dashes.
    let time = time.split(['+', '-', 'Z']).next().unwrap_or(time);
    format!("{date}T{time}")
}
