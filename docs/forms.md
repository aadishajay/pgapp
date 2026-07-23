[← Back to README](../README.md)

# Forms & item types

- [The `ItemType` trait](#the-itemtype-trait)
- [Built-in item types](#built-in-item-types)
- [Adding a new one](#adding-a-new-one)

See [Components reference](./components.md) for the `form`/`editable_table`
grammar itself (`fields:`, `item <field> as <kind> (...)`, `after_save`).

## The `ItemType` trait

A form field's widget is one small Rust file (`src/item_types/`)
implementing:

```rust
pub trait ItemType: Send + Sync {
    fn kind(&self) -> &'static str;                      // the markup keyword
    fn render(&self, args: RenderArgs) -> String;         // the <input>/etc, unwrapped
    fn read_value(&self, field_name: &str,                // default: raw submitted value
                   values: &HashMap<String, String>) -> String { ... }
}
```

`RenderArgs` carries name/value/required/column-type, raw JSON
`config`, and (for `choices`/`query` configs) already-resolved
`(value, label)` pairs.

## Built-in item types

```
text.rs      default for text/integer/timestamp
readonly.rs  visible, not editable, round-trips via hidden input
checkbox.rs  default for boolean
radio.rs     radio group over args.choices
popup.rs     "Pop Up LOV": <dialog> + search filter + pgapp.setItem(...)
slider.rs    <input type=range>, reads min/max/step from config
date.rs      <input type=date> for a text field storing "YYYY-MM-DD";
             optional min/max config bound the pickable range
select.rs    <select> over args.choices ("Select List")
switch.rs    boolean toggle, same read_value as checkbox ("Switch")
password.rs  <input type=password>, autocomplete=new-password
color.rs     <input type=color>, falls back to #000000 if invalid
timestamp.rs <input type=datetime-local>; converts timestamptz's
             "YYYY-MM-DD HH:MM:SS+TZ" to/from the control's "T" format
textarea.rs  <textarea>, optional rows config (default 4)
checkbox_group.rs  checkboxes over args.choices, comma-joined value
             via hidden input + pgapp.syncCheckboxGroup ("Checkbox Group")
star_rating.rs     click-to-rate stars, optional max config (default 5)
list_manager.rs    add/remove free-text list, comma-joined value
shuttle.rs   dual <select multiple> with move buttons, comma-joined
             value in selected order
rich_text.rs contenteditable + execCommand toolbar ("Rich Text Editor");
             read_value runs submitted HTML through ammonia::clean
             (allow-list sanitizer) before it's ever persisted
file_browse.rs  "File Browse" — <input type=file>, uploads via a dedicated
             multipart route (not the universal Form extractor); the
             stored value is "<file_uploads id>:<filename>"
```

`popup` renders every choice into the dialog up front; its search box
(`pgapp.filterPopup`/`pgapp.openPopup` in `/runtime.js`) filters
client-side by substring, resets on every open, and shows "No matches"
instead of a blank list — all with no server round trip.

## Adding a new one

Write `src/item_types/<name>.rs`, add one line to `registry()` —
nothing else changes, since everything goes through the registry by
kind string and a generic JSON config; `date.rs` is the smallest real
example. This is a compile-time plugin point (rebuild + restart, not
hot-loaded); a misspelled `kind` is caught at sync time.

---

Next: [Charts](./charts.md) · [Theming](./themes.md)
