import { GRID_SIZE } from "../grid/grid-size.ts";
import { random } from "./random.ts";

const GLYPHS = [..."!<>-_\\/[]{}=+*^?#01ΔΞΨΩ"];

function shuffled<T>(items: readonly T[]): T[] {
  const result = [...items];
  for (let i = result.length - 1; i > 0; i--) {
    const j = random(i + 1);
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
}

function walk(length: number, steps: readonly number[], modulus: number) {
  const offsets = [random(modulus)];
  for (let i = 1; i < length; i++) {
    const step = steps[random(steps.length)];
    const signed = Math.random() < 0.5 ? step : modulus - step;
    offsets.push((offsets[i - 1] + signed) % modulus);
  }
  return offsets;
}

export function glyphField(): (
  row: number,
  column: number,
  tick: number,
) => string {
  const count = GLYPHS.length;
  const glyphs = shuffled(GLYPHS);
  const halves = shuffled(
    Array.from({ length: (count - 1) / 2 }, (_, i) => i + 1),
  );
  const split = 1 + random(halves.length - 1);
  const columns = walk(GRID_SIZE.columns, halves.slice(0, split), count);
  const rows = walk(GRID_SIZE.rows, halves.slice(split), count);
  const step = 1 + random(count - 1);
  return (row, column, tick) =>
    glyphs[(rows[row] + columns[column] + tick * step) % count];
}
