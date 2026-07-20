//! Builds the embedded dashboard SPA before the crate is compiled.
//!
//! `rust-embed` bakes `web/dist` into the binary, so that folder must exist and — for a real
//! release — hold the built app. This script runs `npm` to produce it when Node is available,
//! and otherwise drops a placeholder so `cargo build` still succeeds for Rust-only contributors.
//!
//! Locally a frontend build failure is a *warning* (the backend never gets blocked by a broken
//! `node_modules`). **Under CI** (`CI` env set), it's a hard error instead — a release must never
//! silently ship the placeholder. Set `FORGETOP_SKIP_WEB_BUILD=1` to skip npm entirely and embed
//! whatever is already in `web/dist` (used by CI after a dedicated SPA build step).

use std::path::Path;
use std::process::Command;

fn main() {
    let web = Path::new("web");
    let dist = web.join("dist");

    // Only the frontend sources should trigger a rebuild — Rust-side edits must not re-run npm.
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/vite.config.ts");
    println!("cargo:rerun-if-env-changed=FORGETOP_SKIP_WEB_BUILD");

    let has_web = web.join("package.json").exists();
    let skip = std::env::var_os("FORGETOP_SKIP_WEB_BUILD").is_some();
    let ci = std::env::var_os("CI").is_some();

    if has_web && !skip {
        if which_npm() {
            if !web.join("node_modules").exists() {
                run(Command::new(npm_bin()).arg("ci").current_dir(web), "npm ci", ci);
            }
            run(
                Command::new(npm_bin())
                    .args(["run", "build"])
                    .env("FORGETOP_VERSION", env!("CARGO_PKG_VERSION"))
                    .current_dir(web),
                "npm run build",
                ci,
            );
        } else if ci {
            panic!("Node/npm is required to build the dashboard SPA under CI, but `npm` was not found on PATH");
        } else {
            println!("cargo:warning=npm not found — embedding a placeholder dashboard. Install Node to build the real SPA.");
        }
    }

    // rust-embed needs the folder to exist at macro-expansion time; guarantee it, and give the
    // server *something* to serve if the SPA wasn't built. Under CI a missing real build is a
    // failure, not a placeholder — we'd rather fail the release than ship a broken dashboard.
    if !dist.join("index.html").exists() {
        if ci && !skip {
            panic!("web/dist/index.html is missing after the SPA build — refusing to embed a placeholder in CI");
        }
        std::fs::create_dir_all(&dist).expect("create web/dist");
        std::fs::write(dist.join("index.html"), PLACEHOLDER).expect("write placeholder index.html");
    }
}

// On Windows, npm is installed as `npm.cmd`, and `std::process::Command` doesn't resolve `.cmd`
// shims the way a shell does — spawning "npm" directly fails with "program not found" even
// though it's on PATH.
fn npm_bin() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

fn which_npm() -> bool {
    Command::new(npm_bin()).arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn run(cmd: &mut Command, label: &str, fatal: bool) {
    let outcome = match cmd.status() {
        Ok(s) if s.success() => return,
        Ok(s) => format!("`{label}` exited with {s}"),
        Err(e) => format!("failed to run `{label}`: {e}"),
    };
    if fatal {
        panic!("{outcome}");
    }
    println!("cargo:warning={outcome}; embedding whatever is in web/dist.");
}

const PLACEHOLDER: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>forgetop dashboard</title>
<style>body{font-family:ui-monospace,Menlo,monospace;background:#1c1c1c;color:#dadada;max-width:640px;margin:64px auto;padding:0 20px;line-height:1.6}h1{color:#5fafff}</style></head>
<body><h1>&#9727; forgetop dashboard</h1><p>The dashboard app wasn't built. Run <code>npm ci &amp;&amp; npm run build</code> in <code>crates/forgetop-server/web</code>, then rebuild forgetop.</p></body></html>"#;
