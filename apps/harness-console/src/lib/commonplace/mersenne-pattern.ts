const ARRAY_SIZE = 624;
const ARRAY_MAX = ARRAY_SIZE - 1;
const M = 397;
const ARRAY_SIZE_MINUS_M = ARRAY_SIZE - M;
const UPPER_MASK = 0x80000000;
const LOWER_MASK = 0x7fffffff;
const A = 0x9908b0df;

export interface MersenneGlyph {
  readonly id: string;
  readonly value: "0" | "1";
  readonly row: number;
  readonly column: number;
  readonly opacity: number;
  readonly delay: number;
}

export function createMersenneBinaryGlyphs({
  seed = 19937,
  rows = 18,
  columns = 30,
}: {
  seed?: number;
  rows?: number;
  columns?: number;
} = {}): readonly MersenneGlyph[] {
  const engine = new MersenneTwister(seed);
  const glyphs: MersenneGlyph[] = [];

  for (let row = 0; row < rows; row += 1) {
    for (let column = 0; column < columns; column += 1) {
      const next = engine.next();
      glyphs.push({
        id: `${row}:${column}`,
        value: (next & 1) === 1 ? "1" : "0",
        row,
        column,
        opacity: 0.18 + ((next >>> 8) % 42) / 1000,
        delay: (next >>> 16) % 2200,
      });
    }
  }

  return glyphs;
}

class MersenneTwister {
  private readonly data = new Int32Array(ARRAY_SIZE);
  private index = ARRAY_SIZE;

  constructor(seed: number) {
    this.seed(seed);
  }

  next(): number {
    if (this.index >= ARRAY_SIZE) {
      this.refresh();
      this.index = 0;
    }

    const value = this.data[this.index] ?? 0;
    this.index += 1;
    return temper(value) >>> 0;
  }

  private seed(initial: number) {
    let previous = initial | 0;
    this.data[0] = previous;

    for (let index = 1; index < ARRAY_SIZE; index += 1) {
      previous = (Math.imul(previous ^ (previous >>> 30), 0x6c078965) + index) | 0;
      this.data[index] = previous;
    }
  }

  private refresh() {
    let k = 0;
    let tmp = 0;

    for (; k < ARRAY_SIZE_MINUS_M; k += 1) {
      tmp = (this.data[k] & UPPER_MASK) | (this.data[k + 1] & LOWER_MASK);
      this.data[k] = this.data[k + M] ^ (tmp >>> 1) ^ ((tmp & 1) === 1 ? A : 0);
    }

    for (; k < ARRAY_MAX; k += 1) {
      tmp = (this.data[k] & UPPER_MASK) | (this.data[k + 1] & LOWER_MASK);
      this.data[k] = this.data[k - ARRAY_SIZE_MINUS_M] ^ (tmp >>> 1) ^ ((tmp & 1) === 1 ? A : 0);
    }

    tmp = (this.data[ARRAY_MAX] & UPPER_MASK) | (this.data[0] & LOWER_MASK);
    this.data[ARRAY_MAX] = this.data[M - 1] ^ (tmp >>> 1) ^ ((tmp & 1) === 1 ? A : 0);
  }
}

function temper(input: number) {
  let value = input;
  value ^= value >>> 11;
  value ^= (value << 7) & 0x9d2c5680;
  value ^= (value << 15) & 0xefc60000;
  return value ^ (value >>> 18);
}
