// 内置的油画插画。都是 scripts/paint.mjs 程序化生成的，不含真人肖像。
import bouquet from "./assets/paintings/bouquet.webp";
import dusk from "./assets/paintings/dusk.webp";
import meadow from "./assets/paintings/meadow.webp";
import nocturne from "./assets/paintings/nocturne.webp";
import palette from "./assets/paintings/palette.webp";
import sitter from "./assets/paintings/sitter.webp";

export const paintings = {
  /** 暮色里的原野：首页横幅 */
  dusk,
  /** 抽象的半身像：面试官的形象 */
  sitter,
  /** 瓶花：空状态、感谢页 */
  bouquet,
  /** 月夜的海：链接已过期、出了问题 */
  nocturne,
  /** 午后的花田：面试完成 */
  meadow,
  /** 调色盘上搅开的颜料：深色底纹 */
  palette,
} as const;

export type PaintingName = keyof typeof paintings;
