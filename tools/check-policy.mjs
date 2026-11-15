#!/usr/bin/env node
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const limits = [
  { directory: ".", extension: ".sh", maxLines: 350, recursive: false },
  { directory: "tunnel-api/src", extension: ".ts", maxLines: 300 },
  { directory: "tunnel-agent/src", extension: ".rs", maxLines: 500 },
  { directory: "tools", extension: ".sh", maxLines: 350 },
  { directory: "tunnel-files/static", extension: ".sh", maxLines: 350 },
];
const allowances = new Map([
  ["tunnel-agent/src/manifest.rs", 1850],
  ["tunnel-files/static/install.sh", 450],
]);
const typeScriptFunctionLimit = 90;
const failures = [];

function filesIn(directory, extension, recursive = true) {
  return readdirSync(join(root, directory), { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) return recursive ? filesIn(path, extension, recursive) : [];
    return entry.isFile() && entry.name.endsWith(extension) ? [path] : [];
  });
}

function lineCount(path) {
  return readFileSync(join(root, path), "utf8").split("\n").length - 1;
}

function countBraces(line) {
  return [...line].reduce((depth, character) => depth + (character === "{") - (character === "}"), 0);
}

function typeScriptFunctions(path) {
  const lines = readFileSync(join(root, path), "utf8").split("\n");
  const functions = [];
  const declaration = /^\s*(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_$][\w$]*)\s*\(/;
  for (let index = 0; index < lines.length; index += 1) {
    const match = lines[index].match(declaration);
    if (!match) continue;
    let depth = 0;
    let began = false;
    let end = index;
    for (; end < lines.length; end += 1) {
      if (lines[end].includes("{")) began = true;
      if (began) depth += countBraces(lines[end]);
      if (began && depth === 0) break;
    }
    functions.push({ name: match[1], start: index + 1, end: end + 1, body: lines.slice(index, end + 1).join("") });
    index = end;
  }
  return functions;
}

function normalized(body) {
  return body.replace(/\/\/.*$/gm, "").replace(/\s+/g, "");
}

for (const rule of limits) {
  for (const path of filesIn(rule.directory, rule.extension, rule.recursive)) {
    const maximum = allowances.get(path) ?? rule.maxLines;
    const lines = lineCount(path);
    if (lines > maximum) failures.push(`${path}: ${lines} lines exceeds ${maximum}`);
  }
}

const seenNames = new Map();
const seenBodies = new Map();
for (const path of filesIn("tunnel-api/src", ".ts")) {
  for (const fn of typeScriptFunctions(path)) {
    const lines = fn.end - fn.start + 1;
    if (lines > typeScriptFunctionLimit) failures.push(`${path}:${fn.start}: ${fn.name} is ${lines} lines (max ${typeScriptFunctionLimit})`);
    const existingName = seenNames.get(fn.name);
    if (existingName) failures.push(`${path}:${fn.start}: duplicate function name ${fn.name}; first declared at ${existingName}`);
    seenNames.set(fn.name, `${path}:${fn.start}`);
    const body = normalized(fn.body.slice(fn.body.indexOf("{") + 1));
    if (body.length < 160) continue;
    const existingBody = seenBodies.get(body);
    if (existingBody) failures.push(`${path}:${fn.start}: duplicate function body; first declared at ${existingBody}`);
    seenBodies.set(body, `${path}:${fn.start}`);
  }
}

if (failures.length) {
  process.stderr.write(`Code policy violations:\n${failures.map((failure) => `- ${failure}`).join("\n")}\n`);
  process.exit(1);
}

process.stdout.write("Code policy checks passed.\n");
