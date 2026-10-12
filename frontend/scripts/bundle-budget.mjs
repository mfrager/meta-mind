import { gzipSync } from "node:zlib";
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const dist = fileURLToPath(new URL("../dist/assets/", import.meta.url));
const files = readdirSync(dist).filter((name) => name.endsWith(".js"));
const rows = files.map((name) => {
  const bytes = readFileSync(join(dist, name));
  return { name, raw: bytes.length, gzip: gzipSync(bytes).length };
});
rows.sort((a, b) => b.gzip - a.gzip);
const entry = rows.find((row) => row.name.startsWith("index-"));
if (!entry) {
  console.error("bundle budget: no index-*.js in dist/assets");
  process.exit(1);
}
const limit = 250 * 1024;
const report = {
  limitGzip: limit,
  entry,
  assets: rows,
};
const out = fileURLToPath(new URL("../dist/bundle-report.json", import.meta.url));
writeFileSync(out, `${JSON.stringify(report, null, 2)}\n`);
const kib = (n) => `${(n / 1024).toFixed(1)} KiB`;
console.log(`initial JS ${entry.name}: ${kib(entry.gzip)} gzip (limit ${kib(limit)})`);
for (const row of rows.slice(0, 8)) {
  console.log(`  ${row.name}  ${kib(row.gzip)} gzip  ${kib(row.raw)} raw`);
}
if (entry.gzip > limit) {
  console.error(`bundle budget exceeded: ${entry.gzip} > ${limit}`);
  process.exit(1);
}
