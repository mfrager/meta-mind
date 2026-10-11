// Radius law (design/planning/ui/02 §5.3). rounded-xl/2xl/3xl/4xl/full are banned.
// Pills use rounded-pill. Files that legitimately need rounded-full (avatars,
// toggle knobs, progress dots) are listed in .radius-allowlist.json.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const allow = JSON.parse(readFileSync(join(root, ".radius-allowlist.json"), "utf8"));
const allowFiles = new Set(allow.files ?? []);
const banned = /rounded-(?:xl|2xl|3xl|4xl|full)\b/g;

function walk(dir, out = []) {
  for (const name of readdirSync(dir)) {
    if (name === "node_modules" || name === "dist") continue;
    const path = join(dir, name);
    if (statSync(path).isDirectory()) walk(path, out);
    else if (/\.(tsx?|css)$/.test(name)) out.push(path);
  }
  return out;
}

const violations = [];
for (const path of walk(join(root, "src"))) {
  const rel = relative(root, path);
  if (allowFiles.has(rel)) continue;
  const text = readFileSync(path, "utf8");
  const lines = text.split("\n");
  lines.forEach((line, index) => {
    if (banned.test(line)) {
      violations.push(`${rel}:${index + 1}: ${line.trim()}`);
    }
    banned.lastIndex = 0;
  });
}

if (violations.length > 0) {
  console.error("radius law violations (design/planning/ui/02 §5):\n");
  for (const line of violations) console.error(`  ${line}`);
  process.exit(1);
}
