//! The Rust side of the playground: functions the browser page calls,
//! compiled to WebAssembly.

mod question;

use wasm_bindgen::prelude::wasm_bindgen;

pub use question::settle;

/// The playground's version, shown in the page heading.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
