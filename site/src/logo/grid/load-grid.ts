import { GRID_SIZE } from "./grid-size.ts";

export type Grid = (
  | { char: string; fg: string; bg?: string; shift?: number }
  | undefined
)[][];

interface GridData {
  name: string;
  cols: number;
  text: string[];
  fg: string[];
  bg?: string[];
}

const files = import.meta.glob<GridData>("./data/*.json", {
  import: "default",
});
const loaded = new Map<string, Promise<Grid>>();

const color = (hex: string | undefined) => (hex ? `#${hex}` : undefined);

export function loadGrid(name: string): Promise<Grid> {
  let grid = loaded.get(name);
  if (!grid) {
    const file = files[`./data/${name}.json`];
    if (!file) throw new Error(`no logo grid ${name}`);
    grid = file().then((data): Grid => {
      const centered = (GRID_SIZE.columns - data.cols) / 2;
      const offset = Math.floor(centered);
      const shift = centered - offset || undefined;
      return data.text.map((line, row) => {
        const fg = data.fg[row].split(",");
        const bg = data.bg?.[row].split(",");
        const cells: Grid[number] = Array.from({ length: GRID_SIZE.columns });
        [...line].forEach((char, column) => {
          const background = color(bg?.[column]);
          if (char === " " && !background) return;
          cells[column + offset] = {
            char,
            fg: color(fg[column]) ?? "transparent",
            bg: background,
            shift,
          };
        });
        return cells;
      });
    });
    loaded.set(name, grid);
  }
  return grid;
}
