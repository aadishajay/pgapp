//! pgapp as a library: the `pgapp` binary (`src/main.rs`, the app
//! server) and the `cargo-pgapp` binary (`src/bin/cargo-pgapp.rs`, the
//! `cargo pgapp` scaffolding subcommand) both depend on this crate
//! rather than duplicating any of it — `scaffold`'s interactive "ask
//! for a DB connection and an app name, then provision it" flow reuses
//! the exact same `meta`/`item_types`/`actions`/`theme` machinery the
//! server itself runs on startup.

pub mod actions;
pub mod chart_lib;
pub mod control;
pub mod instance;
pub mod html;
pub mod icons;
pub mod item_types;
pub mod markup;
pub mod meta;
pub mod model;
pub mod render;
pub mod scaffold;
pub mod secrets;
pub mod server;
pub mod source;
pub mod theme;
