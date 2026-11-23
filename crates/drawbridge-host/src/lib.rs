//! The native side of the Drawbridge Loam plugin.
//!
//! A Soil Spore never gets a Matrix token, a sidecar token, SQLite, or a
//! socket. It talks to a Wasm adapter, which forwards requests to this process
//! over `loam:plugin/ipc` using line-delimited JSON. This crate owns the JSON
//! framing and dispatch; the real Matrix/MeshCore/Rakazo work lives behind
//! [`Backend`].

pub mod backend;
pub mod ipc;
pub mod matrix;
pub mod meshcore;
pub mod rakazo;

pub use backend::{Backend, IpcRequest, IpcResponse};
