// Fails if anything outside the store (src/state/) reads data from the
// backend (docs/ui-data-flow.md). Screens and action factories may send
// actions; only the store reads, so everything the window shows has one
// path and stays current through the change counters.
//
// A read is: a `get_*` / `list_*` / `*_snapshot` command, a command named in
// a variable (it cannot be checked), auto-start's `isEnabled`, or listening
// to the store's own events.

import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

const repoRoot = process.cwd();
const sourceRoot = path.join(repoRoot, "src");
const storeRoot = path.join(sourceRoot, "state") + path.sep;

const READ_COMMAND = /^(?:get_|list_)|_snapshot$/;
const STORE_EVENTS = new Set(["sync-status", "app-data-changed"]);

const invokePattern = /\binvoke\s*(?:<[^>]*>)?\s*\(\s*(["'`])?([A-Za-z0-9_]*)/g;
const listenPattern = /\blisten\s*(?:<[^>]*>)?\s*\(\s*["'`]([^"'`]+)["'`]/g;
const autostartReadPattern = /\bisEnabled\b[^;\n]*from\s+["']@tauri-apps\/plugin-autostart["']|import\s*\{[^}]*\bisEnabled\b[^}]*\}\s*from\s*["']@tauri-apps\/plugin-autostart["']/g;

async function collectSourceFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const absolute = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectSourceFiles(absolute)));
    } else if (entry.isFile() && /\.(ts|tsx)$/.test(entry.name)) {
      files.push(absolute);
    }
  }
  return files;
}

function lineOf(text, index) {
  return text.slice(0, index).split("\n").length;
}

const violations = [];
for (const file of await collectSourceFiles(sourceRoot)) {
  if (file.startsWith(storeRoot)) {
    continue;
  }
  const text = await readFile(file, "utf8");
  const relative = path.relative(repoRoot, file);

  for (const match of text.matchAll(invokePattern)) {
    const [, quote, command] = match;
    if (!quote) {
      violations.push(`${relative}:${lineOf(text, match.index)} invokes a command named in a variable`);
    } else if (READ_COMMAND.test(command)) {
      violations.push(`${relative}:${lineOf(text, match.index)} reads "${command}"`);
    }
  }
  for (const match of text.matchAll(listenPattern)) {
    if (STORE_EVENTS.has(match[1])) {
      violations.push(`${relative}:${lineOf(text, match.index)} listens to the store's "${match[1]}" event`);
    }
  }
  for (const match of text.matchAll(autostartReadPattern)) {
    violations.push(`${relative}:${lineOf(text, match.index)} reads the auto-start state`);
  }
}

if (violations.length > 0) {
  console.error("UI data read guard failed. Only the store (src/state/) may read from the backend:");
  for (const violation of violations) {
    console.error(`  ${violation}`);
  }
  console.error("Read through the store instead (docs/ui-data-flow.md).");
  process.exit(1);
}

console.log("UI data read guard passed.");
