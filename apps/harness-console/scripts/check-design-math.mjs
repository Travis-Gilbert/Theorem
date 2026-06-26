import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(fileURLToPath(new URL("..", import.meta.url)));
const SOURCES = ["src"];
const EXTENSIONS = new Set([".css", ".ts", ".tsx"]);
const IGNORED_DIRS = new Set(["node_modules", ".next", "out", "build"]);

const arbitrarySpacingClass =
  /\b(?:m[trblxy]?|p[trblxy]?|gap(?:-[xy])?|space-[xy])-\[[^\]]*\d(?:\.\d+)?px[^\]]*\]/g;
const cssSpacingDeclaration =
  /\b(?:margin(?:-(?:top|right|bottom|left|inline|block))?|padding(?:-(?:top|right|bottom|left|inline|block))?|gap|row-gap|column-gap)\s*:\s*[^;]*\d(?:\.\d+)?px[^;]*;/g;
const jsSpacingProperty =
  /\b(?:margin|marginTop|marginRight|marginBottom|marginLeft|padding|paddingTop|paddingRight|paddingBottom|paddingLeft|gap|rowGap|columnGap)\s*:\s*["'`][^"'`]*\d(?:\.\d+)?px/g;
const arbitraryRawColorClass =
  /\b(?:bg|text|border|fill|stroke|ring|from|via|to)-\[#(?:[0-9a-fA-F]{3,8})\]/g;
const jsRawColorProperty =
  /\b(?:color|background|backgroundColor|borderColor|fill|stroke)\s*:\s*["'`]#[0-9a-fA-F]{3,8}["'`]/g;

const violations = [];

for (const source of SOURCES) {
  walk(join(ROOT, source));
}

if (violations.length > 0) {
  console.error("Design math lint failed: raw spacing or color values found.");
  for (const violation of violations) {
    console.error(`- ${violation}`);
  }
  process.exit(1);
}

console.log("Design math lint passed: spacing and token checks clean.");

function walk(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (!IGNORED_DIRS.has(entry.name)) {
        walk(join(dir, entry.name));
      }
      continue;
    }

    const file = join(dir, entry.name);
    if (!EXTENSIONS.has(extension(entry.name))) {
      continue;
    }
    checkFile(file);
  }
}

function checkFile(file) {
  const rel = relative(ROOT, file);
  const lines = readFileSync(file, "utf8").split("\n");
  lines.forEach((line, index) => {
    const matches = [
      ...line.matchAll(arbitrarySpacingClass),
      ...line.matchAll(cssSpacingDeclaration),
      ...line.matchAll(jsSpacingProperty),
      ...line.matchAll(arbitraryRawColorClass),
      ...line.matchAll(jsRawColorProperty),
    ];
    for (const match of matches) {
      violations.push(`${rel}:${index + 1}: ${match[0].trim()}`);
    }
  });
}

function extension(name) {
  const idx = name.lastIndexOf(".");
  return idx === -1 ? "" : name.slice(idx);
}
