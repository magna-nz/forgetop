//! Builds the embedded dashboard SPA before the crate is compiled.
//!
//! `rust-embed` bakes `web/dist` into the binary, so that folder must exist and — for a real
//! release — hold the built app. This script runs `npm` to produce it when Node is available,
//! and otherwise drops a placeholder so `cargo build` still succeeds for Rust-only contributors.
//! A frontend build failure is a *warning*, never a hard error, so the backend never gets
//! blocked by a broken `node_modules`. CI builds the SPA in its own step where failures are fatal.

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

    if has_web && !skip && which_npm() {
        if !web.join("node_modules").exists() {
            run(Command::new("npm").arg("ci").current_dir(web), "npm ci");
        }
        run(
            Command::new("npm")
                .args(["run", "build"])
                .env("FORGETOP_VERSION", env!("CARGO_PKG_VERSION"))
                .current_dir(web),
            "npm run build",
        );
    } else if has_web && !skip {
        println!("cargo:warning=npm not found — embedding a placeholder dashboard. Install Node to build the real SPA.");
    }

    // rust-embed needs the folder to exist at macro-expansion time; guarantee it, and give the
    // server *something* to serve if the SPA wasn't built.
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist).expect("create web/dist");
        std::fs::write(dist.join("index.html"), PLACEHOLDER).expect("write placeholder index.html");
    }
}

fn which_npm() -> bool {
    Command::new("npm").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

fn run(cmd: &mut Command, label: &str) {
    match cmd.status() {
        Ok(s) if s.success() => {}
        Ok(s) => println!("cargo:warning=`{label}` exited with {s}; embedding whatever is in web/dist."),
        Err(e) => println!("cargo:warning=failed to run `{label}`: {e}"),
    }
}

const PLACEHOLDER: &str = r#"<!doctype html><html><head><meta charset="utf-8"><title>forgetop dashboard</title>
<style>body{font-family:ui-monospace,Menlo,monospace;background:#1c1c1c;color:#dadada;max-width:640px;margin:64px auto;padding:0 20px;line-height:1.6}h1{color:#5fafff}</style></head>
<body><h1>&#9727; forgetop dashboard</h1><p>The dashboard app wasn't built. Run <code>npm ci &amp;&amp; npm run build</code> in <code>crates/forgetop-server/web</code>, then rebuild forgetop.</p></body></html>"#;
