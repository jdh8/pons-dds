// A two-tab demo over the pons-dds wasm solver.  The Solve tab hands a PBN
// deal to the solver and renders the JSON it returns (the gin-rummy pattern);
// the Edit tab is a client-side PBN⇄card-palette deal builder that feeds Solve.
import init, { WebSolver } from './pkg/pons_dds_web.js';

const SEATS = ['N', 'E', 'S', 'W'];
const SEAT_NAMES = { N: 'North', E: 'East', S: 'South', W: 'West' };
const SUIT_CLASS = { '♠': 's-s', '♥': 's-h', '♦': 's-d', '♣': 's-c' };
const SUIT_KEYS = { '♠': 'spades', '♥': 'hearts', '♦': 'diamonds', '♣': 'clubs' };
const HAND_ORDER = ['♠', '♥', '♦', '♣']; // spades first in hand panels
const RANKS = ['A', 'K', 'Q', 'J', 'T', '9', '8', '7', '6', '5', '4', '3', '2'];
const HCP = { A: 4, K: 3, Q: 2, J: 1 };
const SEAT_CYCLE = [null, 'N', 'E', 'S', 'W']; // click order; null = unassigned

const id = (x) => document.getElementById(x);

let solver; // the wasm WebSolver, created after init()
let editAssign = {}; // "♠A" → "N" | "E" | "S" | "W"
let solvePBN = ''; // the deal currently shown on the Solve tab

async function main() {
  await init();
  solver = new WebSolver();
  for (const b of document.querySelectorAll('nav button')) {
    b.onclick = () => { location.hash = b.dataset.tab; };
  }
  window.addEventListener('hashchange', () => showTab(location.hash.slice(1)));
  initSolve();
  initEdit();
  showTab(location.hash.slice(1));
}

function showTab(tab) {
  if (!['solve', 'edit'].includes(tab)) tab = 'solve';
  for (const sec of document.querySelectorAll('main > section')) {
    sec.classList.toggle('hidden', sec.id !== tab);
  }
  for (const b of document.querySelectorAll('nav button')) {
    b.classList.toggle('active', b.dataset.tab === tab);
  }
}

// --- solve tab -----------------------------------------------------------------

function initSolve() {
  id('sv-random').onclick = () => setSolveDeal(toPBN(randomDeal()));
  id('sv-edit').onclick = exportToEdit;
  // Dealer/vul only change par + the best lead, but re-solving is instant once
  // the transposition table is warm, so just re-run.
  id('sv-dealer').onchange = () => { if (solvePBN) scheduleSolve(); };
  id('sv-vul').onchange = () => { if (solvePBN) scheduleSolve(); };
  setSolveDeal(toPBN(randomDeal())); // open on a solved random deal
}

// Load the solved deal into the editor and switch to it (the reverse of the
// editor's "Solve →" hand-off).
function exportToEdit() {
  editAssign = fromPBN(solvePBN);
  location.hash = 'edit';
  syncFromBoard();
}

// Show a deal and solve it: paint the compass now, solve after a repaint.
function setSolveDeal(pbn) {
  solvePBN = pbn;
  id('sv-hands').innerHTML = compassHTML(handsFrom(fromPBN(pbn)));
  scheduleSolve();
}

// Solve after a paint so the "solving…" placeholder actually shows; the wasm
// solve blocks the main thread for a few hundred ms.
function scheduleSolve() {
  const box = id('sv-dd');
  box.innerHTML = '<div class="panel-title">Double dummy</div><div class="solving">solving…</div>';
  const pbn = solvePBN;
  setTimeout(() => {
    if (pbn !== solvePBN) return; // a newer deal superseded this one
    let out;
    try {
      out = JSON.parse(solver.solve(pbn, id('sv-vul').value, id('sv-dealer').value));
    } catch (e) {
      box.innerHTML = `<div class="panel-title">Double dummy</div>` +
        `<div class="solving">${escapeHTML(String(e.message || e))}</div>`;
      return;
    }
    if (pbn === solvePBN) box.innerHTML = solveHTML(out);
  }, 50);
}

// DD table (NT on top, then ♠♥♦♣), the par line, and the best opening lead.
function solveHTML(out) {
  const head = '<tr><th></th>' + SEATS.map((x) => `<th>${SEAT_NAMES[x]}</th>`).join('') + '</tr>';
  // Rows arrive ascending (♣ ♦ ♥ ♠ NT); reverse for the conventional NT-first table.
  const rows = out.table.slice().reverse().map((r) =>
    `<tr><th>${colorizeCalls(r.strain)}</th>` + r.tricks.map((t) => `<td>${t}</td>`).join('') + '</tr>',
  ).join('');
  // Par score is signed from N/S; the winning side is whoever the points go to.
  const side = out.par_score < 0 ? 'EW' : 'NS';
  const par = out.par_contracts.length
    ? `<div class="verdict">${Math.abs(out.par_score)} to ${side} — ${out.par_contracts.map(colorizeCalls).join(', ')}</div>`
    : '<div class="verdict">Passed out</div>';
  const lead = out.lead ? leadHTML(out.lead) : '';
  return '<div class="panel-title">Double dummy</div>' +
    `<table class="dd">${head}${rows}</table>${par}${lead}`;
}

// One line for the best opening lead.  Equal leads are always the same suit,
// so collapse them into one bridge-notation holding: ["♥3","♥2"] → "♥32".  A
// solid suit makes every card equal, so cap the ranks shown (a real deal has
// one or two).  The trick count is left off on purpose: the par contract
// already implies it, and "declarer makes N" reads as a contract result
// (6 + N) rather than N tricks.
function leadHTML(l) {
  const CAP = 6;
  const suit = l.best[0].slice(0, -1); // suit glyph, shared by every equal card
  const ranks = l.best.map((c) => c.slice(-1)); // one rank char each (T, not 10)
  const holding = suit + ranks.slice(0, CAP).join('') + (ranks.length > CAP ? '…' : '');
  return `<div class="lead">Best opening lead vs ${colorizeCalls(l.contract)}: <strong>${colorizeCalls(holding)}</strong></div>`;
}

// --- HTML builders -------------------------------------------------------------

// Four suit lines, spades first; a void renders as an em dash.
function handHTML(hand) {
  return HAND_ORDER.map((g) =>
    `<div class="suitline"><span class="${SUIT_CLASS[g]}">${g}</span>` +
    `<span class="ranks">${escapeHTML(hand[SUIT_KEYS[g]]) || '—'}</span></div>`,
  ).join('');
}

// All four hands in compass layout: N top, W left, E right, S bottom.
function compassHTML(hands) {
  const cell = (seat) => {
    const h = hands[seat];
    return `<div class="compass-seat pos-${seat.toLowerCase()}">` +
      (h ? `<div class="seat-head">${SEAT_NAMES[seat]} · ${h.hcp} HCP</div>${handHTML(h)}` : '') +
      '</div>';
  };
  return `<div class="compass">${SEATS.map(cell).join('')}</div>`;
}

// Wrap suit glyphs in per-suit colour spans; safe on already plain text.
function colorizeCalls(text) {
  return escapeHTML(text).replace(/[♠♥♦♣]/g, (g) => `<span class="${SUIT_CLASS[g]}">${g}</span>`);
}

function escapeHTML(str) {
  const d = document.createElement('div');
  d.textContent = str;
  return d.innerHTML;
}

// --- deal editor ---------------------------------------------------------------
//
// A PBN text field that two-way-syncs with a 4×13 card palette (the lichess
// analysis-board idiom).  The whole tab is client-side: PBN is a trivial
// string, so no wasm round-trip.  State is one card→seat map; both the palette
// and the compass render from it.

let editReady = false; // whether the current assignment is a full 13/13/13/13

function initEdit() {
  id('e-pbn').oninput = () => { editAssign = fromPBN(id('e-pbn').value); paintEdit(); };
  id('e-random').onclick = () => { editAssign = randomDeal(); syncFromBoard(); };
  id('e-clear').onclick = () => { editAssign = {}; syncFromBoard(); };
  id('e-copy').onclick = () => navigator.clipboard?.writeText(id('e-pbn').value);
  id('e-solve').onclick = () => {
    if (!editReady) return;
    location.hash = 'solve'; // hand the edited deal to the Solve tab and solve it
    setSolveDeal(toPBN(editAssign));
  };
  id('e-grid').onclick = (ev) => {
    const card = ev.target.closest('button')?.dataset.card;
    if (!card) return;
    const next = SEAT_CYCLE[(SEAT_CYCLE.indexOf(editAssign[card] ?? null) + 1) % SEAT_CYCLE.length];
    if (next) editAssign[card] = next; else delete editAssign[card];
    syncFromBoard();
  };
  editAssign = randomDeal();
  syncFromBoard();
}

// Board edit → repaint everything and push the canonical PBN into the field.
function syncFromBoard() {
  paintEdit();
  id('e-pbn').value = toPBN(editAssign);
}

// Repaint from state only — never touches the text field, so typing is stable.
function paintEdit() {
  id('e-grid').innerHTML = editGridHTML();
  id('e-board').innerHTML = compassHTML(handsFrom(editAssign));
  const n = { N: 0, E: 0, S: 0, W: 0 };
  for (const seat of Object.values(editAssign)) n[seat]++;
  const total = n.N + n.E + n.S + n.W;
  editReady = total === 52 && SEATS.every((s) => n[s] === 13);
  id('e-status').textContent = editReady
    ? 'Full deal ✓ — click a card to cycle N→E→S→W→out, or solve it'
    : `N ${n.N} · E ${n.E} · S ${n.S} · W ${n.W} — ${total}/52 placed`;
  id('e-solve').disabled = !editReady; // the solver needs a complete deal
}

// PBN deal: "N:<N> <E> <S> <W>", each hand "spades.hearts.diamonds.clubs",
// ranks high→low.  We always emit from North (canonical); parsing honours a
// leading seat.
function toPBN(assign) {
  const holding = (seat) => HAND_ORDER.map((g) =>
    RANKS.filter((r) => assign[g + r] === seat).join('')).join('.');
  return 'N:' + SEATS.map(holding).join(' ');
}

// Tolerant parse: optional "<seat>:" prefix, whitespace-split hands clockwise,
// unknown chars (voids '-', 'x' spots) ignored; a repeated card just re-homes.
function fromPBN(text) {
  let s = text.trim();
  let start = 0;
  const m = s.match(/^([NESW])\s*:\s*/i);
  if (m) { start = SEATS.indexOf(m[1].toUpperCase()); s = s.slice(m[0].length); }
  const assign = {};
  s.split(/\s+/).filter(Boolean).forEach((hand, i) => {
    const seat = SEATS[(start + i) % 4];
    hand.split('.').forEach((holding, si) => {
      const g = HAND_ORDER[si];
      if (!g) return;
      for (const ch of holding.toUpperCase()) if (RANKS.includes(ch)) assign[g + ch] = seat;
    });
  });
  return assign;
}

function randomDeal() {
  const deck = HAND_ORDER.flatMap((g) => RANKS.map((r) => g + r));
  for (let i = deck.length - 1; i > 0; i--) { // Fisher–Yates; Math.random is fine (UI only)
    const j = Math.floor(Math.random() * (i + 1));
    [deck[i], deck[j]] = [deck[j], deck[i]];
  }
  return Object.fromEntries(deck.map((c, i) => [c, SEATS[Math.floor(i / 13)]]));
}

// One hand object per seat (ranks-per-suit + HCP), so compassHTML renders as-is.
function handsFrom(assign) {
  const hands = {};
  for (const seat of SEATS) {
    const h = { hcp: 0 };
    for (const g of HAND_ORDER) {
      const ranks = RANKS.filter((r) => assign[g + r] === seat);
      h[SUIT_KEYS[g]] = ranks.join('');
      for (const r of ranks) h.hcp += HCP[r] || 0;
    }
    hands[seat] = h;
  }
  return hands;
}

// 4 suit rows × 13 rank cells; each cell tinted by its owner seat (legend in CSS).
function editGridHTML() {
  return HAND_ORDER.map((g) =>
    `<div class="editrow"><span class="${SUIT_CLASS[g]} editsuit">${g}</span>` +
    RANKS.map((r) => {
      const seat = editAssign[g + r];
      return `<button class="editcell${seat ? ' seat-' + seat.toLowerCase() : ''}" ` +
        `data-card="${g}${r}">${r}<small>${seat || ''}</small></button>`;
    }).join('') + '</div>',
  ).join('');
}

main();
