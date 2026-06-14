// Runtime stand-ins for the Obsidian API symbols that src/sync.ts (and its
// imports) touch at runtime. The real `obsidian` package ships only type
// declarations (its package `main` is empty), so tests must provide a runtime.
//
// esbuild aliases the bare `obsidian` import to this file for the whole test
// bundle, so the classes here have a single identity that both the code under
// test and the test's fakes share — which is what makes `instanceof` work.

export class TAbstractFile {
  path: string;
  name: string;
  constructor(path: string) {
    this.path = path;
    this.name = path.split("/").pop() ?? path;
  }
}

export class TFile extends TAbstractFile {
  basename: string;
  extension: string;
  constructor(path: string) {
    super(path);
    const dot = this.name.lastIndexOf(".");
    this.basename = dot > 0 ? this.name.slice(0, dot) : this.name;
    this.extension = dot > 0 ? this.name.slice(dot + 1) : "";
  }
}

export class TFolder extends TAbstractFile {
  children: TAbstractFile[] = [];
}

// `App` is only ever used as a type by the code under test; a bare class is
// enough to satisfy the ESM named-import binding.
export class App {}

/** A faithful-enough port of Obsidian's path normalizer for tests. */
export function normalizePath(path: string): string {
  return path
    .replace(/\\/g, "/")
    .replace(/\/{2,}/g, "/")
    .replace(/^\/+/, "")
    .replace(/\/+$/, "");
}
