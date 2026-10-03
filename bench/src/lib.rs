//! `chauffeur-bench`: compare OpenCode setups (plain, with Chauffeur, and
//! with one Chauffeur capability disabled) on a suite of tasks.

pub mod audit;
pub mod cli;
pub mod clock;
pub mod daemon;
pub mod events;
pub mod fsutil;
pub mod inputs;
pub mod jobs;
pub mod judges;
pub mod process;
pub mod report;
pub mod result;
pub mod run;
pub mod runner;
pub mod stats;
pub mod verify;
pub mod which;
pub mod workspace;
