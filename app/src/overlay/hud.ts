// HUD card: eval, side to move, depth/nps, win percentage, top lines with PV and
// an explanation sentence. Layout is computed when data changes, never per frame.
import type { AnalysisFrame, Corner, Settings } from '../shared/types';
import { Spring, clamp } from './anim';
import { formatEval, formatPv } from './chess';
import { mixRgb, parseColor, rgba } from './color';
import { Env, Rect, fitText, font, roundRect, snap, textWidth, wrapText } from './env';

interface Row {
  y: number;
  h: number;
  dot: number;
  san: string;
  cls: string;
  clsX: number;
  eval: string;
  pv: string | null;
}

interface Model {
  w: number;
  h: number;
  compact: boolean;
  hasFrame: boolean;
  mate: number | null;
  white: boolean;          // side to move
  sideText: string;
  stat1: string | null;
  stat2: string | null;
  winPct: number | null;
  winLeft: string;
  winRight: string;
  winY: number;
  dividerY: number;
  rows: Row[];
  explLines: string[];
  explY: number;
  explColor: number;
  thinking: boolean;
  /** Searching a new position below the display depth: skeleton instead of lines. */
  loading: { depth: number; target: number; frac: number; nps: number } | null;
  prelim: boolean;
  /** Win/draw/loss percentages (white POV), when the engine provided them. */
  wdl: [number, number, number] | null;
  wdlRaw: [number, number, number] | null;
  skelRows: number;
}

export interface HudLoading { depth: number; target: number; nps: number; side: 'w' | 'b' | null; frac: number }

const SKEL_W = [[44, 70], [36, 82], [52, 58]];

const MARGIN_X = 20;
const MARGIN_TOP = 36;
const MARGIN_BOTTOM = 24;
const PAD = 14;

const CORNERS: Corner[] = ['tl', 'tr', 'bl', 'br'];

export type HudMode = 'none' | 'hover' | 'drag';

export class Hud {
  private x = new Spring(0, 18, 1, 0.05);
  private y = new Spring(0, 18, 1, 0.05);
  private h = new Spring(0, 16, 1, 0.05);
  private vis = new Spring(0, 15, 1, 0.004);
  private flash = new Spring(1, 13, 1, 0.004);
  private pawns = new Spring(0, 15, 1, 0.004);
  private model: Model | null = null;
  private opts: Settings['hud'] | null = null;
  private settings: Settings | null = null;
  private fen = '';
  private W = 0;
  private H = 0;
  private placed = false;
  private mode: HudMode = 'none';
  private dragging = false;
  private pawnQ = NaN;
  private pawnStr = '0.00';

  /** True while the skeleton is on screen and needs the shimmer to animate. */
  get loading(): boolean {
    return !!this.model?.loading && !!this.opts?.show && !this.still;
  }
  private still = false;

  setMode(m: HudMode): boolean {
    if (m === this.mode) return false;
    this.mode = m;
    return true;
  }

  /** True when the card is (mostly) visible. */
  get visible(): boolean {
    return this.vis.x > 0.3;
  }

  private readonly rect: Rect = { x: 0, y: 0, w: 0, h: 0 };

  /** Current animated bounds. The returned object is reused: copy it to keep it. */
  bounds(): Rect | null {
    const m = this.model;
    if (!m || !this.opts || !this.opts.show) return null;
    const sc = this.opts.scale;
    const r = this.rect;
    r.x = this.x.x; r.y = this.y.x; r.w = m.w * sc; r.h = this.h.x * sc;
    return r;
  }

  private slot(corner: Corner, w: number, h: number): { x: number; y: number } {
    const left = corner === 'tl' || corner === 'bl';
    const top = corner === 'tl' || corner === 'tr';
    return {
      x: left ? MARGIN_X : Math.max(MARGIN_X, this.W - MARGIN_X - w),
      y: top ? MARGIN_TOP : Math.max(MARGIN_TOP, this.H - MARGIN_BOTTOM - h),
    };
  }

  /** Corner whose quadrant contains the point. */
  nearestCorner(cx: number, cy: number): Corner {
    const left = cx < this.W / 2;
    const top = cy < this.H / 2;
    return top ? (left ? 'tl' : 'tr') : left ? 'bl' : 'br';
  }

  beginDrag(): void {
    this.dragging = true;
    this.x.w = 45;
    this.y.w = 45;
  }

  /** Move the card so its top-left corner follows (x, y). */
  dragTo(x: number, y: number): void {
    this.x.set(x, true);
    this.y.set(y, true);
  }

  endDrag(): Corner {
    const b = this.bounds();
    const corner = b ? this.nearestCorner(b.x + b.w / 2, b.y + b.h / 2) : this.opts?.corner ?? 'br';
    this.dragging = false;
    this.x.w = 18;
    this.y.w = 18;
    this.retarget(true);
    return corner;
  }

  resize(W: number, H: number): void {
    this.W = W;
    this.H = H;
    this.retarget(false);
  }

  private retarget(animate: boolean): void {
    const m = this.model;
    const o = this.opts;
    if (!m || !o || this.dragging || this.W === 0) return;
    const sc = o.scale;
    const p = this.slot(o.corner, m.w * sc, m.h * sc);
    const anim = animate && this.placed;
    this.x.set(p.x, anim);
    this.y.set(p.y, anim);
    this.h.set(m.h, anim);
    this.placed = true;
  }

  update(env: Env, frame: AnalysisFrame | null, settings: Settings, animate: boolean, loading: HudLoading | null = null, prelim = false): void {
    this.settings = settings;
    this.opts = settings.hud;
    this.still = !animate;
    const o = settings.hud;
    this.vis.set(o.show ? 1 : 0, animate);
    const wasLoading = !!this.model?.loading;
    this.model = this.build(env, frame, settings, loading, prelim);
    // Lines fade in after the skeleton instead of popping.
    if (wasLoading && !loading && frame && animate) {
      this.flash.x = 0;
      this.flash.v = 0;
      this.flash.t = 1;
    }
    if (frame) {
      const p = frame.evalCp === null ? 0 : clamp(frame.evalCp / 100, -30, 30);
      this.pawns.set(p, animate);
      if (frame.fen !== this.fen) {
        this.fen = frame.fen;
        if (animate) {
          this.flash.x = 0.25;
          this.flash.v = 0;
        }
        this.flash.t = 1;
      }
    } else {
      this.pawns.set(0, animate);
      this.fen = '';
    }
    this.retarget(animate);
  }

  step(dt: number): boolean {
    const a = this.x.step(dt);
    const b = this.y.step(dt);
    const c = this.h.step(dt);
    const d = this.vis.step(dt);
    const e = this.flash.step(dt);
    const f = this.pawns.step(dt);
    return a || b || c || d || e || f;
  }

  // ------------------------------------------------------------- layout
  private build(env: Env, frame: AnalysisFrame | null, st: Settings, loading: HudLoading | null, prelim: boolean): Model {
    const { ctx } = env;
    const o = st.hud;
    const compact = o.compact;
    const w = compact ? 244 : 304;
    const cw = w - PAD * 2;
    const rowsWanted = clamp(Math.round(o.lines), 0, 5);
    const m: Model = {
      w, h: 0, compact, hasFrame: frame !== null, mate: frame ? frame.mate : null,
      white: frame ? frame.sideToMove === 'w' : true,
      sideText: '', stat1: null, stat2: null, winPct: null, winLeft: '', winRight: '',
      winY: 0, dividerY: 0, rows: [], explLines: [], explY: 0, explColor: 0, thinking: frame ? frame.thinking : false,
      loading: null, prelim: frame !== null && prelim, wdl: null, wdlRaw: null, skelRows: 0,
    };
    let y = PAD;
    if (!frame && loading) {
      // Skeleton: same geometry as the real card, so it grows into it without a jump.
      m.loading = { depth: loading.depth, target: loading.target, frac: loading.frac, nps: loading.nps };
      m.white = loading.side !== 'b';
      m.sideText = loading.side === 'b' ? 'Black to move' : 'White to move';
      m.thinking = true;
      y += compact ? 26 : 46;
      m.winY = y + 4;
      y += 4 + 5 + (compact ? 0 : 18) + 4;
      m.skelRows = clamp(rowsWanted, 0, 3);
      if (m.skelRows > 0) {
        m.dividerY = y;
        y += 6 + m.skelRows * (compact ? 26 : 28);
      }
      m.h = y + PAD - 4;
      return m;
    }
    if (!frame) {
      m.sideText = 'Waiting for a position';
      y += compact ? 28 : 44;
      m.h = y + PAD - (compact ? 4 : 2);
      return m;
    }
    m.sideText = frame.sideToMove === 'w' ? 'White to move' : 'Black to move';
    if (o.showDepth) {
      m.stat1 = `depth ${frame.depth}`;
      m.stat2 = frame.nps >= 1e6 ? `${(frame.nps / 1e6).toFixed(2)} Mn/s` : frame.nps >= 1e3 ? `${Math.round(frame.nps / 1e3)} kn/s` : null;
    }
    y += compact ? 26 : 46;

    if (o.showWinPct) {
      m.winPct = clamp(frame.winPct, 0, 100);
      const raw = frame.wdl;
      const sum = raw ? raw[0] + raw[1] + raw[2] : 0;
      if (raw && sum > 0 && raw.every((v) => Number.isFinite(v) && v >= 0)) {
        const w = Math.round((raw[0] / sum) * 100);
        const bl = Math.round((raw[2] / sum) * 100);
        m.wdl = [w, Math.max(0, 100 - w - bl), bl];
        m.wdlRaw = [raw[0] / sum, raw[1] / sum, raw[2] / sum];
      }
      m.winY = y + 4;
      if (compact) {
        y += 4 + 7;
      } else {
        const wp = m.winPct;
        m.winLeft = m.wdl ? `White ${m.wdl[0]}%` : `White ${wp.toFixed(1)}%`;
        m.winRight = m.wdl ? `${m.wdl[2]}% Black` : `${(100 - wp).toFixed(1)}% Black`;
        y += 4 + 6 + 17;
      }
      y += 4;
    }

    const rows = Math.min(rowsWanted, frame.candidates.length);
    if (rows > 0) {
      m.dividerY = y;
      y += 6;
      const fs = font(700, 14);
      const fp = font(500, 11);
      for (let i = 0; i < rows; i++) {
        const c = frame.candidates[i];
        const cs = st.labels.classes[c.cls];
        const pvText = o.showPv && !compact && c.pvSan.length > 1 ? formatPv(frame.fen, c.pvSan, 9) : null;
        const rh = compact ? 26 : pvText ? 44 : 28;
        const sanW = textWidth(ctx, fs, c.san);
        m.rows.push({
          y, h: rh, dot: parseColor(cs.color), san: c.san,
          cls: compact ? '' : cs.label,
          clsX: PAD + 17 + sanW + 8,
          eval: formatEval(c.scoreCp, c.mate, 2),
          pv: pvText ? fitText(ctx, fp, pvText, cw - 17) : null,
        });
        y += rh;
      }
    }

    const top = frame.candidates[0];
    if (o.showExplanation && !compact && top && top.explanation) {
      m.explLines = wrapText(ctx, font(500, 11.5), top.explanation, cw - 12, 3);
      m.explColor = parseColor(st.labels.classes[top.cls].color);
      y += 4;
      m.explY = y;
      y += m.explLines.length * 16 + 2;
    }
    m.h = y + PAD - 4;
    return m;
  }

  // --------------------------------------------------------------- draw
  private pawnLabel(): string {
    const q = Math.round(this.pawns.x * 100);
    if (q !== this.pawnQ) {
      this.pawnQ = q;
      this.pawnStr = formatEval(q, null, 2);
    }
    return this.pawnStr;
  }

  drawGuides(env: Env, alpha: number): void {
    const o = this.opts;
    const m = this.model;
    if (!o || !m || this.mode !== 'drag') return;
    const { ctx, pal } = env;
    const sc = o.scale;
    const w = m.w * sc;
    const h = m.h * sc;
    const b = this.bounds();
    const near = b ? this.nearestCorner(b.x + b.w / 2, b.y + b.h / 2) : o.corner;
    ctx.save();
    ctx.setLineDash([7, 5]);
    ctx.lineWidth = 1.5;
    for (let i = 0; i < 4; i++) {
      const c = CORNERS[i];
      const p = this.slot(c, w, h);
      roundRect(ctx, p.x, p.y, w, h, 16 * sc);
      if (c === near) {
        ctx.fillStyle = rgba(pal.accent, 0.14 * alpha);
        ctx.fill();
        ctx.strokeStyle = rgba(pal.accent, 0.95 * alpha);
      } else {
        ctx.strokeStyle = rgba(pal.accent, 0.4 * alpha);
      }
      ctx.stroke();
    }
    ctx.restore();
  }

  draw(env: Env, alpha: number): void {
    const o = this.opts;
    const m = this.model;
    const st = this.settings;
    const a = alpha * this.vis.x * (o ? o.opacity : 1);
    if (!o || !m || !st || a < 0.01) return;
    const { ctx, dpr, pal } = env;
    const sc = o.scale;
    const h = Math.max(24, this.h.x);
    const px = snap(this.x.x, dpr);
    const py = snap(this.y.x, dpr);
    const content = clamp(0.35 + 0.65 * this.flash.x, 0, 1);

    ctx.save();
    ctx.globalAlpha = a;
    ctx.translate(px, py);
    ctx.scale(sc, sc);

    // Card body with soft shadow.
    roundRect(ctx, 0, 0, m.w, h, 16);
    ctx.shadowColor = 'rgba(0,0,0,0.42)';
    ctx.shadowBlur = 26 * dpr;
    ctx.shadowOffsetY = 8 * dpr;
    ctx.fillStyle = rgba(pal.bg, pal.bgAlpha);
    ctx.fill();
    ctx.shadowColor = 'transparent';
    ctx.shadowBlur = 0;
    ctx.shadowOffsetY = 0;
    ctx.lineWidth = 1;
    ctx.strokeStyle = rgba(pal.border, pal.dark ? 0.12 : 0.14);
    roundRect(ctx, 0.5, 0.5, m.w - 1, h - 1, 15.5);
    ctx.stroke();

    ctx.save();
    roundRect(ctx, 0, 0, m.w, h, 16);
    ctx.clip();
    // Accent hairline along the top edge.
    ctx.fillStyle = rgba(pal.accent, 0.9);
    ctx.fillRect(16, 0, m.w - 32, 2);

    ctx.textBaseline = 'middle';
    const right = m.w - PAD;

    // ---- header
    ctx.textAlign = 'left';
    ctx.fillStyle = rgba(pal.text, 1);
    if (m.loading) {
      this.drawLoadingHeader(env, m, right);
    } else if (!m.hasFrame) {
      ctx.font = font(700, 14);
      ctx.fillText(m.sideText, PAD, PAD + 12);
    } else {
      const evalStr = m.mate !== null ? formatEval(null, m.mate) : this.pawnLabel();
      if (m.compact) {
        ctx.font = font(800, 17);
        ctx.fillText(evalStr, PAD, PAD + 10);
        const ew = textWidth(ctx, font(800, 17), evalStr);
        this.sideDisc(ctx, pal.dark, m.white, PAD + ew + 12, PAD + 10, 4.5);
      } else {
        ctx.font = font(800, 27);
        ctx.fillText(evalStr, PAD, PAD + 15);
        this.sideDisc(ctx, pal.dark, m.white, PAD + 5, PAD + 36, 4.5);
        ctx.font = font(600, 11.5);
        ctx.fillStyle = rgba(pal.muted, 1);
        ctx.fillText(m.sideText, PAD + 15, PAD + 36);
        if (m.prelim) this.prelimPill(ctx, pal.dark, PAD + 15 + textWidth(ctx, font(600, 11.5), m.sideText) + 8, PAD + 36);
      }
      if (m.compact && m.prelim) this.prelimPill(ctx, pal.dark, PAD + textWidth(ctx, font(800, 17), evalStr) + 24, PAD + 10);
      // Right column: depth / speed and the thinking dot.
      ctx.textAlign = 'right';
      ctx.fillStyle = rgba(pal.muted, 1);
      if (m.stat1) {
        ctx.font = font(600, 11, true);
        ctx.fillText(m.stat1, right - 13, PAD + (m.compact ? 10 : 8));
        if (m.stat2 && !m.compact) {
          ctx.font = font(500, 10.5, true);
          ctx.fillStyle = rgba(pal.faint, 1);
          ctx.fillText(m.stat2, right, PAD + 24);
        }
      }
      ctx.beginPath();
      ctx.arc(right - 4, PAD + (m.compact ? 10 : 8), 3.5, 0, Math.PI * 2);
      if (m.thinking) {
        ctx.fillStyle = rgba(pal.accent, 1);
        ctx.fill();
      } else {
        ctx.lineWidth = 1.25;
        ctx.strokeStyle = rgba(pal.faint, 1);
        ctx.stroke();
      }
      ctx.textAlign = 'left';
    }

    // ---- win percentage bar
    if (m.winPct !== null) {
      const bw = m.w - PAD * 2;
      const bh = m.compact ? 5 : 6;
      const wy = m.winY;
      const white = parseColor(st.evalBar.whiteColor);
      const black = parseColor(st.evalBar.blackColor);
      roundRect(ctx, PAD, wy, bw, bh, bh / 2);
      ctx.save();
      ctx.clip();
      ctx.fillStyle = rgba(black, 1);
      ctx.fillRect(PAD, wy, bw, bh);
      if (m.wdlRaw) {
        // Stacked win / draw / loss: white wins | draws | black wins.
        const ww = bw * m.wdlRaw[0];
        const dw = bw * m.wdlRaw[1];
        ctx.fillStyle = rgba(white, 1);
        ctx.fillRect(PAD, wy, ww, bh);
        ctx.fillStyle = rgba(mixRgb(white, black, 0.5), 1);
        ctx.fillRect(PAD + ww, wy, dw, bh);
      } else {
        ctx.fillStyle = rgba(white, 1);
        ctx.fillRect(PAD, wy, bw * (m.winPct / 100), bh);
      }
      ctx.restore();
      roundRect(ctx, PAD + 0.5, wy + 0.5, bw - 1, bh - 1, bh / 2);
      ctx.lineWidth = 1;
      ctx.strokeStyle = rgba(pal.border, pal.dark ? 0.25 : 0.3);
      ctx.stroke();
      if (!m.compact) {
        ctx.font = font(600, 10.5);
        ctx.fillStyle = rgba(pal.muted, 1);
        ctx.textAlign = 'left';
        ctx.fillText(m.winLeft, PAD, wy + bh + 11);
        if (m.wdl) {
          ctx.textAlign = 'center';
          ctx.fillText(`Draw ${m.wdl[1]}%`, PAD + bw / 2, wy + bh + 11);
        }
        ctx.textAlign = 'right';
        ctx.fillText(m.winRight, right, wy + bh + 11);
        ctx.textAlign = 'left';
      }
    }

    // ---- skeleton lines while analysing
    if (m.loading) this.drawSkeletonRows(env, m);

    // ---- lines
    if (m.rows.length > 0) {
      ctx.globalAlpha = a * content;
      ctx.fillStyle = rgba(pal.border, pal.dark ? 0.1 : 0.12);
      ctx.fillRect(PAD, m.dividerY, m.w - PAD * 2, 1);
      const fs = font(700, 14);
      const fcl = font(600, 10.5);
      const fev = font(700, 12.5, true);
      const fpv = font(500, 11);
      for (let i = 0; i < m.rows.length; i++) {
        const r = m.rows[i];
        if (i === 0) {
          roundRect(ctx, 6, r.y + 1, m.w - 12, r.h - 2, 9);
          ctx.fillStyle = rgba(pal.accent, pal.dark ? 0.14 : 0.13);
          ctx.fill();
        }
        const cy = r.y + (m.compact ? 13 : 15);
        // class colour dot with a thin ring so it reads on any card colour
        ctx.beginPath();
        ctx.arc(PAD + 5, cy, 4.75, 0, Math.PI * 2);
        ctx.fillStyle = rgba(r.dot, 1);
        ctx.fill();
        ctx.lineWidth = 1;
        ctx.strokeStyle = rgba(mixRgb(r.dot, pal.dark ? 0xffffff : 0x000000, 0.35), 0.9);
        ctx.stroke();
        ctx.textAlign = 'left';
        ctx.font = fs;
        ctx.fillStyle = rgba(pal.text, 1);
        ctx.fillText(r.san, PAD + 17, cy + 0.5);
        if (r.cls) {
          ctx.font = fcl;
          ctx.fillStyle = rgba(pal.dark ? r.dot : mixRgb(r.dot, 0x000000, 0.35), 1);
          ctx.fillText(r.cls, r.clsX, cy + 1);
        }
        ctx.font = fev;
        ctx.textAlign = 'right';
        ctx.fillStyle = rgba(pal.text, 0.92);
        ctx.fillText(r.eval, right, cy + 0.5);
        if (r.pv) {
          ctx.textAlign = 'left';
          ctx.font = fpv;
          ctx.fillStyle = rgba(pal.muted, 1);
          ctx.fillText(r.pv, PAD + 17, cy + 19);
        }
      }
      ctx.textAlign = 'left';
    }

    // ---- explanation
    if (m.explLines.length > 0) {
      ctx.globalAlpha = a * content;
      const eh = m.explLines.length * 16;
      roundRect(ctx, PAD, m.explY + 1, 3, eh - 2, 1.5);
      ctx.fillStyle = rgba(m.explColor, 1);
      ctx.fill();
      ctx.font = font(500, 11.5);
      ctx.fillStyle = rgba(pal.text, 0.86);
      for (let i = 0; i < m.explLines.length; i++) {
        ctx.fillText(m.explLines[i], PAD + 11, m.explY + 8 + i * 16);
      }
    }
    ctx.restore();

    // ---- edit affordance
    if (this.mode !== 'none') {
      ctx.globalAlpha = a;
      roundRect(ctx, -4, -4, m.w + 8, h + 8, 20);
      ctx.setLineDash([7, 5]);
      ctx.lineWidth = 1.75;
      ctx.strokeStyle = rgba(pal.accent, this.mode === 'drag' ? 1 : 0.85);
      ctx.stroke();
      ctx.setLineDash([]);
    }
    ctx.restore();
  }

  /** Rounded placeholder block with an optional light sweep (static when motion is reduced). */
  private skel(env: Env, m: Model, x: number, y: number, w: number, hgt: number, r: number): void {
    const { ctx, pal } = env;
    roundRect(ctx, x, y, w, hgt, r);
    ctx.fillStyle = rgba(pal.text, pal.dark ? 0.09 : 0.08);
    ctx.fill();
    if (this.still) return;
    const cx = ((env.t * 0.85) % 1.9 - 0.45) * m.w;
    const g = ctx.createLinearGradient(cx - 70, 0, cx + 70, 0);
    const c = pal.dark ? '255,255,255' : '15,21,31';
    g.addColorStop(0, `rgba(${c},0)`);
    g.addColorStop(0.5, `rgba(${c},${pal.dark ? 0.2 : 0.13})`);
    g.addColorStop(1, `rgba(${c},0)`);
    ctx.save();
    roundRect(ctx, x, y, w, hgt, r);
    ctx.clip();
    ctx.fillStyle = g;
    ctx.fillRect(x, y, w, hgt);
    ctx.restore();
  }

  private drawLoadingHeader(env: Env, m: Model, right: number): void {
    const { ctx, pal } = env;
    const ld = m.loading as NonNullable<Model['loading']>;
    if (m.compact) {
      this.skel(env, m, PAD, PAD + 2, 54, 16, 8);
      this.sideDisc(ctx, pal.dark, m.white, PAD + 54 + 12, PAD + 10, 4.5);
    } else {
      this.skel(env, m, PAD, PAD + 2, 92, 26, 9);
      this.sideDisc(ctx, pal.dark, m.white, PAD + 5, PAD + 36, 4.5);
      ctx.font = font(600, 11.5);
      ctx.fillStyle = rgba(pal.muted, 1);
      ctx.textAlign = 'left';
      ctx.fillText(m.sideText, PAD + 15, PAD + 36);
    }
    ctx.textAlign = 'right';
    ctx.fillStyle = rgba(pal.muted, 1);
    ctx.font = font(600, 11, true);
    ctx.fillText(ld.target > 0 ? `depth ${ld.depth}/${ld.target}` : `depth ${ld.depth}`, right - 13, PAD + (m.compact ? 10 : 8));
    if (!m.compact) {
      const n = ld.nps;
      const t = n >= 1e6 ? `${(n / 1e6).toFixed(2)} Mn/s` : n >= 1e3 ? `${Math.round(n / 1e3)} kn/s` : '';
      if (t) {
        ctx.font = font(500, 10.5, true);
        ctx.fillStyle = rgba(pal.faint, 1);
        ctx.fillText(t, right, PAD + 24);
      }
    }
    ctx.beginPath();
    ctx.arc(right - 4, PAD + (m.compact ? 10 : 8), 3.5, 0, Math.PI * 2);
    ctx.fillStyle = rgba(pal.accent, 1);
    ctx.fill();
    ctx.textAlign = 'left';

    // Depth progress: slim accent bar with a "Analysing" caption.
    const bw = m.w - PAD * 2;
    const by = m.winY;
    roundRect(ctx, PAD, by, bw, 5, 2.5);
    ctx.fillStyle = rgba(pal.text, pal.dark ? 0.12 : 0.11);
    ctx.fill();
    const fw = Math.max(5, bw * ld.frac);
    roundRect(ctx, PAD, by, fw, 5, 2.5);
    ctx.fillStyle = rgba(pal.accent, 1);
    ctx.fill();
    if (!m.compact) {
      ctx.font = font(600, 10.5);
      ctx.fillStyle = rgba(pal.muted, 1);
      ctx.fillText(this.still ? 'Analysing...' : 'Analysing\u2026', PAD, by + 5 + 11);
      ctx.textAlign = 'right';
      ctx.fillText(ld.target > 0 ? `moves show at depth ${ld.target}` : 'moves show soon', right, by + 5 + 11);
      ctx.textAlign = 'left';
    }
  }

  private drawSkeletonRows(env: Env, m: Model): void {
    if (m.skelRows <= 0) return;
    const { ctx, pal } = env;
    ctx.fillStyle = rgba(pal.border, pal.dark ? 0.1 : 0.12);
    ctx.fillRect(PAD, m.dividerY, m.w - PAD * 2, 1);
    const right = m.w - PAD;
    const rh = m.compact ? 26 : 28;
    for (let i = 0; i < m.skelRows; i++) {
      const y = m.dividerY + 6 + i * rh;
      const cy = y + (m.compact ? 13 : 15);
      const [a, b] = SKEL_W[i % SKEL_W.length];
      ctx.beginPath();
      ctx.arc(PAD + 5, cy, 4.75, 0, Math.PI * 2);
      ctx.fillStyle = rgba(pal.text, pal.dark ? 0.12 : 0.1);
      ctx.fill();
      this.skel(env, m, PAD + 17, cy - 6, a, 12, 6);
      if (!m.compact) this.skel(env, m, PAD + 17 + a + 8, cy - 4, b * 0.55, 8, 4);
      this.skel(env, m, right - 38, cy - 6, 38, 12, 6);
    }
  }

  /** Small amber "PRELIMINARY" tag: lines are shown early and may still change. */
  private prelimPill(ctx: CanvasRenderingContext2D, dark: boolean, x: number, cy: number): void {
    const f = font(700, 9);
    const label = 'PRELIMINARY';
    ctx.font = f;
    const w = textWidth(ctx, f, label) + 12;
    roundRect(ctx, x, cy - 8, w, 16, 8);
    ctx.fillStyle = dark ? 'rgba(255,179,64,0.18)' : 'rgba(176,106,0,0.14)';
    ctx.fill();
    ctx.lineWidth = 1;
    ctx.strokeStyle = dark ? 'rgba(255,179,64,0.5)' : 'rgba(176,106,0,0.45)';
    ctx.stroke();
    ctx.fillStyle = dark ? '#ffbf5a' : '#8a5200';
    ctx.textAlign = 'left';
    ctx.fillText(label, x + 6, cy + 0.5);
  }

  /** Small white/black disc showing who moves. */
  private sideDisc(ctx: CanvasRenderingContext2D, dark: boolean, white: boolean, cx: number, cy: number, r: number): void {
    ctx.beginPath();
    ctx.arc(cx, cy, r, 0, Math.PI * 2);
    ctx.fillStyle = white ? '#f4f8fc' : '#111926';
    ctx.fill();
    ctx.lineWidth = 1.25;
    ctx.strokeStyle = white ? (dark ? 'rgba(255,255,255,0.55)' : 'rgba(16,23,34,0.55)') : (dark ? 'rgba(255,255,255,0.7)' : 'rgba(16,23,34,0.7)');
    ctx.stroke();
  }
}
