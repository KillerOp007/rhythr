//! Replay simulation: deriving per-note hit/miss results from the replay's
//! frame stream, and the integrity check that compares those derived values
//! against the replay header (project hard rule #1).

pub mod hitreg;
pub mod integrity;
pub mod timebase;
