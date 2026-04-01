//! CodeTracer recorder for Leo/Aleo smart contract programs.
//!
//! This crate captures execution traces from Leo programs by parsing
//! function definitions, variable declarations, and expressions,
//! evaluating them in order, and converting the results into the
//! CodeTracer trace format for debugging and analysis.

pub mod finalize;
pub mod recorder;
pub mod replay;
pub mod source_map;
pub mod tracer;
pub mod transition;
