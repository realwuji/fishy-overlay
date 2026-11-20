//! fishy-engine: Stockfish UCI driver + analysis (classification, tactic tags).
//!
//! CONTRACT FILE: public signatures below are frozen. The engine owner may add
//! items and replace the `todo!()` bodies but must not change existing public
//! signatures or serde field names (they mirror `app/src/shared/types.ts`).

pub mod analysis;
pub mod hardware;
pub mod uci;

pub use analysis::*;
pub use hardware::*;
pub use uci::*;
