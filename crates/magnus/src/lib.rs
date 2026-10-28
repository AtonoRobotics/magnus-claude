//! Magnus: an autonomous digital worker that ships as an appliance.
//!
//! Kernel pieces: the event log (`log`, `events`, `state`), the execution
//! boundary (`boundary`, `policy`, `vault`) with its vocabulary (`ontology`), and the run loop (`run`).
//! Everything outside the kernel talks to it through `api`, the adapter
//! contract (`adapters`), MCP (`apps`), or vsock (`sandbox`).

pub mod accounts;
pub mod adapters;
pub mod api;
pub mod apps;
pub mod boundary;
pub mod browser;
pub mod chat;
pub mod cli;
pub mod compose;
pub mod config;
pub mod conformance;
pub mod dev;
pub mod events;
pub mod inbox;
pub mod kernel;
pub mod log;
pub mod oidc;
pub mod ontology;
pub mod policy;
pub mod run;
pub mod sandbox;
pub mod spawn;
pub mod speech;
pub mod state;
pub mod tls;
pub mod tools;
pub mod vault;
pub mod web;
