#![doc = include_str!("../README.md")]
// The crate is a line-by-line port of a C++ codebase: it mixes `i32`,
// `usize`, `u8`, and `u16` throughout its indexing and bit-twiddling
// math, uses bridge-shorthand bindings (lho/rho, lh/rh), keeps long
// translated functions and large fixed-size lookup tables, and mirrors
// vendor instance methods even when they do not touch `self`. Fighting
// these lints would obscure the correspondence with the vendor source
// without buying real safety — invariants are guaranteed by the
// surrounding bridge-specific context.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::large_stack_arrays,
    clippy::large_stack_frames,
    clippy::needless_pass_by_ref_mut,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unused_self
)]

pub(crate) mod analyse;
pub mod board;
pub(crate) mod convert;
pub(crate) mod dealer_par;
pub(crate) mod later_tricks;
pub(crate) mod lookup;
pub(crate) mod move_type;
pub(crate) mod moves;
pub mod par;
pub mod play;
pub(crate) mod pos;
pub(crate) mod quick_tricks;
pub(crate) mod search;
pub(crate) mod solve_board;
pub mod solver;
pub mod strain_flags;
pub mod tricks;
pub(crate) mod tt;
pub mod vulnerability;

pub use board::{
    Board, BoardError, CurrentTrick, CurrentTrickError, Objective, RevokePosition, Target,
};
pub use contract_bridge::FullDeal;
pub use par::{Par, ParContract, calculate_par, calculate_pars};
pub use play::{FoundPlays, Play, PlayAnalysis, PlayFaultError, PlayFaultKind, PlayTrace};
pub use search::SearchStats;
pub use solver::{
    Solver, analyse_play, analyse_plays, solve_board, solve_boards, solve_boards_with_memory,
    solve_deal, solve_deal_on, solve_deals, solve_deals_with_memory,
};
pub use strain_flags::{NonEmptyStrainFlags, StrainFlags};
pub use tricks::{InvalidTrickCount, TrickCount, TrickCountRow, TrickCountTable};
pub use vulnerability::{ParseVulnerabilityError, Vulnerability};
