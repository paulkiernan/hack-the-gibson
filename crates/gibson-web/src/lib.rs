//! Hack the Gibson web entry point (wasm-bindgen).
//!
//! Batch 0 scaffold: only installs the panic hook + console logger and logs a banner. The crate
//! body is gated to `wasm32` so `cargo build --workspace` on native hosts builds an empty rlib;
//! `cargo build -p gibson-web --target wasm32-unknown-unknown` compiles this real code.
//! Batch 1 (Web agent) adds the canvas bootstrap, query-param settings, and the rAF loop.
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::prelude::*;

/// Called automatically when the module loads.
#[wasm_bindgen(start)]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).map_err(|e| JsValue::from_str(&e.to_string()))?;
    log::info!("gibson-web scaffold: canvas bootstrap lands in Batch 1");
    Ok(())
}
