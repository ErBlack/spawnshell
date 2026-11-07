import { Glitch } from "./glitch/glitch.ts";
import { drawGrid } from "./grid/draw-grid.ts";
import type { Grid } from "./grid/load-grid.ts";

type Play = (onLevel: (level: number) => void) => Promise<void>;

let glitch: Glitch | undefined;

export async function glitchBetween(
  canvas: HTMLCanvasElement,
  vector: HTMLImageElement,
  from: Grid | undefined,
  to: Grid | undefined,
): Promise<Play | undefined> {
  glitch ??= Glitch.create(canvas);
  const ready = glitch;
  if (!ready) return undefined;
  await vector.decode();
  const box = canvas.parentElement ?? canvas;
  const ratio = devicePixelRatio || 1;
  canvas.width = Math.round(box.clientWidth * ratio);
  canvas.height = Math.round(box.clientHeight * ratio);
  const [source, target] = [from, to].map((grid) => {
    const raster = document.createElement("canvas");
    raster.width = canvas.width;
    raster.height = canvas.height;
    const context = raster.getContext("2d");
    if (!context) throw new Error("no 2d canvas");
    if (grid) drawGrid(context, grid);
    else context.drawImage(vector, 0, 0, raster.width, raster.height);
    return raster;
  });
  return (onLevel) => ready.play(source, target, onLevel);
}
