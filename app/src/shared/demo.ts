// Demo data for previewing the overlay renderer without an engine: realistic
// analysis frames (positions, classes, tactic tags) plus a board geometry helper.
// The control window's live preview imports `demoFrames` and `demoGeometry`.
import type { AnalysisFrame, BoardGeometry, BoardState, CandidateMove, MoveClass, SessionPhase, SessionStatus, TacticTag } from './types';

/** Lichess-style logistic mapping from centipawns to white's expected score (0..100). */
const winFromCp = (cp: number): number => 50 + 50 * Math.tanh(0.00184104 * cp);

/** Classify a candidate from the win-percentage it gives up versus the best line. */
function classify(rank: number, lossPct: number): MoveClass {
  if (rank === 1 || lossPct < 0.5) return 'best';
  if (lossPct <= 2) return 'excellent';
  if (lossPct <= 5) return 'good';
  if (lossPct <= 10) return 'inaccuracy';
  if (lossPct <= 20) return 'mistake';
  return 'blunder';
}

/** Depth the demo engine "aims for" before it shows moves (matches the default display depth). */
export const DEMO_TARGET_DEPTH = 12;

/** Rough win/draw/loss split (per-mille, white POV) from white's expected score 0..100. */
export function wdlFromWin(winPct: number): [number, number, number] {
  const p = Math.min(1, Math.max(0, winPct / 100));
  const draw = Math.round(1000 * 0.62 * 4 * p * (1 - p));
  const w = Math.round(1000 * p - draw / 2);
  const b = Math.max(0, 1000 - w - draw);
  return [Math.max(0, w), draw, b];
}

interface Spec {
  uci: string;
  san: string;
  cp?: number;
  mate?: number;
  tags?: TacticTag[];
  pv: [string, string];           // [space separated UCI, space separated SAN] including the move itself
  explanation?: string;
  cls?: MoveClass;                // override the automatic classification
}

function winOf(spec: Spec): number {
  if (spec.mate !== undefined) return spec.mate > 0 ? 100 : 0;
  return winFromCp(spec.cp ?? 0);
}

function buildFrame(index: number, fen: string, specs: Spec[], depth: number, nps: number): AnalysisFrame {
  const side = fen.split(' ')[1] === 'b' ? 'b' : 'w';
  const best = specs[0];
  const bestWin = winOf(best);
  const candidates: CandidateMove[] = specs.map((s, i) => {
    const win = winOf(s);
    const loss = Math.max(0, side === 'w' ? bestWin - win : win - bestWin);
    const rounded = Math.round(loss * 10) / 10;
    return {
      rank: i + 1,
      uci: s.uci,
      san: s.san,
      from: s.uci.slice(0, 2),
      to: s.uci.slice(2, 4),
      scoreCp: s.mate === undefined ? s.cp ?? 0 : null,
      mate: s.mate ?? null,
      winPct: Math.round(win * 10) / 10,
      lossPct: rounded,
      cls: s.cls ?? classify(i + 1, loss),
      tags: s.tags ?? [],
      pvUci: s.pv[0].split(' '),
      pvSan: s.pv[1].split(' '),
      depth: depth - (i === 0 ? 0 : 1 + (i % 2)),
      explanation: s.explanation ?? null,
    };
  });
  return {
    fen,
    sideToMove: side,
    depth,
    nps,
    evalCp: best.mate === undefined ? best.cp ?? 0 : null,
    mate: best.mate ?? null,
    winPct: Math.round(bestWin * 10) / 10,
    candidates,
    thinking: false,
    ts: 1_760_000_000_000 + index * 4000,
    wdl: wdlFromWin(bestWin),
    targetDepth: DEMO_TARGET_DEPTH,
  };
}

/** A handful of realistic frames (different positions, classes, tags) to cycle through. */
export const demoFrames: AnalysisFrame[] = [
  // 0. Starting position, three principled first moves.
  buildFrame(0, 'rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1', [
    {
      uci: 'e2e4', san: 'e4', cp: 32, pv: ['e2e4 e7e5 g1f3 b8c6 f1b5', 'e4 e5 Nf3 Nc6 Bb5'],
      explanation: 'Claims the centre and opens lines for the queen and the light-squared bishop.',
    },
    {
      uci: 'd2d4', san: 'd4', cp: 29, pv: ['d2d4 d7d5 c2c4 e7e6 b1c3', 'd4 d5 c4 e6 Nc3'],
      explanation: 'Controls e5 and keeps the position solid.',
    },
    {
      uci: 'g1f3', san: 'Nf3', cp: 24, pv: ['g1f3 g8f6 c2c4 g7g6 b1c3', 'Nf3 Nf6 c4 g6 Nc3'],
      explanation: 'Develops with tempo on e5 and keeps every pawn plan open.',
    },
  ], 28, 1_840_000),

  // 1. Knight fork: Nc7+ hits the king and the a8 rook.
  buildFrame(1, 'r3kb1r/pp3ppp/2n2n2/3Np3/4P1bq/8/PPP2PPP/R1BQKB1R w KQkq - 0 1', [
    {
      uci: 'd5c7', san: 'Nc7+', cp: 480, tags: ['fork', 'check'], cls: 'brilliant',
      pv: ['d5c7 e8d8 c7a8', 'Nc7+ Kd8 Nxa8'],
      explanation: 'The knight forks the king on e8 and the rook on a8, so the rook falls next move.',
    },
    {
      uci: 'd5f6', san: 'Nxf6+', cp: 440, tags: ['capture', 'check'],
      pv: ['d5f6 g7f6 d1d2', 'Nxf6+ gxf6 Qd2'],
      explanation: 'Wins a piece for a pawn, but gives up the stronger fork.',
    },
    {
      uci: 'f1b5', san: 'Bb5', cp: 405, tags: ['pin'],
      pv: ['f1b5 a7a6 d5c7 e8d8', 'Bb5 a6 Nc7+ Kd8'],
      explanation: 'Pins the c6 knight to the king, but lets Black untangle.',
    },
  ], 31, 2_110_000),

  // 2. Forced mate in two with a rook ladder.
  buildFrame(2, '4k3/8/8/8/8/8/R7/1R4K1 w - - 0 1', [
    {
      uci: 'a2a7', san: 'Ra7', mate: 2, tags: ['mate'], cls: 'great',
      pv: ['a2a7 e8d8 b1b8', 'Ra7 Kd8 Rb8#'],
      explanation: 'The a7 rook seals the seventh rank, so Rb8 is checkmate next move.',
    },
    {
      uci: 'b1b7', san: 'Rb7', mate: 2, tags: ['mate'],
      pv: ['b1b7 e8d8 a2a8', 'Rb7 Kd8 Ra8#'],
      explanation: 'The mirrored ladder also mates in two.',
    },
    {
      uci: 'a2a8', san: 'Ra8+', cp: 900, tags: ['check'],
      pv: ['a2a8 e8d7 b1b7', 'Ra8+ Kd7 Rb7+'],
      explanation: 'Wins eventually, but lets the king run to the centre and delays the mate.',
    },
  ], 36, 2_760_000),

  // 3. Italian-style middlegame with five arrows and a blunder.
  buildFrame(3, 'r1bqkb1r/pppp1ppp/2n2n2/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4', [
    {
      uci: 'd2d3', san: 'd3', cp: 30, pv: ['d2d3 d7d6 e1g1 f8e7', 'd3 d6 O-O Be7'],
      explanation: 'A calm reinforcement of e4 that prepares to castle and play c3.',
    },
    {
      uci: 'e1g1', san: 'O-O', cp: 28, tags: ['castle'], pv: ['e1g1 f6e4 f1e1 e4d6', 'O-O Nxe4 Re1 Nd6'],
      explanation: 'Gets the king safe, though it allows the symmetrical Nxe4 line.',
    },
    {
      uci: 'c2c3', san: 'c3', cp: -10, pv: ['c2c3 f6e4 c4f7 e8f7', 'c3 Nxe4 Bxf7+ Kxf7'],
      explanation: 'Prepares d4 but is a little slow against Nxe4.',
    },
    {
      uci: 'f3g5', san: 'Ng5', cp: -95, pv: ['f3g5 d7d5 e4d5 f6d5', 'Ng5 d5 exd5 Nxd5'],
      explanation: 'The knight raid on f7 is parried by d5 and White falls behind in development.',
    },
    {
      uci: 'c4f7', san: 'Bxf7+', cp: -280, tags: ['capture', 'check', 'sacrifice'],
      pv: ['c4f7 e8f7 f3e5 f7e8', 'Bxf7+ Kxf7 Nxe5+ Ke8'],
      explanation: 'The sacrifice does not work: Black keeps the extra piece after Kxf7.',
    },
  ], 30, 1_950_000),

  // 4. Black to move in the Ruy Lopez.
  buildFrame(4, 'r1bqkbnr/pppp1ppp/2n5/1B2p3/4P3/5N2/PPPP1PPP/RNBQK2R b KQkq - 3 3', [
    {
      uci: 'a7a6', san: 'a6', cp: 18, pv: ['a7a6 b5a4 g8f6 e1g1 f8e7', 'a6 Ba4 Nf6 O-O Be7'],
      explanation: 'Asks the bishop what it wants to do before Black commits further.',
    },
    {
      uci: 'g8f6', san: 'Nf6', cp: 22, pv: ['g8f6 e1g1 f6e4 d2d4', 'Nf6 O-O Nxe4 d4'],
      explanation: 'The Berlin Defence: develops and hits e4 at once.',
    },
    {
      uci: 'f8c5', san: 'Bc5', cp: 31, pv: ['f8c5 c2c3 g8f6 d2d4 e5d4', 'Bc5 c3 Nf6 d4 exd4'],
      explanation: 'The Classical Defence puts the bishop on an active diagonal.',
    },
    {
      uci: 'd7d6', san: 'd6', cp: 52, pv: ['d7d6 d2d4 c8d7', 'd6 d4 Bd7'],
      explanation: 'Solid, though White keeps a small, comfortable edge.',
    },
  ], 29, 1_720_000),

  // 5. Pin: Bg5 pins the f6 knight to the queen.
  buildFrame(5, 'rnbqkb1r/ppp2ppp/3p1n2/4p3/4P3/2NP1N2/PPP2PPP/R1BQKB1R w KQkq - 0 5', [
    {
      uci: 'c1g5', san: 'Bg5', cp: 36, tags: ['pin'],
      pv: ['c1g5 f8e7 c3d5 e8g8', 'Bg5 Be7 Nd5 O-O'],
      explanation: 'Pins the f6 knight against the queen, adding pressure on d5 and e7.',
    },
    {
      uci: 'f1e2', san: 'Be2', cp: 27, pv: ['f1e2 f8e7 e1g1 e8g8', 'Be2 Be7 O-O O-O'],
      explanation: 'Quiet development that prepares to castle.',
    },
    {
      uci: 'c3d5', san: 'Nd5', cp: 22, pv: ['c3d5 f6d5 e4d5 c8g4', 'Nd5 Nxd5 exd5 Bg4'],
      explanation: 'Trades a knight, but Black develops with tempo.',
    },
  ], 27, 2_030_000),

  // 6. Hanging piece: the queen on h4 is attacked by the knight on f3.
  buildFrame(6, 'rnb1kbnr/pppp1ppp/8/4p3/4P2q/5N2/PPPP1PPP/RNBQKB1R w KQkq - 2 3', [
    {
      uci: 'f3h4', san: 'Nxh4', cp: 880, tags: ['capture', 'hanging'],
      pv: ['f3h4 g8f6 d2d3 d7d5', 'Nxh4 Nf6 d3 d5'],
      explanation: 'The queen on h4 is undefended, so the knight simply wins it.',
    },
    {
      uci: 'b1c3', san: 'Nc3', cp: 150, pv: ['b1c3 f8b4 f3h4 b4c3', 'Nc3 Bb4 Nxh4 Bxc3'],
      explanation: 'Misses the free queen and lets Black untangle.',
    },
    {
      uci: 'f1e2', san: 'Be2', cp: 120, pv: ['f1e2 h4e7 d2d4', 'Be2 Qe7 d4'],
      explanation: 'Gives the queen time to retreat to safety.',
    },
  ], 24, 2_240_000),
];

/** Board geometry that fits a preview area of the given CSS size. */
export function demoGeometry(width: number, height: number, flipped = false): BoardGeometry {
  const size = Math.min(width, height) * 0.8;
  return { x: (width - size) / 2, y: (height - size) / 2, size, flipped };
}

/** Demo states used to preview the overlay status chip and the loading sequence. */
export type DemoStatus =
  | 'none' | 'starting' | 'finding' | 'recognising' | 'analysing' | 'loading'
  | 'calibrate' | 'lost' | 'thinking';
export const DEMO_STATUSES: DemoStatus[] = [
  'none', 'starting', 'finding', 'recognising', 'analysing', 'loading', 'calibrate', 'lost', 'thinking',
];

export function demoBoardState(status: DemoStatus, geometry: BoardGeometry, fen: string | null): BoardState {
  const base = { geometry, fen: null, lastMove: null, confidence: 0 };
  switch (status) {
    case 'calibrate': return { ...base, state: 'needsCalibration', calibrated: false };
    case 'recognising': return { ...base, state: 'needsCalibration', calibrated: true };
    case 'lost': return { ...base, state: 'lost', calibrated: true, fen, confidence: 0.2 };
    case 'thinking': return { ...base, state: 'tracking', calibrated: true, fen, confidence: 0.95 };
    default: return { ...base, state: 'tracking', calibrated: true, fen, confidence: 0.95 };
  }
}

/** Session status for a given phase, with plausible engine facts (this Mac: M4, 6 threads). */
export function demoSessionStatus(phase: SessionPhase, nps = 0): SessionStatus {
  const running = phase !== 'idle';
  return {
    running, permission: true, captureFps: running ? 8 : 0, message: null, phase,
    engine: phase === 'idle' || phase === 'startingEngine'
      ? { name: null, threads: 6, hashMb: 1024, nps: 0, ready: false }
      : { name: 'Stockfish 17.1', threads: 6, hashMb: 1024, nps, ready: true },
  };
}

/** A frame as the engine reports it mid-search: right position, still shallow. */
export function demoPendingFrame(base: AnalysisFrame, depth: number, target = DEMO_TARGET_DEPTH): AnalysisFrame {
  return {
    ...base,
    depth,
    thinking: true,
    targetDepth: target,
    nps: depth < 3 ? 0 : base.nps,
    candidates: base.candidates.map((c) => ({ ...c, depth: Math.max(1, depth - (c.rank === 1 ? 0 : 1)) })),
  };
}
