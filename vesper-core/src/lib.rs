//! Vesper core: Discord protocol, session management, configuration and theme
//! logic. This crate is UI-toolkit agnostic and must never depend on GTK.

pub mod config;
pub mod error;
pub mod gateway;
pub mod models;
pub mod props;
pub mod rest;
pub mod session;
pub mod theme;

pub use error::CoreError;
pub use session::{Command, CommandSender, CoreHandle, Event};
