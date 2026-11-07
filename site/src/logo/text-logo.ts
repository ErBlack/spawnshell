import { GRID_SIZE } from "./grid/grid-size.ts";
import type { Grid } from "./grid/load-grid.ts";

const { rows: ROWS } = GRID_SIZE;

export class TextLogo {
  private readonly text = new Text();
  private readonly sheet = new CSSStyleSheet();
  private readonly highlights = new Map<string, Highlight>();

  constructor(container: HTMLElement) {
    container.append(this.text);
    document.adoptedStyleSheets.push(this.sheet);
  }

  render(grid: Grid) {
    const lines: string[] = [];
    const runs = new Map<string, [number, number][]>();
    let start = 0;
    for (let row = 0; row < ROWS; row++) {
      let line = "";
      let previous: [number, number] | undefined;
      let previousKey = "";
      const cells = grid[row] ?? [];
      const length = cells.findLastIndex(Boolean) + 1;
      for (let column = 0; column < length; column++) {
        const cell = cells[column];
        line += cell?.char ?? " ";
        const key = cell ? `${cell.fg} ${cell.bg ?? "transparent"}` : "";
        const offset = start + column;
        if (key && key === previousKey && previous) {
          previous[1] = offset + 1;
          continue;
        }
        previousKey = key;
        previous = undefined;
        if (!key) continue;
        previous = [offset, offset + 1];
        const list = runs.get(key) ?? [];
        list.push(previous);
        runs.set(key, list);
      }
      lines.push(line);
      start += line.length + 1;
    }
    this.text.data = lines.join("\n");
    for (const highlight of this.highlights.values()) highlight.clear();
    for (const [key, spans] of runs) {
      let highlight = this.highlights.get(key);
      if (!highlight) {
        highlight = new Highlight();
        const name = `logo-${this.highlights.size}`;
        const [fg, bg] = key.split(" ");
        this.highlights.set(key, highlight);
        CSS.highlights.set(name, highlight);
        this.sheet.insertRule(
          `::highlight(${name}) { color: ${fg}; background-color: ${bg}; }`,
        );
      }
      for (const [start, end] of spans) {
        const range = new Range();
        range.setStart(this.text, start);
        range.setEnd(this.text, end);
        highlight.add(range);
      }
    }
  }
}
