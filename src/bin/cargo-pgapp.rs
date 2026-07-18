//! `cargo pgapp` — the same scaffolding CLI as `pgapp new`/`pgapp
//! create` on the main binary (see `pgapp::scaffold`), reachable as a
//! cargo subcommand once this binary is on `PATH` (`cargo install
//! --path . --bin cargo-pgapp`, or just adding `target/debug`/
//! `target/release` to `PATH` for local development).
//!
//! Cargo's subcommand convention: running `cargo pgapp ARGS...` looks
//! for an executable named `cargo-pgapp` and runs it as `cargo-pgapp
//! pgapp ARGS...` — the subcommand name itself is re-inserted as the
//! first argument. Strip it so this behaves identically whether it's
//! invoked via `cargo pgapp ...` or directly as `cargo-pgapp ...`.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("pgapp") {
        args.remove(1);
    }
    // `cargo pgapp new Foo`/`cargo pgapp create` both work, but the
    // subcommand word is optional here — scaffolding is this binary's
    // only job, so `cargo pgapp Foo` works exactly the same way.
    if matches!(args.get(1).map(|s| s.as_str()), Some("new") | Some("create")) {
        args.remove(1);
    }
    pgapp::scaffold::run(&args[1..]).await
}
