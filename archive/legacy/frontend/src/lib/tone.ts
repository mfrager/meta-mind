/** Semantic hues from the token gallery. Class strings are complete so Tailwind emits them. */
export const TONES = [
  "azure",
  "teal",
  "violet",
  "amber",
  "red",
  "green",
  "cyan",
  "indigo",
  "rose",
  "slate",
] as const;

export type Tone = (typeof TONES)[number];

export const TONE_CHIP: Record<Tone, string> = {
  azure: "bg-azure-100 text-azure-800 dark:bg-azure-900 dark:text-azure-200",
  teal: "bg-teal-100 text-teal-800 dark:bg-teal-900 dark:text-teal-200",
  violet: "bg-violet-100 text-violet-800 dark:bg-violet-900 dark:text-violet-200",
  amber: "bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200",
  red: "bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200",
  green: "bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200",
  cyan: "bg-cyan-100 text-cyan-800 dark:bg-cyan-900 dark:text-cyan-200",
  indigo: "bg-indigo-100 text-indigo-800 dark:bg-indigo-900 dark:text-indigo-200",
  rose: "bg-rose-100 text-rose-800 dark:bg-rose-900 dark:text-rose-200",
  slate: "bg-slate-100 text-slate-800 dark:bg-slate-900 dark:text-slate-200",
};

export const TONE_EDGE: Record<Tone, string> = {
  azure: "border-l-azure-500",
  teal: "border-l-teal-500",
  violet: "border-l-violet-500",
  amber: "border-l-amber-500",
  red: "border-l-red-500",
  green: "border-l-green-500",
  cyan: "border-l-cyan-500",
  indigo: "border-l-indigo-500",
  rose: "border-l-rose-500",
  slate: "border-l-slate-500",
};
