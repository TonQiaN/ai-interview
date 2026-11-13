// 程序化「油画」渲染器，在浏览器（无头 Chromium）里跑，由 ../paint.mjs 驱动。
//
// 做法是经典的 painterly rendering：先把场景画成一张色块参考图，再用由大到小的几遍笔刷
// 沿着参考图的边缘方向铺笔触；每一笔由一束带明暗抖动的「鬃毛」线组成。同时在另一张画布上
// 记录颜料厚度，最后按厚度做一次侧光照明，得到厚涂（impasto）的凹凸高光。
//
// 输出是静态图片，页面上不跑任何滤镜或动画，所以面试录制时不占 CPU。

export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** 周期性的二维值噪声，返回 [0, 1)。period 是晶格边长，坐标按它取模，所以可以无缝平铺。 */
export function valueNoise(seed, period = 64) {
  const r = rng(seed);
  const grid = new Float32Array(period * period);
  for (let i = 0; i < grid.length; i++) grid[i] = r();
  const at = (x, y) =>
    grid[(((y % period) + period) % period) * period + (((x % period) + period) % period)];
  return (x, y) => {
    const xi = Math.floor(x);
    const yi = Math.floor(y);
    const fx = x - xi;
    const fy = y - yi;
    const sx = fx * fx * (3 - 2 * fx);
    const sy = fy * fy * (3 - 2 * fy);
    const a = at(xi, yi);
    const b = at(xi + 1, yi);
    const c = at(xi, yi + 1);
    const d = at(xi + 1, yi + 1);
    return a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy;
  };
}

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

function makeCanvas(w, h) {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  return c;
}

/**
 * 把一个场景画成油画。
 * @param {object} o
 * @param {number} o.width
 * @param {number} o.height
 * @param {number} o.seed
 * @param {(ctx: CanvasRenderingContext2D, w: number, h: number, r: () => number) => void} o.draw 画参考图
 * @param {number[]} [o.brushes] 各遍笔刷半径，占短边的比例，由大到小
 * @param {number} [o.relief] 厚涂凹凸强度
 * @param {number} [o.flowScale] 流场噪声的尺度，越大笔触方向变化越缓
 * @param {number} [o.swirl] 流场转动的圈数
 */
export function paint(o) {
  const { width: w, height: h, seed, draw } = o;
  const short = Math.min(w, h);
  const brushes = (o.brushes ?? [0.05, 0.026, 0.013, 0.0065]).map((b) => Math.max(2, b * short));
  const relief = o.relief ?? 1;
  const r = rng(seed);

  // 1. 参考图
  const ref = makeCanvas(w, h);
  const rc = ref.getContext("2d", { willReadFrequently: true });
  draw(rc, w, h, rng(seed + 101));
  const refPx = rc.getImageData(0, 0, w, h).data;

  // 2. 亮度（先模糊，梯度才稳定）
  const blur = makeCanvas(w, h);
  const bc = blur.getContext("2d", { willReadFrequently: true });
  bc.filter = `blur(${Math.max(1, Math.round(short / 160))}px)`;
  bc.drawImage(ref, 0, 0);
  bc.filter = "none";
  const blurPx = bc.getImageData(0, 0, w, h).data;
  const lum = new Float32Array(w * h);
  for (let i = 0; i < w * h; i++) {
    lum[i] = 0.299 * blurPx[i * 4] + 0.587 * blurPx[i * 4 + 1] + 0.114 * blurPx[i * 4 + 2];
  }
  const lumAt = (x, y) => lum[clamp(y | 0, 0, h - 1) * w + clamp(x | 0, 0, w - 1)];

  const flow = valueNoise(seed + 7, 64);
  const flowScale = (o.flowScale ?? 0.35) * short;
  const swirl = o.swirl ?? 1.2;
  const baseAngle = o.baseAngle ?? 0;

  /** 笔触方向：平坦处跟流场走，边缘处顺着边缘走。 */
  const dirAt = (x, y, d) => {
    const na = baseAngle + (flow(x / flowScale, y / flowScale) - 0.5) * Math.PI * 2 * swirl;
    let vx = Math.cos(na);
    let vy = Math.sin(na);
    const gx = lumAt(x + d, y) - lumAt(x - d, y);
    const gy = lumAt(x, y + d) - lumAt(x, y - d);
    const mag = Math.hypot(gx, gy);
    if (mag > 1.5) {
      // 边缘切线，取和流场同侧的那个方向
      let tx = -gy / mag;
      let ty = gx / mag;
      if (tx * vx + ty * vy < 0) {
        tx = -tx;
        ty = -ty;
      }
      const k = clamp(mag / 14, 0, 4);
      vx += tx * k;
      vy += ty * k;
      const n = Math.hypot(vx, vy) || 1;
      vx /= n;
      vy /= n;
    }
    return [vx, vy, mag];
  };

  // 3. 颜色画布和厚度画布
  const col = makeCanvas(w, h);
  const cc = col.getContext("2d");
  cc.filter = `blur(${Math.round(short / 30)}px)`;
  cc.drawImage(ref, 0, 0);
  cc.filter = "none";
  const ht = makeCanvas(w, h);
  const hc = ht.getContext("2d", { willReadFrequently: true });
  hc.fillStyle = "rgb(96,96,96)";
  hc.fillRect(0, 0, w, h);
  cc.lineCap = "round";
  cc.lineJoin = "round";
  hc.lineCap = "round";
  hc.lineJoin = "round";

  brushes.forEach((radius, pass) => {
    const spacing = radius * (pass === 0 ? 0.75 : 0.9);
    const cols = Math.ceil(w / spacing) + 2;
    const rows = Math.ceil(h / spacing) + 2;
    const order = new Uint32Array(cols * rows);
    for (let i = 0; i < order.length; i++) order[i] = i;
    for (let i = order.length - 1; i > 0; i--) {
      const j = (r() * (i + 1)) | 0;
      const t = order[i];
      order[i] = order[j];
      order[j] = t;
    }
    const stepLen = Math.max(2, radius * 0.85);
    const maxSteps = pass === 0 ? 10 : pass === 1 ? 10 : 8;
    const tol = pass === 0 ? 110 : pass === 1 ? 70 : 46;
    const bristles = Math.max(3, Math.round(radius * 0.9));
    const lineW = Math.max(1, ((radius * 2) / bristles) * 1.7);

    for (const idx of order) {
      const gx = idx % cols;
      const gy = (idx / cols) | 0;
      let x = (gx - 0.5 + (r() - 0.5) * 0.9) * spacing;
      let y = (gy - 0.5 + (r() - 0.5) * 0.9) * spacing;
      const sx = clamp(x | 0, 0, w - 1);
      const sy = clamp(y | 0, 0, h - 1);

      // 后面的小笔刷只在有细节的地方落笔，平坦处保留大笔触
      let [vx, vy, mag] = dirAt(x, y, Math.max(2, radius * 0.5));
      if (pass >= 2) {
        const keep = pass === 2 ? 0.1 + mag / 9 : 0.015 + mag / 14;
        if (r() > keep) continue;
      }

      // 取色：偶尔从旁边蘸一点别的颜色（脏画笔）
      let px = sx;
      let py = sy;
      if (r() < 0.12) {
        px = clamp((x + (r() - 0.5) * radius * 3) | 0, 0, w - 1);
        py = clamp((y + (r() - 0.5) * radius * 3) | 0, 0, h - 1);
      }
      const pi = (py * w + px) * 4;
      const jitter = 1 + (r() - 0.5) * 0.14;
      const temp = (r() - 0.5) * 14;
      const cr = refPx[pi] * jitter + temp;
      const cg = refPx[pi + 1] * jitter;
      const cb = refPx[pi + 2] * jitter - temp;
      const br0 = refPx[(sy * w + sx) * 4];
      const bg0 = refPx[(sy * w + sx) * 4 + 1];
      const bb0 = refPx[(sy * w + sx) * 4 + 2];

      // 沿流场走出一条弯曲的笔触
      const pts = [[x, y, -vy, vx]];
      const steps = 4 + ((r() * (maxSteps - 3)) | 0);
      for (let s = 0; s < steps; s++) {
        x += vx * stepLen;
        y += vy * stepLen;
        if (x < -radius || y < -radius || x > w + radius || y > h + radius) break;
        const qi = (clamp(y | 0, 0, h - 1) * w + clamp(x | 0, 0, w - 1)) * 4;
        const dist =
          Math.abs(refPx[qi] - br0) + Math.abs(refPx[qi + 1] - bg0) + Math.abs(refPx[qi + 2] - bb0);
        if (dist > tol) break;
        const [nx, ny] = dirAt(x, y, Math.max(2, radius * 0.5));
        // 保持方向连续，并做一点惯性平滑
        const dot = nx * vx + ny * vy;
        const sgn = dot < 0 ? -1 : 1;
        vx = vx * 0.55 + nx * sgn * 0.45;
        vy = vy * 0.55 + ny * sgn * 0.45;
        const n = Math.hypot(vx, vy) || 1;
        vx /= n;
        vy /= n;
        pts.push([x, y, -vy, vx]);
      }
      if (pts.length < 2) continue;

      const strokeHeight = 96 + r() * 64;
      const taper = pts.length - 1;
      for (let b = 0; b < bristles; b++) {
        const u = bristles === 1 ? 0 : (b / (bristles - 1)) * 2 - 1; // -1..1
        const off = u * radius * (0.92 + r() * 0.16);
        // 干笔：有的鬃毛起笔晚、收笔早
        const start = r() < 0.3 ? 1 : 0;
        const end = taper - (r() < 0.45 ? 1 : 0);
        if (end - start < 1) continue;
        const l = 1 + (r() - 0.5) * 0.16;
        const a = 0.7 + r() * 0.3;
        cc.strokeStyle = `rgba(${clamp(cr * l, 0, 255) | 0},${clamp(cg * l, 0, 255) | 0},${clamp(cb * l, 0, 255) | 0},${a})`;
        cc.lineWidth = lineW * (0.7 + r() * 0.6);
        cc.beginPath();
        hc.beginPath();
        for (let k = start; k <= end; k++) {
          const p = pts[k];
          // 笔触两头略收拢
          const squeeze = k === 0 || k === taper ? 0.78 : 1;
          const qx = p[0] + p[2] * off * squeeze;
          const qy = p[1] + p[3] * off * squeeze;
          if (k === start) {
            cc.moveTo(qx, qy);
            hc.moveTo(qx, qy);
          } else {
            cc.lineTo(qx, qy);
            hc.lineTo(qx, qy);
          }
        }
        cc.stroke();
        // 厚度：鬃毛之间有沟，笔触边缘颜料堆得高
        const ridge = Math.abs(u) > 0.8 ? 16 : 0;
        const hv = clamp(strokeHeight + (r() - 0.5) * 34 + ridge, 0, 255) | 0;
        hc.strokeStyle = `rgb(${hv},${hv},${hv})`;
        hc.lineWidth = cc.lineWidth;
        hc.stroke();
      }
    }
  });

  // 4. 侧光照明
  const soft = makeCanvas(w, h);
  const sc = soft.getContext("2d", { willReadFrequently: true });
  sc.filter = "blur(1.1px)";
  sc.drawImage(ht, 0, 0);
  const hp = sc.getImageData(0, 0, w, h).data;
  const H = new Float32Array(w * h);
  const weave = o.weave ?? 1.6;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      // 画布的经纬纹理从薄处透出来
      const wv = (Math.sin((x * Math.PI * 2) / 3.2) + Math.sin((y * Math.PI * 2) / 3.2)) * weave;
      H[y * w + x] = hp[(y * w + x) * 4] + wv;
    }
  }
  const out = cc.getImageData(0, 0, w, h);
  const d = out.data;
  const k = 0.0075 * relief;
  for (let y = 0; y < h; y++) {
    const y0 = y > 0 ? y - 1 : y;
    const y1 = y < h - 1 ? y + 1 : y;
    for (let x = 0; x < w; x++) {
      const x0 = x > 0 ? x - 1 : x;
      const x1 = x < w - 1 ? x + 1 : x;
      const dx = H[y * w + x1] - H[y * w + x0];
      const dy = H[y1 * w + x] - H[y0 * w + x];
      // 光从左上来：朝左上的坡面亮，朝右下的坡面暗
      const shade = clamp((dx * 0.62 + dy * 0.78) * k, -0.22, 0.26);
      const spec = shade > 0.17 ? (shade - 0.17) * 240 : 0;
      const i = (y * w + x) * 4;
      d[i] = clamp(d[i] * (1 + shade) + spec, 0, 255);
      d[i + 1] = clamp(d[i + 1] * (1 + shade) + spec * 0.96, 0, 255);
      d[i + 2] = clamp(d[i + 2] * (1 + shade) + spec * 0.86, 0, 255);
    }
  }
  cc.putImageData(out, 0, 0);
  return col;
}

/**
 * 单独一笔，白色带透明度，用作 CSS mask：颜色由页面的 CSS 变量决定。
 * @param {{width:number,height:number,seed:number,kind:"stroke"|"dab"|"underline"}} o
 */
export function brushMask(o) {
  const { width: w, height: h, seed, kind } = o;
  const r = rng(seed);
  const c = makeCanvas(w, h);
  const ctx = c.getContext("2d");
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  const wob = valueNoise(seed + 3, 32);

  if (kind === "dab") {
    // 一团颜料：几圈鬃毛转出来的圆
    const cx = w / 2;
    const cy = h / 2;
    const R = Math.min(w, h) * 0.44;
    const n = 220;
    // 先铺一层实心的底，中间不留洞
    ctx.fillStyle = "rgba(255,255,255,0.92)";
    ctx.beginPath();
    ctx.ellipse(cx, cy, R * 0.8, R * 0.77, 0, 0, Math.PI * 2);
    ctx.fill();
    for (let b = 0; b < n; b++) {
      const rad = R * Math.sqrt(b / n) * (0.96 + r() * 0.08);
      const a0 = r() * Math.PI * 2;
      const sweep = Math.PI * (0.9 + r() * 1.4);
      ctx.strokeStyle = `rgba(255,255,255,${0.5 + r() * 0.5})`;
      ctx.lineWidth = 2 + r() * 4;
      ctx.beginPath();
      for (let t = 0; t <= 24; t++) {
        const a = a0 + (sweep * t) / 24;
        const rr = rad * (1 + (wob(Math.cos(a) * 2 + 8, Math.sin(a) * 2 + 8) - 0.5) * 0.22);
        const x = cx + Math.cos(a) * rr;
        const y = cy + Math.sin(a) * rr * 0.96;
        if (t === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
      ctx.stroke();
    }
    return c;
  }

  const thick = kind === "underline" ? h * 0.3 : h * 0.4;
  const n = kind === "underline" ? 26 : Math.round(h * 0.75);
  const padX = h * (kind === "underline" ? 0.5 : 0.45);
  for (let b = 0; b < n; b++) {
    const u = (b / (n - 1)) * 2 - 1;
    // 起笔处齐，收笔处参差（颜料越拖越少）
    const x0 = padX * (0.5 + r() * 0.5) + Math.abs(u) * h * 0.12;
    const x1 = w - padX * (0.4 + r() * 1.5) - Math.abs(u) * h * 0.25;
    const a = (0.55 + r() * 0.45) * (1 - Math.abs(u) ** 6 * 0.5);
    ctx.lineWidth = Math.max(1, ((thick * 2) / n) * (1.7 + r() * 1.1));
    const seg = 40;
    let drawing = false;
    ctx.beginPath();
    for (let t = 0; t <= seg; t++) {
      const f = t / seg;
      const x = x0 + (x1 - x0) * f;
      const bend = (wob(f * 3, 1.5) - 0.5) * h * 0.16 + Math.sin(f * Math.PI) * -h * 0.03;
      const y = h / 2 + u * thick * (0.86 + 0.14 * Math.sin(f * Math.PI)) + bend;
      // 飞白：靠后的位置鬃毛会断
      const gap = f > 0.55 && wob(f * 26 + b * 7.3, b * 1.7) < (f - 0.55) * 0.9;
      if (gap) {
        if (drawing) {
          ctx.strokeStyle = `rgba(255,255,255,${a})`;
          ctx.stroke();
          ctx.beginPath();
          drawing = false;
        }
        continue;
      }
      if (!drawing) {
        ctx.moveTo(x, y);
        drawing = true;
      } else ctx.lineTo(x, y);
    }
    ctx.strokeStyle = `rgba(255,255,255,${a})`;
    ctx.stroke();
  }
  return c;
}

/**
 * 可无缝平铺的质感贴图：黑白带透明度，叠在任何底色上。
 * kind = "weave"：亚麻画布的经纬；"impasto"：一层薄薄的笔触起伏。
 */
export function textureTile(o) {
  const { size, seed, kind } = o;
  const c = makeCanvas(size, size);
  const ctx = c.getContext("2d");
  const img = ctx.createImageData(size, size);
  const d = img.data;
  const n1 = valueNoise(seed, 64);
  const n2 = valueNoise(seed + 1, 32);
  const n3 = valueNoise(seed + 2, 16);
  const H = new Float32Array(size * size);
  if (kind === "weave") {
    const period = 4; // 线的粗细，size 要能被它整除
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        const warp = Math.sin((x * Math.PI * 2) / period);
        const weft = Math.sin((y * Math.PI * 2) / period);
        // 平纹：经纬交替压在上面
        const over = ((x / period) | 0) % 2 === ((y / period) | 0) % 2 ? warp : weft;
        const slub =
          (n1((x / size) * 64, (y / size) * 64 * 6) - 0.5) * 1.4 +
          (n2((x / size) * 32 * 5, (y / size) * 32) - 0.5) * 1.2;
        H[y * size + x] = over * 0.9 + slub + (n3((x / size) * 16, (y / size) * 16) - 0.5) * 1.5;
      }
    }
  } else {
    // 横向拉长的噪声，像刷子扫过的底色
    for (let y = 0; y < size; y++) {
      for (let x = 0; x < size; x++) {
        const u = (x / size) * 64;
        const v = (y / size) * 64;
        H[y * size + x] =
          (n1(u * 0.5, v * 4) - 0.5) * 3 +
          (n2(u * 0.25, v * 2) - 0.5) * 2.4 +
          (n3(u * 0.25, v * 0.5) - 0.5) * 2;
      }
    }
  }
  const at = (x, y) => H[((y + size) % size) * size + ((x + size) % size)];
  const gain = kind === "weave" ? 26 : 30;
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const s = (at(x + 1, y) - at(x - 1, y)) * 0.62 + (at(x, y + 1) - at(x, y - 1)) * 0.78;
      const v = clamp(s * gain, -70, 70);
      const i = (y * size + x) * 4;
      const white = v > 0;
      d[i] = d[i + 1] = d[i + 2] = white ? 255 : 0;
      d[i + 3] = Math.abs(v);
    }
  }
  ctx.putImageData(img, 0, 0);
  return c;
}
