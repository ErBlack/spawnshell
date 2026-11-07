import { GRID_SIZE } from "./grid-size.ts";
import type { Grid } from "./load-grid.ts";

const GLYPH_STROKE = 0.07;
const BRAILLE = 0x2800;
const BRAILLE_DOTS: [number, number][] = [
  [0, 0],
  [0, 1],
  [0, 2],
  [1, 0],
  [1, 1],
  [1, 2],
  [0, 3],
  [1, 3],
];

export function drawGrid(context: CanvasRenderingContext2D, grid: Grid) {
  const { width, height } = context.canvas;
  const cellWidth = width / GRID_SIZE.columns;
  const cellHeight = height / GRID_SIZE.rows;
  context.clearRect(0, 0, width, height);
  context.font = `bold ${(cellHeight * 12) / 14}px "Fira Code"`;
  context.textAlign = "center";
  context.textBaseline = "middle";
  context.lineWidth = cellHeight * GLYPH_STROKE;
  context.lineJoin = "round";
  grid.forEach((cells, row) => {
    cells.forEach((cell, column) => {
      if (!cell) return;
      const x = (column + (cell.shift ?? 0)) * cellWidth;
      const y = row * cellHeight;
      const braille = (cell.char.codePointAt(0) ?? 0) - BRAILLE;
      if (cell.bg) {
        context.fillStyle = cell.bg;
        context.fillRect(x, y, cellWidth + 0.5, cellHeight + 0.5);
      }
      context.fillStyle = cell.fg;
      if (cell.char === "▀")
        context.fillRect(x, y, cellWidth + 0.5, cellHeight / 2 + 0.5);
      else if (cell.char === "▄")
        context.fillRect(
          x,
          y + cellHeight / 2,
          cellWidth + 0.5,
          cellHeight / 2 + 0.5,
        );
      else if (cell.char === "█")
        context.fillRect(x, y, cellWidth + 0.5, cellHeight + 0.5);
      else if (braille > 0 && braille <= 0xff) {
        const radius = Math.min(cellWidth / 4, cellHeight / 8) * 0.9;
        context.beginPath();
        BRAILLE_DOTS.forEach(([dotColumn, dotRow], bit) => {
          if (!(braille & (1 << bit))) return;
          const cx = x + (cellWidth * (1 + 2 * dotColumn)) / 4;
          const cy = y + (cellHeight * (1 + 2 * dotRow)) / 8;
          context.moveTo(cx + radius, cy);
          context.arc(cx, cy, radius, 0, 2 * Math.PI);
        });
        context.fill();
      } else if (cell.char !== " ") {
        context.strokeStyle = cell.fg;
        context.strokeText(cell.char, x + cellWidth / 2, y + cellHeight / 2);
        context.fillText(cell.char, x + cellWidth / 2, y + cellHeight / 2);
      }
    });
  });
}
