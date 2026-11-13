#!/usr/bin/env node
// 生成设计系统用到的油画插画、笔触蒙版和质感贴图，写到 src/assets/。
//
// 用法（在 web/packages/ui 下）：
//   node scripts/paint.mjs            # 全部重画
//   node scripts/paint.mjs dusk sitter # 只画点名的几张
//
// 需要一个 Chromium 和 Playwright。云端容器里已经预装，直接跑即可；
// 别的机器上先 `npx playwright install chromium`，再用 NODE_PATH 指到装了 playwright 的 node_modules。
// 生成结果是确定的（固定随机种子），产物提交进仓库，平时构建不需要跑这个脚本。

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "..", "src", "assets");

function loadPlaywright() {
  const require = createRequire(import.meta.url);
  for (const base of [undefined, "/opt/node-tools/node_modules"]) {
    try {
      return require(base ? join(base, "playwright") : "playwright");
    } catch {
      // 换下一个位置找
    }
  }
  throw new Error("找不到 playwright。先安装它，或用 NODE_PATH 指到装了它的 node_modules。");
}

/** @type {Array<{name:string, file:string, kind:string, args:Record<string, unknown>, quality?:number}>} */
const JOBS = [
  // 插画
  {
    name: "dusk",
    file: "paintings/dusk.webp",
    kind: "paint",
    args: { scene: "dusk", width: 1800, height: 720, seed: 11, swirl: 0.5 },
    quality: 0.8,
  },
  {
    name: "sitter",
    file: "paintings/sitter.webp",
    kind: "paint",
    args: { scene: "sitter", width: 900, height: 1080, seed: 23, swirl: 0.9, baseAngle: 0.9 },
    quality: 0.82,
  },
  {
    name: "bouquet",
    file: "paintings/bouquet.webp",
    kind: "paint",
    args: { scene: "bouquet", width: 900, height: 900, seed: 5, swirl: 1.4 },
    quality: 0.8,
  },
  {
    name: "nocturne",
    file: "paintings/nocturne.webp",
    kind: "paint",
    args: { scene: "nocturne", width: 1200, height: 900, seed: 31, swirl: 0.4 },
    quality: 0.8,
  },
  {
    name: "meadow",
    file: "paintings/meadow.webp",
    kind: "paint",
    args: { scene: "meadow", width: 1200, height: 900, seed: 47, swirl: 0.6 },
    quality: 0.8,
  },
  {
    name: "palette",
    file: "paintings/palette.webp",
    kind: "paint",
    args: {
      scene: "palette",
      width: 720,
      height: 1440,
      seed: 3,
      swirl: 2.2,
      flowScale: 0.5,
      brushes: [0.07, 0.035, 0.018],
    },
    quality: 0.72,
  },
  // 笔触蒙版（白色带透明度）
  {
    name: "stroke",
    file: "brush/stroke.webp",
    kind: "mask",
    args: { kind: "stroke", width: 640, height: 120, seed: 9 },
    quality: 0.9,
  },
  {
    name: "stroke-b",
    file: "brush/stroke-b.webp",
    kind: "mask",
    args: { kind: "stroke", width: 640, height: 120, seed: 21 },
    quality: 0.9,
  },
  {
    name: "underline",
    file: "brush/underline.webp",
    kind: "mask",
    args: { kind: "underline", width: 640, height: 48, seed: 14 },
    quality: 0.9,
  },
  {
    name: "dab",
    file: "brush/dab.webp",
    kind: "mask",
    args: { kind: "dab", width: 240, height: 240, seed: 17 },
    quality: 0.9,
  },
  // 平铺质感
  {
    name: "weave",
    file: "texture/weave.webp",
    kind: "tile",
    args: { kind: "weave", size: 128, seed: 2 },
    quality: 0.8,
  },
  {
    name: "impasto",
    file: "texture/impasto.webp",
    kind: "tile",
    args: { kind: "impasto", size: 256, seed: 6 },
    quality: 0.8,
  },
];

const only = process.argv.slice(2);
const jobs = only.length ? JOBS.filter((j) => only.includes(j.name)) : JOBS;
if (!jobs.length) {
  console.error(`没有这些名字。可选：${JOBS.map((j) => j.name).join(" ")}`);
  process.exit(1);
}

const { chromium } = loadPlaywright();
const browser = await chromium.launch();
const page = await browser.newPage();
page.on("console", (m) => console.log(`[page] ${m.text()}`));
page.on("pageerror", (e) => console.error(`[page error] ${e.message}`));

// 把两个模块内联进页面：不起本地服务器，也不依赖 file:// 的模块加载
const strip = (src) => src.replace(/^export /gm, "");
const painter = strip(await readFile(join(here, "paint", "painter.js"), "utf8"));
const scenes = strip(await readFile(join(here, "paint", "scenes.js"), "utf8"));
await page.setContent("<!doctype html><html><body></body></html>");
await page.addScriptTag({
  content: `${painter}\n${scenes}\nwindow.__scenes = { dusk, sitter, bouquet, nocturne, meadow, palette };
window.__run = (kind, args, quality) => {
  let canvas;
  if (kind === "paint") canvas = paint({ ...args, draw: window.__scenes[args.scene] });
  else if (kind === "mask") canvas = brushMask(args);
  else canvas = textureTile(args);
  return canvas.toDataURL("image/webp", quality);
};`,
});

for (const job of jobs) {
  const started = Date.now();
  const dataUrl = await page.evaluate(
    ([kind, args, quality]) => window.__run(kind, args, quality),
    [job.kind, job.args, job.quality ?? 0.8],
  );
  const bytes = Buffer.from(dataUrl.slice(dataUrl.indexOf(",") + 1), "base64");
  const target = join(outDir, job.file);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, bytes);
  console.log(
    `${job.file}  ${(bytes.length / 1024).toFixed(0)} KB  ${((Date.now() - started) / 1000).toFixed(1)}s`,
  );
}

await browser.close();
