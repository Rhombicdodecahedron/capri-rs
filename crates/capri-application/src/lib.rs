//! # capri-application
//!
//! The use cases (services) and the ports they depend on. Services only see traits, never concrete
//! adapters, so every one of them is tested here with in-memory fakes.

pub mod ports;
pub mod services;

#[cfg(test)]
mod tests;
