// Consumes pnpm licenses list --json from the installed workspace.
import { json } from "node:stream/consumers";

const allowed = new Set([
  "MIT", "ISC", "BSD-3-Clause", "Apache-2.0", "MPL-2.0",
  "MIT OR Apache-2.0", "Apache-2.0 OR MIT",
]);
const groups = Object.entries(await json(process.stdin));
if (!groups.length) throw new Error("No package licenses found");
for (const [license, packages] of groups) {
  console.log(`${license}: ${packages.map(pkg => pkg.name).join(", ")}`);
  if (!allowed.has(license)) {
    console.error(`Dependency license needs review: ${license}`);
    process.exitCode = 1;
  }
}
