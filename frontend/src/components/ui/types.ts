export type Preview = "none" | "hover" | "active" | "focus";

export function focusPreview(preview?: Preview): string {
  return preview === "focus" ? "ring-2 ring-ring ring-offset-2" : "";
}
