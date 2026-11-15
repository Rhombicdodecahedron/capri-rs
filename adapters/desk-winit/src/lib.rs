//! # capri-desk-winit
//!
//! The driving adapter for the desktop: it owns the window and the input, turns clicks and key presses
//! into calls on the application services, and draws what the domain says (a [`capri_domain::Pose`]).
//! No behaviour lives here: falling, perching, blinking and chatting are all decided further in.

pub mod app;
pub mod bubble;
pub mod present;
pub mod scene;
pub mod text;

pub use app::{DeskError, DeskFactory, DeskHooks, DeskOptions, Reload, run};
