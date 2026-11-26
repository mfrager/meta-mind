/** WCAG contrast for the oklch tokens in `index.css`. */

export type Oklch = { l: number; c: number; h: number };

export function parseOklch(value: string): Oklch | null {
  const match = value.trim().match(/oklch\(\s*([0-9.]+)\s+([0-9.]+)\s+([0-9.]+)/);
  if (!match) return null;
  return { l: Number(match[1]), c: Number(match[2]), h: Number(match[3]) };
}

/** Linear sRGB from OKLCH (Björn Ottosson), clipped into gamut. */
export function linearSrgb(color: Oklch): [number, number, number] {
  const h = (color.h * Math.PI) / 180;
  const a = color.c * Math.cos(h);
  const b = color.c * Math.sin(h);
  const l_ = color.l + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = color.l - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = color.l - 0.0894841775 * a - 1.291485548 * b;
  const l = l_ ** 3;
  const m = m_ ** 3;
  const s = s_ ** 3;
  const rgb: [number, number, number] = [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
  return rgb.map((channel) => Math.min(1, Math.max(0, channel))) as [number, number, number];
}

export function relativeLuminance(color: Oklch): number {
  const [r, g, b] = linearSrgb(color);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrastRatio(foreground: Oklch, background: Oklch): number {
  const a = relativeLuminance(foreground);
  const b = relativeLuminance(background);
  const lighter = Math.max(a, b);
  const darker = Math.min(a, b);
  return (lighter + 0.05) / (darker + 0.05);
}
