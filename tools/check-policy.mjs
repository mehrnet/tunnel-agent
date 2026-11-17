#!/usr/bin/env node
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const failures = [];
const rules = [
  { directory: "tunnel-agent/src", extensions: [".rs"], maxLines: 500 },
  { directory: "tunnel-api/src", extensions: [".ts"], maxLines: 300 },
  { directory: "tunnel-api", extensions: [".mjs"], maxLines: 300 },
  { directory: "tunnel-api/web", extensions: [".ts", ".tsx", ".css", ".html"], maxLines: 300 },
  { directory: "tools", extensions: [".mjs"], maxLines: 300 },
  { directory: "tools", extensions: [".sh"], maxLines: 350 },
  { directory: "tunnel-files/static", extensions: [".sh"], maxLines: 350 },
];
const shellFunctionLimit = 100;
const shellAllowances = new Map([["tunnel-files/static/install.sh", 450]]);
const excludedDirectories = new Set([".git", "dist", "node_modules", "web-dist"]);

function filesIn(directory) {
  return readdirSync(join(root, directory), { withFileTypes: true }).flatMap((entry) => {
    const relative = join(directory, entry.name);
    if (entry.isDirectory()) return excludedDirectories.has(entry.name) ? [] : filesIn(relative);
    return entry.isFile() ? [relative] : [];
  });
}

function lineCount(path) {
  return readFileSync(join(root, path), "utf8").split("\n").length - 1;
}

function braceDelta(line) {
  return [...line].reduce((depth, character) => depth + (character === "{") - (character === "}"), 0);
}

function checkShellFunctions(path) {
  const lines = readFileSync(join(root, path), "utf8").split("\n");
  let functionName;
  let start;
  let depth = 0;
  for (const [index, line] of lines.entries()) {
    if (!functionName && /^[_a-zA-Z][_a-zA-Z0-9]*\(\)\s*\{/.test(line)) {
      [functionName] = line.split("(");
      start = index;
      depth = 0;
    }
    if (!functionName) continue;
    depth += braceDelta(line);
    if (depth !== 0) continue;
    const size = index - start + 1;
    if (size > shellFunctionLimit) {
      failures.push(`${path}:${start + 1}: ${functionName} is ${size} lines (max ${shellFunctionLimit})`);
    }
    functionName = undefined;
  }
}

for (const rule of rules) {
  for (const path of filesIn(rule.directory)) {
    if (!rule.extensions.some((extension) => path.endsWith(extension))) continue;
    const maximum = shellAllowances.get(path) ?? rule.maxLines;
    const lines = lineCount(path);
    if (lines > maximum) failures.push(`${path}: ${lines} lines exceeds ${maximum}`);
    if (path.endsWith(".sh")) checkShellFunctions(path);
  }
}

if (failures.length) {
  process.stderr.write(`Code policy violations:\n${failures.map((failure) => `- ${failure}`).join("\n")}\n`);
  process.exit(1);
}

process.stdout.write("Code policy checks passed.\n");
