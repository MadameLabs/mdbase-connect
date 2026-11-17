// Adapter of mdbase-tasks for the tasknotes-spec conformance suite
// (ADAPTER_CONTRACT.md, JavaScript binding). Each operation runs the binary's
// hidden `mdbase tasks exec <operation>`: the input object goes in on stdin, one
// envelope comes out. The binary is MDBASE_BIN, or runtime/target/debug/mdbase.
//
// Use: TASKNOTES_ADAPTER=<this file> npm run conformance:test (in the spec clone).

import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const binary = process.env.MDBASE_BIN || resolve(here, "../../../target/debug/mdbase");

function run(operation, input) {
  const child = spawnSync(binary, ["tasks", "exec", operation], {
    input: JSON.stringify(input ?? {}),
    encoding: "utf8",
  });
  if (child.error) {
    return { ok: false, error: `cannot run ${binary}: ${child.error.message}` };
  }
  try {
    return JSON.parse(child.stdout);
  } catch {
    return {
      ok: false,
      error: `no envelope from ${binary} (exit ${child.status}): ${child.stderr.trim()}`,
    };
  }
}

// The suite reads `metadata` as an object at load time, so it is the claim the
// binary reports, taken once.
const claim = run("meta.claim", {});
if (!claim.ok) {
  throw new Error(`mdbase-tasks adapter: meta.claim failed: ${claim.error}`);
}
export const metadata = claim.result;

export async function execute(operation, input) {
  return run(operation, input);
}
