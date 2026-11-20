//! fishy-vision: screen capture, board detection, piece recognition, position tracking.
//!
//! CONTRACT FILE: public signatures below are frozen. The vision owner may add
//! items and replace the `todo!()` bodies but must not change existing public
//! signatures or serde field names (they mirror `app/src/shared/types.ts`).

pub mod capture;
pub mod detect;
pub mod labels;
pub mod orient;
pub mod recognize;
pub mod tracker;

pub use capture::*;
pub use detect::*;
pub use labels::{detect_orientation, detect_orientation_with, learn_labels};
pub use orient::*;
pub use recognize::*;
pub use tracker::*;

#[cfg(test)]
mod orient_tests;
#[cfg(test)]
mod real_tests;
/// Synthetic screen renderer; compiled for tests and for dependants that enable the `test-support` feature.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
#[cfg(test)]
mod tests;
