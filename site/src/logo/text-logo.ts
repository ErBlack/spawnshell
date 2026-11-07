import { GRID_SIZE } from "./grid/grid-size.ts";
import type { Grid } from "./grid/load-grid.ts";

const { columns: COLUMNS, rows: ROWS } = GRID_SIZE;

interface Shown {
  char: string;
  fg: string;
  bg: string;
  shift: boolean;
}

const BLANK: Shown = { char: " ", fg: "", bg: "", shift: false };

export class TextLogo {
  private readonly spans: HTMLSpanElement[][] = [];
  private readonly shown: Shown[][] = [];

  constructor(container: HTMLElement) {
    for (let row = 0; row < ROWS; row++) {
      const line = document.createElement("div");
      const spans: HTMLSpanElement[] = [];
      for (let column = 0; column < COLUMNS; column++) {
        const span = document.createElement("span");
        span.textContent = BLANK.char;
        spans.push(span);
      }
      line.append(...spans);
      container.append(line);
      this.spans.push(spans);
      this.shown.push(Array.from({ length: COLUMNS }, () => BLANK));
    }
  }

  render(grid: Grid) {
    for (let row = 0; row < ROWS; row++)
      for (let column = 0; column < COLUMNS; column++) {
        const cell = grid[row]?.[column];
        const next: Shown = {
          char: cell?.char ?? " ",
          fg: cell?.fg ?? "",
          bg: cell?.bg ?? "",
          shift: Boolean(cell?.shift),
        };
        const previous = this.shown[row][column];
        const span = this.spans[row][column];
        if (previous.char !== next.char) span.textContent = next.char;
        if (previous.fg !== next.fg) span.style.color = next.fg;
        if (previous.bg !== next.bg) span.style.backgroundColor = next.bg;
        if (previous.shift !== next.shift)
          span.toggleAttribute("data-shifted", next.shift);
        this.shown[row][column] = next;
      }
  }
}
