//! Browser front end for pons-dds: solve a deal's double-dummy table, par, and
//! best opening lead entirely client-side.  JS passes a PBN string in and gets
//! JSON back (the gin-rummy pattern); the actual solving is the only thing that
//! crosses into wasm.

use contract_bridge::{FullDeal, Rank, Seat, Strain, Suit};
use pons_dds::{
    Board, CurrentTrick, Objective, Par, Solver, Target, TrickCountTable, Vulnerability,
    calculate_par, solve_deal_on,
};
use serde::Serialize;
use wasm_bindgen::prelude::*;

/// One strain's double-dummy row: tricks each declarer takes, in N, E, S, W
/// order.
#[derive(Serialize)]
struct DdRow {
    strain: String,
    tricks: [u8; 4],
}

/// Best opening lead against the par contract.
#[derive(Serialize)]
struct Lead {
    /// The par contract this lead is played against, e.g. `4♥ by S`.
    contract: String,
    /// The opening leader (declarer's LHO).
    leader: String,
    /// The best lead card and its equal alternatives, e.g. `["♦K", "♦Q"]`.
    best: Vec<String>,
    /// Tricks declarer takes against best defense — matches the DD table.
    declarer_tricks: u8,
}

/// The JSON payload returned to JS.
#[derive(Serialize)]
struct SolveOut {
    /// Five rows in ascending strain order: ♣ ♦ ♥ ♠ NT.
    table: Vec<DdRow>,
    /// Par score, signed from North–South's perspective.
    par_score: i32,
    /// Par contracts, e.g. `["4♥ by S"]`; empty when the board is passed out.
    par_contracts: Vec<String>,
    /// Best opening lead, or `None` when the board is passed out.
    lead: Option<Lead>,
}

/// A reusable single-threaded solver.
///
/// The rayon-backed free functions (`solve_deal`, `solve_board`, …) need threads
/// wasm doesn't have, so we drive [`Solver`] directly.  One instance keeps its
/// transposition table warm across successive deals.
#[wasm_bindgen]
pub struct WebSolver {
    solver: Solver,
}

#[wasm_bindgen]
impl WebSolver {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new() -> Self {
        // 32/64 MiB TT: snappy solves, small enough for the wasm heap.
        Self {
            solver: Solver::with_memory(Strain::Notrump, 32, 64),
        }
    }

    /// Solve a PBN deal, returning `{ table, par_score, par_contracts, lead }`
    /// as a JSON string.
    ///
    /// # Errors
    ///
    /// Returns a `JsError` if `pbn` is not a full 52-card PBN deal.
    pub fn solve(&mut self, pbn: &str, vul: &str, dealer: &str) -> Result<String, JsError> {
        let out =
            solve_deal_json(&mut self.solver, pbn, vul, dealer).map_err(|e| JsError::new(&e))?;
        serde_json::to_string(&out).map_err(|e| JsError::new(&e.to_string()))
    }
}

impl Default for WebSolver {
    fn default() -> Self {
        Self::new()
    }
}

/// The solving core, split out so the native test can call it without the wasm
/// `JsError`/JSON round-trip.
fn solve_deal_json(
    solver: &mut Solver,
    pbn: &str,
    vul: &str,
    dealer: &str,
) -> Result<SolveOut, String> {
    let deal: FullDeal = pbn
        .parse()
        .map_err(|_| "could not parse PBN deal".to_owned())?;
    let vul: Vulnerability = vul.parse().unwrap_or(Vulnerability::NONE);
    let dealer: Seat = dealer.parse().unwrap_or(Seat::North);

    let table = solve_deal_on(solver, deal);
    let par = calculate_par(table, vul, dealer);

    Ok(SolveOut {
        table: Strain::ASC
            .iter()
            .map(|&strain| DdRow {
                strain: strain.to_string(),
                tricks: [
                    table[strain].get(Seat::North).get(),
                    table[strain].get(Seat::East).get(),
                    table[strain].get(Seat::South).get(),
                    table[strain].get(Seat::West).get(),
                ],
            })
            .collect(),
        par_score: par.score,
        par_contracts: par
            .contracts
            .iter()
            .map(|pc| format!("{} by {}", pc.contract, pc.declarer))
            .collect(),
        lead: best_lead(solver, deal, &table, &par),
    })
}

/// Best opening lead against the first par contract; `None` when the board is
/// passed out (no par contract).
fn best_lead(
    solver: &mut Solver,
    deal: FullDeal,
    table: &TrickCountTable,
    par: &Par,
) -> Option<Lead> {
    let pc = par.contracts.first()?;
    let trump = pc.contract.bid.strain;
    let declarer = pc.declarer;
    let leader = declarer.lho();

    let board = Board::try_new(deal.into(), CurrentTrick::new(trump, leader)).ok()?;
    let found = solver.solve_board(&Objective {
        board,
        target: Target::Any(None),
    });
    let best = found.plays.first()?;

    // `solve_board` scores tricks for the side on lead — at the opening lead
    // that's the defenders, so declarer takes the other 13.  This must agree
    // with the DD table entry for the same strain and declarer.
    let declarer_tricks = 13 - best.score.get();
    debug_assert_eq!(declarer_tricks, table[trump].get(declarer).get());

    let suit = best.card.suit;
    let best = core::iter::once(best.card.rank)
        .chain(best.equals.iter())
        .map(|rank| card_label(suit, rank))
        .collect();

    Some(Lead {
        contract: format!("{} by {}", pc.contract, declarer),
        leader: leader.to_string(),
        best,
        declarer_tricks,
    })
}

/// A card face as glyph-then-rank, e.g. `♠A`.
fn card_label(suit: Suit, rank: Rank) -> String {
    format!("{suit}{rank}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // North holds all spades, East all hearts, South all diamonds, West all
    // clubs — the README's straight-flush deal.
    const GRAND_SLAM: &str = "N:AKQJT98765432... .AKQJT98765432.. \
                              ..AKQJT98765432. ...AKQJT98765432";

    #[test]
    fn solves_grand_slam_table_par_and_lead() {
        let mut solver = Solver::new(Strain::Notrump);
        let out = solve_deal_json(&mut solver, GRAND_SLAM, "none", "N").unwrap();

        // North ruffs the field and runs all 13 spades.
        let spades = out.table.iter().find(|r| r.strain == "♠").unwrap();
        assert_eq!(spades.tricks[0], 13); // North

        // NS grand slam in spades, non-vulnerable.
        assert_eq!(out.par_score, 1510);

        let lead = out.lead.expect("a par contract exists");
        assert!(lead.contract.starts_with("7♠"), "got {}", lead.contract);
        assert_eq!(lead.leader, "E"); // declarer North's LHO
        assert_eq!(lead.declarer_tricks, 13);
    }
}
