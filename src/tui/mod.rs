//! Full-screen terminal UI opened by a bare `whk` on a terminal or by `whk ui`.
pub mod action;
pub mod app;
pub mod budget;
pub mod forms;
pub mod model;
pub mod names;
pub mod poller;
pub mod screen;
pub mod settings;
pub mod status;
pub mod theme;

#[cfg(test)]
pub(crate) mod fixtures;
pub mod hints;
pub mod keys;
pub mod worker;
