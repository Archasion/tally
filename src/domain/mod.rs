//! Pure business logic with no I/O. Everything here is unit-tested.

pub mod dates;
pub mod model;
pub mod money;
pub mod numbering;

pub use model::*;
pub use money::{Money, Quantity};
