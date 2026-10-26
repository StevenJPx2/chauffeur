//! The Rust side of the playground: functions the browser page calls,
//! compiled to WebAssembly.

mod state_machine;

pub use state_machine::Session;
