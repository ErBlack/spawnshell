import { GRID_SIZE } from "../grid/grid-size.ts";
import type { Grid } from "../grid/load-grid.ts";
import { glyphField } from "./glyph-field.ts";
import { random } from "./random.ts";

const { columns: COLUMNS, rows: ROWS } = GRID_SIZE;
const TICK_MS = 50;
const START_MS = 600;
const SCRAMBLE_FROM_MS = 300;
const SCRAMBLE_TO_MS = 900;
const DURATION_MS = START_MS + SCRAMBLE_TO_MS;
const SCATTER_CHANCE = 0.2;
const MAX_NEIGHBORS = 3;
const NEIGHBOR_RADIUS = 5;

interface Scrambled {
  start: number;
  settle: number;
  fg: string;
}

const timing = () => {
  const start = Math.random() * START_MS;
  const scramble =
    SCRAMBLE_FROM_MS + Math.random() * (SCRAMBLE_TO_MS - SCRAMBLE_FROM_MS);
  return { start, settle: start + scramble };
};

export function retype(
  render: (grid: Grid) => void,
  from: Grid,
  to: Grid,
  onLevel: (level: number) => void,
): Promise<void> {
  const plan: (Scrambled | undefined)[][] = Array.from({ length: ROWS }, () =>
    Array.from({ length: COLUMNS }),
  );
  for (let row = 0; row < ROWS; row++)
    for (let column = 0; column < COLUMNS; column++) {
      const filled = to[row][column] ?? from[row][column];
      if (!filled) continue;
      plan[row][column] = { ...timing(), fg: filled.fg };
      if (Math.random() >= SCATTER_CHANCE) continue;
      const empty: [number, number][] = [];
      for (let y = row - NEIGHBOR_RADIUS; y <= row + NEIGHBOR_RADIUS; y++)
        for (
          let x = column - NEIGHBOR_RADIUS;
          x <= column + NEIGHBOR_RADIUS;
          x++
        )
          if (
            y >= 0 &&
            y < ROWS &&
            x >= 0 &&
            x < COLUMNS &&
            !to[y][x] &&
            !from[y][x]
          )
            empty.push([y, x]);
      for (let count = 1 + random(MAX_NEIGHBORS); count > 0; count--) {
        const [y, x] = empty.splice(random(empty.length), 1)[0] ?? [];
        if (y === undefined) break;
        plan[y][x] ??= { ...timing(), fg: filled.fg };
      }
    }
  const total = plan.flat().filter(Boolean).length || 1;
  const glyphAt = glyphField();
  const start = performance.now();

  return new Promise((resolve) => {
    const frame = (now: number) => {
      const elapsed = now - start;
      const tick = Math.floor(elapsed / TICK_MS);
      let scrambling = 0;
      render(
        plan.map((cells, row) =>
          cells.map((cell, column) => {
            if (!cell || elapsed >= cell.settle) return to[row][column];
            if (elapsed < cell.start) return from[row][column];
            scrambling++;
            return {
              char: glyphAt(row, column, tick),
              fg: cell.fg,
            };
          }),
        ),
      );
      onLevel(scrambling / total);
      if (elapsed < DURATION_MS) requestAnimationFrame(frame);
      else resolve();
    };
    requestAnimationFrame(frame);
  });
}
