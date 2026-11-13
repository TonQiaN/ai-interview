// 各幅画的参考图：只画大色块，笔触和厚涂交给 painter.js。
// 颜色都取自设计令牌里的颜料色（见 src/styles/tokens.css）。

const P = {
  prussian: "#1e3a5c",
  prussianDeep: "#12263f",
  ultramarine: "#2f4fa2",
  cerulean: "#6f9bc0",
  viridian: "#2f7566",
  sap: "#56743a",
  ochre: "#c99a3c",
  naples: "#efd08a",
  sienna: "#a65a34",
  vermilion: "#c2452d",
  alizarin: "#8e2b3a",
  umber: "#2a2019",
  umberWarm: "#4a3526",
  flesh: "#e2b088",
  white: "#f6efdf",
};

function linear(ctx, x0, y0, x1, y1, stops) {
  const g = ctx.createLinearGradient(x0, y0, x1, y1);
  for (const [at, color] of stops) g.addColorStop(at, color);
  return g;
}

function radial(ctx, x, y, r0, r1, stops) {
  const g = ctx.createRadialGradient(x, y, r0, x, y, r1);
  for (const [at, color] of stops) g.addColorStop(at, color);
  return g;
}

function ellipse(ctx, x, y, rx, ry, fill, rot = 0) {
  ctx.fillStyle = fill;
  ctx.beginPath();
  ctx.ellipse(x, y, rx, ry, rot, 0, Math.PI * 2);
  ctx.fill();
}

/** 一条起伏的山脊线，填到画面底部。 */
function ridge(ctx, w, h, r, yBase, amp, fill, waves = 3) {
  const ph = r() * 10;
  ctx.fillStyle = fill;
  ctx.beginPath();
  ctx.moveTo(0, h);
  for (let x = 0; x <= w; x += w / 80) {
    const f = x / w;
    const y =
      yBase +
      Math.sin(f * Math.PI * waves + ph) * amp +
      Math.sin(f * Math.PI * waves * 2.7 + ph * 1.7) * amp * 0.35;
    ctx.lineTo(x, y);
  }
  ctx.lineTo(w, h);
  ctx.closePath();
  ctx.fill();
}

/** 暮色里的原野：后台首页横幅。暖色的天在右，深蓝的山在左，留出左侧放标题文字。 */
export function dusk(ctx, w, h, r) {
  const horizon = h * 0.6;
  ctx.fillStyle = linear(ctx, 0, 0, 0, horizon, [
    [0, P.prussianDeep],
    [0.35, "#27466f"],
    [0.68, "#7f8aa0"],
    [0.86, "#e0a868"],
    [1, "#f0c27a"],
  ]);
  ctx.fillRect(0, 0, w, horizon + 2);

  const sx = w * 0.74;
  const sy = horizon - h * 0.07;
  ctx.fillStyle = radial(ctx, sx, sy, 0, w * 0.34, [
    [0, "rgba(250,222,150,0.95)"],
    [0.22, "rgba(240,180,100,0.55)"],
    [1, "rgba(240,180,100,0)"],
  ]);
  ctx.fillRect(0, 0, w, horizon);
  ellipse(ctx, sx, sy, h * 0.062, h * 0.062, "#fbe7a8");

  // 云：被落日从下面照亮
  ctx.filter = `blur(${Math.round(h / 90)}px)`;
  for (let i = 0; i < 16; i++) {
    const x = r() * w;
    const y = h * (0.06 + r() * 0.4);
    const warm = x / w + (y / horizon) * 0.5;
    const color =
      warm > 1.05
        ? "rgba(236,150,96,0.78)"
        : warm > 0.72
          ? "rgba(203,116,96,0.6)"
          : "rgba(88,110,150,0.6)";
    ellipse(
      ctx,
      x,
      y,
      w * (0.05 + r() * 0.1),
      h * (0.014 + r() * 0.026),
      color,
      (r() - 0.5) * 0.12,
    );
  }
  ctx.filter = "none";

  ridge(ctx, w, h, r, horizon - h * 0.04, h * 0.035, "#3d5c8a", 2.2);
  ridge(ctx, w, h, r, horizon + h * 0.005, h * 0.028, "#294a63", 3.1);
  ridge(ctx, w, h, r, horizon + h * 0.05, h * 0.02, "#2c5f55", 4);

  // 田野：赭石到土黄，带几道被夕阳照亮的垄
  ctx.fillStyle = linear(ctx, 0, horizon + h * 0.06, 0, h, [
    [0, "#6f6a3c"],
    [0.35, "#a8793a"],
    [1, "#8a4b2c"],
  ]);
  ctx.beginPath();
  ctx.moveTo(0, h);
  ctx.lineTo(0, horizon + h * 0.11);
  ctx.bezierCurveTo(
    w * 0.3,
    horizon + h * 0.07,
    w * 0.6,
    horizon + h * 0.13,
    w,
    horizon + h * 0.09,
  );
  ctx.lineTo(w, h);
  ctx.closePath();
  ctx.fill();
  for (let i = 0; i < 9; i++) {
    const f = i / 8;
    ctx.strokeStyle = i % 2 ? "rgba(226,178,90,0.75)" : "rgba(122,62,38,0.6)";
    ctx.lineWidth = h * (0.012 + f * 0.022);
    ctx.beginPath();
    ctx.moveTo(w * (0.68 + (f - 0.5) * 0.1), horizon + h * 0.1);
    ctx.quadraticCurveTo(
      w * (0.6 + (f - 0.5) * 0.9),
      horizon + h * 0.22,
      w * (0.5 + (f - 0.5) * 2.4),
      h * 1.02,
    );
    ctx.stroke();
  }
  // 河：映着天光
  ctx.strokeStyle = "rgba(244,206,140,0.9)";
  ctx.lineWidth = h * 0.022;
  ctx.beginPath();
  ctx.moveTo(w * 0.72, horizon + h * 0.075);
  ctx.bezierCurveTo(w * 0.6, horizon + h * 0.12, w * 0.86, horizon + h * 0.2, w * 0.7, h * 0.94);
  ctx.stroke();

  // 左侧几棵柏树
  for (const [fx, fh] of [
    [0.07, 0.36],
    [0.115, 0.27],
    [0.21, 0.2],
    [0.93, 0.17],
  ]) {
    const x = w * fx;
    const base = horizon + h * 0.12;
    ctx.fillStyle = "#17302b";
    ctx.beginPath();
    ctx.moveTo(x - h * fh * 0.13, base);
    ctx.quadraticCurveTo(x - h * fh * 0.2, base - h * fh * 0.55, x, base - h * fh);
    ctx.quadraticCurveTo(x + h * fh * 0.2, base - h * fh * 0.55, x + h * fh * 0.13, base);
    ctx.closePath();
    ctx.fill();
    ctx.fillStyle = "rgba(92,128,84,0.75)";
    ctx.beginPath();
    ctx.ellipse(
      x + h * fh * 0.06,
      base - h * fh * 0.5,
      h * fh * 0.035,
      h * fh * 0.3,
      0.05,
      0,
      Math.PI * 2,
    );
    ctx.fill();
  }
}

/** 抽象的半身像：面试官的形象。没有五官，只有光影。 */
export function sitter(ctx, w, h, _r) {
  ctx.fillStyle = radial(ctx, w * 0.36, h * 0.36, 0, w * 0.95, [
    [0, "#8a6a44"],
    [0.4, "#4f4638"],
    [0.75, "#243449"],
    [1, "#131d2c"],
  ]);
  ctx.fillRect(0, 0, w, h);

  const cx = w * 0.5;
  // 肩和外套
  ctx.fillStyle = linear(ctx, w * 0.1, 0, w * 0.9, 0, [
    [0, "#3b5d8c"],
    [0.45, P.prussian],
    [1, "#101f33"],
  ]);
  ctx.beginPath();
  ctx.moveTo(w * 0.04, h * 1.02);
  ctx.bezierCurveTo(w * 0.06, h * 0.78, w * 0.24, h * 0.7, cx - w * 0.1, h * 0.66);
  ctx.lineTo(cx + w * 0.1, h * 0.66);
  ctx.bezierCurveTo(w * 0.76, h * 0.7, w * 0.94, h * 0.78, w * 0.96, h * 1.02);
  ctx.closePath();
  ctx.fill();
  // 衬衫领口
  ctx.fillStyle = P.white;
  ctx.beginPath();
  ctx.moveTo(cx - w * 0.095, h * 0.665);
  ctx.lineTo(cx, h * 0.86);
  ctx.lineTo(cx + w * 0.095, h * 0.665);
  ctx.closePath();
  ctx.fill();
  ctx.fillStyle = "rgba(160,150,140,0.8)";
  ctx.beginPath();
  ctx.moveTo(cx + w * 0.02, h * 0.7);
  ctx.lineTo(cx, h * 0.86);
  ctx.lineTo(cx + w * 0.095, h * 0.665);
  ctx.closePath();
  ctx.fill();
  // 颈
  ctx.fillStyle = linear(ctx, cx - w * 0.08, 0, cx + w * 0.08, 0, [
    [0, "#d49a6c"],
    [0.6, "#a8683f"],
    [1, "#6c3d28"],
  ]);
  ctx.beginPath();
  ctx.moveTo(cx - w * 0.062, h * 0.54);
  ctx.lineTo(cx - w * 0.07, h * 0.675);
  ctx.lineTo(cx, h * 0.745);
  ctx.lineTo(cx + w * 0.07, h * 0.675);
  ctx.lineTo(cx + w * 0.062, h * 0.54);
  ctx.closePath();
  ctx.fill();
  // 头：左侧受光
  const hy = h * 0.41;
  ctx.fillStyle = linear(ctx, cx - w * 0.17, 0, cx + w * 0.17, 0, [
    [0, "#f0c79a"],
    [0.38, "#dba377"],
    [0.68, "#b0704a"],
    [1, "#6e402b"],
  ]);
  ctx.beginPath();
  ctx.ellipse(cx, hy, w * 0.158, h * 0.2, 0, 0, Math.PI * 2);
  ctx.fill();
  // 眼窝、鼻影、唇的暗部：只是色块，不是五官
  ctx.filter = `blur(${Math.round(w / 110)}px)`;
  ellipse(ctx, cx - w * 0.066, hy - h * 0.012, w * 0.04, h * 0.016, "rgba(110,62,42,0.75)");
  ellipse(ctx, cx + w * 0.07, hy - h * 0.012, w * 0.042, h * 0.018, "rgba(78,40,28,0.8)");
  ellipse(ctx, cx + w * 0.022, hy + h * 0.055, w * 0.016, h * 0.045, "rgba(96,52,36,0.7)");
  ellipse(ctx, cx + w * 0.006, hy + h * 0.125, w * 0.045, h * 0.012, "rgba(142,62,52,0.75)");
  ellipse(ctx, cx - w * 0.085, hy + h * 0.06, w * 0.04, h * 0.03, "rgba(232,150,120,0.55)");
  ctx.filter = "none";
  // 头发
  ctx.fillStyle = linear(ctx, cx - w * 0.2, 0, cx + w * 0.2, 0, [
    [0, "#6a4630"],
    [0.4, "#35241b"],
    [1, "#17110d"],
  ]);
  ctx.beginPath();
  ctx.moveTo(cx - w * 0.175, hy - h * 0.02);
  ctx.bezierCurveTo(
    cx - w * 0.21,
    hy - h * 0.27,
    cx + w * 0.02,
    hy - h * 0.31,
    cx + w * 0.12,
    hy - h * 0.23,
  );
  ctx.bezierCurveTo(
    cx + w * 0.22,
    hy - h * 0.17,
    cx + w * 0.2,
    hy + h * 0.02,
    cx + w * 0.172,
    hy + h * 0.08,
  );
  ctx.bezierCurveTo(
    cx + w * 0.17,
    hy - h * 0.06,
    cx + w * 0.12,
    hy - h * 0.12,
    cx + w * 0.03,
    hy - h * 0.13,
  );
  ctx.bezierCurveTo(
    cx - w * 0.06,
    hy - h * 0.14,
    cx - w * 0.14,
    hy - h * 0.11,
    cx - w * 0.175,
    hy - h * 0.02,
  );
  ctx.closePath();
  ctx.fill();
}

/** 瓶花：空状态和「谢谢参加」。 */
export function bouquet(ctx, w, h, r) {
  ctx.fillStyle = linear(ctx, 0, 0, w, h * 0.7, [
    [0, "#6b7a63"],
    [0.6, "#48584c"],
    [1, "#2c3a35"],
  ]);
  ctx.fillRect(0, 0, w, h);
  // 桌面
  ctx.fillStyle = linear(ctx, 0, h * 0.72, 0, h, [
    [0, "#c08a4a"],
    [1, "#8a5a35"],
  ]);
  ctx.fillRect(0, h * 0.72, w, h * 0.28);
  ellipse(ctx, w * 0.6, h * 0.79, w * 0.26, h * 0.035, "rgba(70,44,28,0.55)");
  // 瓶
  const vx = w * 0.5;
  ctx.fillStyle = linear(ctx, vx - w * 0.13, 0, vx + w * 0.13, 0, [
    [0, "#4a73a8"],
    [0.3, "#2b4f80"],
    [1, P.prussianDeep],
  ]);
  ctx.beginPath();
  ctx.moveTo(vx - w * 0.055, h * 0.5);
  ctx.bezierCurveTo(vx - w * 0.16, h * 0.58, vx - w * 0.15, h * 0.78, vx - w * 0.07, h * 0.8);
  ctx.lineTo(vx + w * 0.07, h * 0.8);
  ctx.bezierCurveTo(vx + w * 0.15, h * 0.78, vx + w * 0.16, h * 0.58, vx + w * 0.055, h * 0.5);
  ctx.closePath();
  ctx.fill();
  ellipse(ctx, vx - w * 0.07, h * 0.63, w * 0.014, h * 0.06, "rgba(220,232,244,0.8)", 0.2);
  // 叶
  for (let i = 0; i < 26; i++) {
    const a = -Math.PI / 2 + (r() - 0.5) * 2.4;
    const d = w * (0.1 + r() * 0.22);
    ellipse(
      ctx,
      vx + Math.cos(a) * d,
      h * 0.47 + Math.sin(a) * d * 0.95,
      w * (0.02 + r() * 0.03),
      w * (0.045 + r() * 0.04),
      r() < 0.5 ? "#2f6a52" : "#577c42",
      a + Math.PI / 2 + (r() - 0.5),
    );
  }
  // 花
  const flowers = [P.vermilion, "#e0703f", "#e6b84c", P.white, P.alizarin, "#e9c9b0", "#d2553a"];
  for (let i = 0; i < 20; i++) {
    const a = -Math.PI / 2 + (r() - 0.5) * 2.6;
    const d = w * (0.05 + r() * 0.26);
    const x = vx + Math.cos(a) * d;
    const y = h * 0.44 + Math.sin(a) * d * 0.9;
    const rad = w * (0.03 + r() * 0.035);
    const color = flowers[(r() * flowers.length) | 0];
    ellipse(ctx, x, y, rad, rad * (0.8 + r() * 0.3), color, r() * 3);
    ellipse(ctx, x - rad * 0.25, y - rad * 0.25, rad * 0.45, rad * 0.4, "rgba(255,240,210,0.45)");
    ellipse(ctx, x + rad * 0.1, y + rad * 0.1, rad * 0.22, rad * 0.22, "rgba(90,50,24,0.7)");
  }
}

/** 月夜的海：链接已过期、出了问题。 */
export function nocturne(ctx, w, h, r) {
  const horizon = h * 0.58;
  ctx.fillStyle = linear(ctx, 0, 0, 0, horizon, [
    [0, "#0d1727"],
    [0.6, "#1b3355"],
    [1, "#35557e"],
  ]);
  ctx.fillRect(0, 0, w, horizon + 1);
  const mx = w * 0.64;
  const my = h * 0.24;
  ctx.fillStyle = radial(ctx, mx, my, 0, w * 0.42, [
    [0, "rgba(240,226,176,0.8)"],
    [0.18, "rgba(150,170,200,0.4)"],
    [1, "rgba(150,170,200,0)"],
  ]);
  ctx.fillRect(0, 0, w, horizon);
  ellipse(ctx, mx, my, w * 0.058, w * 0.058, "#f4e7b8");
  ctx.filter = `blur(${Math.round(h / 80)}px)`;
  for (let i = 0; i < 9; i++) {
    ellipse(
      ctx,
      r() * w,
      h * (0.08 + r() * 0.36),
      w * (0.08 + r() * 0.12),
      h * (0.015 + r() * 0.02),
      "rgba(70,96,136,0.6)",
    );
  }
  ctx.filter = "none";
  ctx.fillStyle = linear(ctx, 0, horizon, 0, h, [
    [0, "#27456c"],
    [0.5, "#152a47"],
    [1, "#0c1626"],
  ]);
  ctx.fillRect(0, horizon, w, h - horizon);
  // 月光在水面上碎开
  for (let i = 0; i < 46; i++) {
    const f = r();
    const y = horizon + (h - horizon) * f;
    const spread = w * (0.03 + f * 0.2);
    ctx.strokeStyle = `rgba(240,222,170,${0.85 - f * 0.5})`;
    ctx.lineWidth = h * (0.006 + f * 0.012);
    const x = mx + (r() - 0.5) * spread * 2;
    ctx.beginPath();
    ctx.moveTo(x - w * (0.01 + f * 0.04), y);
    ctx.lineTo(x + w * (0.01 + f * 0.04), y);
    ctx.stroke();
  }
  // 岬角和灯塔
  ctx.fillStyle = "#0b1320";
  ctx.beginPath();
  ctx.moveTo(0, horizon + h * 0.03);
  ctx.bezierCurveTo(
    w * 0.12,
    horizon - h * 0.12,
    w * 0.26,
    horizon - h * 0.07,
    w * 0.4,
    horizon + h * 0.02,
  );
  ctx.lineTo(0, horizon + h * 0.1);
  ctx.closePath();
  ctx.fill();
  ctx.fillStyle = "#d8cdb6";
  ctx.fillRect(w * 0.15, horizon - h * 0.21, w * 0.022, h * 0.13);
  ellipse(ctx, w * 0.161, horizon - h * 0.215, w * 0.02, w * 0.02, "#f2c463");
}

/** 午后的花田：面试完成。 */
export function meadow(ctx, w, h, r) {
  const horizon = h * 0.46;
  ctx.fillStyle = linear(ctx, 0, 0, 0, horizon, [
    [0, "#6f9bc0"],
    [0.7, "#bcd3dc"],
    [1, "#ece3c4"],
  ]);
  ctx.fillRect(0, 0, w, horizon + 1);
  ctx.filter = `blur(${Math.round(h / 70)}px)`;
  for (let i = 0; i < 8; i++) {
    const x = r() * w;
    const y = h * (0.07 + r() * 0.24);
    ellipse(ctx, x, y, w * (0.08 + r() * 0.1), h * (0.03 + r() * 0.03), "rgba(252,247,234,0.95)");
    ellipse(ctx, x + w * 0.02, y + h * 0.025, w * 0.08, h * 0.018, "rgba(170,186,204,0.7)");
  }
  ctx.filter = "none";
  ridge(ctx, w, h, r, horizon - h * 0.03, h * 0.03, "#7f9bb0", 2.4);
  ridge(ctx, w, h, r, horizon + h * 0.02, h * 0.025, "#5f8a5c", 3.3);
  ctx.fillStyle = linear(ctx, 0, horizon + h * 0.05, 0, h, [
    [0, "#8fae5e"],
    [0.5, "#6f9648"],
    [1, "#4c7436"],
  ]);
  ctx.fillRect(0, horizon + h * 0.06, w, h);
  // 小路
  ctx.fillStyle = "#e2c98f";
  ctx.beginPath();
  ctx.moveTo(w * 0.47, horizon + h * 0.07);
  ctx.bezierCurveTo(w * 0.56, h * 0.66, w * 0.3, h * 0.8, w * 0.36, h * 1.02);
  ctx.lineTo(w * 0.62, h * 1.02);
  ctx.bezierCurveTo(w * 0.5, h * 0.8, w * 0.63, h * 0.66, w * 0.5, horizon + h * 0.07);
  ctx.closePath();
  ctx.fill();
  // 树
  ellipse(ctx, w * 0.82, horizon - h * 0.04, w * 0.09, h * 0.13, "#3d6a3f");
  ellipse(ctx, w * 0.79, horizon - h * 0.07, w * 0.05, h * 0.07, "#6f9a4e");
  ctx.fillStyle = "#4a3526";
  ctx.fillRect(w * 0.815, horizon + h * 0.04, w * 0.012, h * 0.08);
  // 花：近大远小
  const dots = [P.vermilion, "#e25a3a", "#f0cf5a", P.white, "#d98ba0"];
  for (let i = 0; i < 260; i++) {
    const f = r() ** 0.7;
    const y = horizon + h * 0.09 + (h - horizon - h * 0.09) * f;
    const x = r() * w;
    const rad = w * (0.003 + f * 0.012);
    ellipse(ctx, x, y, rad * (1 + r()), rad, dots[(r() * dots.length) | 0], r() * 3);
  }
}

/** 调色盘上搅开的颜料：侧栏、深色横幅的底。 */
export function palette(ctx, w, h, r) {
  ctx.fillStyle = "#1a1612";
  ctx.fillRect(0, 0, w, h);
  const colors = ["#12263f", "#1e3a5c", "#2a2019", "#3a2a1c", "#27466f", "#1b2f2b", "#5a3a22"];
  ctx.filter = `blur(${Math.round(Math.min(w, h) / 26)}px)`;
  for (let i = 0; i < 40; i++) {
    ellipse(
      ctx,
      r() * w,
      r() * h,
      w * (0.12 + r() * 0.3),
      h * (0.06 + r() * 0.14),
      colors[(r() * colors.length) | 0],
      r() * 3,
    );
  }
  // 少量亮色，像没搅匀的土黄和朱红
  for (let i = 0; i < 6; i++) {
    ellipse(
      ctx,
      r() * w,
      r() * h,
      w * (0.05 + r() * 0.1),
      h * (0.012 + r() * 0.02),
      i % 3 === 0 ? "rgba(194,69,45,0.5)" : "rgba(201,154,60,0.55)",
      r() * 3,
    );
  }
  ctx.filter = "none";
}
