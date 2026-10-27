//! Magnus: an autonomous digital worker that ships as an appliance.
//!
//! Kernel pieces: the event log (`log`, `events`, `state`), the execution
//! boundary (`boundary`, `policy`, `vault`), and the run loop (`run`).
//! Everything outside the kernel talks to it through `api`, the adapter
//! contract (`adapters`), MCP (`apps`), or vsock (`sandbox`).

pub mod adapters;
pub mod api;
pub mod apps;
pub mod boundary;
pub mod cli;
pub mod config;
pub mod events;
pub mod inbox;
pub mod kernel;
pub mod log;
pub mod policy;
pub mod run;
pub mod sandbox;
pub mod spawn;
pub mod state;
pub mod tools;
pub mod vault;
pub mod web;
