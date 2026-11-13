# @ai-interview/ui：油画风格设计系统

HR 后台（`web/apps/admin`）和候选人面试页（`web/apps/candidate`）共用的设计令牌、质感和组件。

亚麻画布做底，生褐做墨，普鲁士蓝是主色，朱红和土黄点睛。插画、笔触和画布纹理都是程序画出来的静态图片，页面上不跑滤镜，不占面试时的 CPU。

## 怎么用

```ts
// 入口文件里引一次样式
import "@ai-interview/ui/styles.css";
import { Button, Card, Field, Input } from "@ai-interview/ui";
```

在使用方的 `package.json` 里加 `"@ai-interview/ui": "workspace:*"`。这个包直接导出源码，不用先构建，Vite 会一起编译。

- 所有样式都在 `@layer oil` 里，使用方不加层的样式天然能覆盖它。
- 类名都以 `oil-` 开头，可以不用 React 组件直接写类名。
- 颜色、字号、间距、圆角都是 CSS 变量（`--oil-*`），定义在 `src/styles/tokens.css`。只想要令牌就引 `@ai-interview/ui/tokens.css`。
- 在 canvas 里画东西要取颜色，用 `token("--oil-meter-on")`。

## 主题

默认是「画布」（浅色）。`<html data-theme="dark">` 切到「夜画」（深色）。

```ts
import { applyTheme, readTheme } from "@ai-interview/ui";
applyTheme(readTheme()); // 读上次的选择，没有就跟随系统
```

面试进行中的深色舞台（`<Stage night>`）不管页面是什么主题，里面都用夜画的取值。要在浅色页面里单独做一块深色区域，给容器加 `oil-night` 类。

## 有什么

| 类别 | 组件 |
| --- | --- |
| 按钮 | `Button`（主色、朱红、金、次要、幽灵、危险；三种尺寸；加载中） |
| 表单 | `Field`、`Input`、`Textarea`、`Select`、`Checkbox`、`Radio`、`Switch`、`Slider`、`Segmented`、`Dropzone`、`CopyField` |
| 容器 | `Card`、`Frame`（镀金画框）、`Painting`、`Hero`、`Dialog` |
| 数据 | `Table`、`Badge`、`Chip`、`Stat`、`Avatar`、`KeyValue`、`ScoreBar`、`Ring`、`Progress` |
| 反馈 | `Banner`、`Spinner`、`Skeleton`、`EmptyState`、`ToastProvider` 和 `useToast` |
| 导航 | `AppShell`、`Brand`、`Nav`、`navItemClass`、`PageHeader`、`Tabs`、`Steps` |
| 面试页 | `Stage`、`DeviceCheckCard`、`ConsentList`、`VideoFrame`、`InterviewerPresence`、`CaptionCard`、`QuestionProgress`、`Timer`、`RecordingIndicator`、`SilenceRing`、`LevelMeter`、`StatusPage` |

每个组件长什么样、怎么写，看展示页：

```bash
pnpm --filter @ai-interview/ui dev   # http://localhost:5175
```

## 约束

- 手机优先：触控目标不小于 44px，单列也要好用。
- 字体自托管，不走外网。标题用按字切片的 Noto Serif SC 和 Playfair Display（都是 SIL OFL 1.1），正文用系统字体。
- 动效只动 `transform` 和 `opacity`，并且尊重「减少动态效果」的系统设置。
- 文字对比度达到 WCAG AA，`src/tokens.test.ts` 会检查两套主题。

## 重新生成插画

`src/assets/` 里的图是 `scripts/paint.mjs` 用无头浏览器画的：先铺色块，再沿着边缘一笔一笔铺笔触，最后按颜料厚度打侧光。种子固定，同样的输入得到同样的画。

```bash
pnpm --filter @ai-interview/ui paint            # 全部重画
pnpm --filter @ai-interview/ui paint dusk sitter # 只画这两幅
```

需要本机有 Playwright 和 Chromium。画面内容在 `scripts/paint/scenes.js`，笔触和光照在 `scripts/paint/painter.js`。

## 检查

```bash
pnpm --filter @ai-interview/ui typecheck
pnpm --filter @ai-interview/ui test
```
