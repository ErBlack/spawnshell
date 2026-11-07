import { type Grid, loadGrid } from "./grid/load-grid.ts";
import { Shimmer } from "./shimmer.ts";
import { TextLogo } from "./text-logo.ts";

const VECTOR = "vector";
const VARIANTS = ["logo", "shade", "braille", "half", "wire", VECTOR];

const loadAnimations = () =>
  Promise.all([import("./retype/retype.ts"), import("./glitch-between.ts")]);

const resolveVariant = async (name: string) =>
  name === VECTOR ? undefined : loadGrid(name);

export async function mountLogo(logo: HTMLElement, ground: HTMLElement) {
  const textBox = logo.querySelector(".logo__text") as HTMLElement;
  const vectorBox = logo.querySelector(".logo__vector") as HTMLImageElement;
  const canvas = logo.querySelector(".logo__effect") as HTMLCanvasElement;
  const button = logo.querySelector(".logo__switch") as HTMLButtonElement;
  const text = new TextLogo(textBox);
  const shimmer = new Shimmer(ground);
  let index = Math.floor(Math.random() * VARIANTS.length);
  let busy = false;
  let animations: ReturnType<typeof loadAnimations> | undefined;

  const prepare = () => {
    animations ??= loadAnimations();
    for (const name of VARIANTS) if (name !== VECTOR) loadGrid(name);
    return animations;
  };

  const show = (grid: Grid | undefined) => {
    canvas.hidden = true;
    textBox.hidden = !grid;
    vectorBox.hidden = Boolean(grid);
    if (grid) text.render(grid);
  };

  show(await resolveVariant(VARIANTS[index]));
  logo.dataset.ready = "";
  button.addEventListener("pointerenter", prepare, { once: true });
  button.addEventListener("focus", prepare, { once: true });
  button.addEventListener("click", async () => {
    if (busy) return;
    busy = true;
    const previous = VARIANTS[index];
    index = (index + 1) % VARIANTS.length;
    try {
      const to = await resolveVariant(VARIANTS[index]);
      try {
        if (!matchMedia("(prefers-reduced-motion: reduce)").matches) {
          const from = await resolveVariant(previous);
          const [{ retype }, { glitchBetween }] = await prepare();
          if (from && to) {
            await retype(
              (grid) => text.render(grid),
              from,
              to,
              (level) => shimmer.update(level, "scatter"),
            );
          } else {
            const play = await glitchBetween(canvas, vectorBox, from, to);
            if (play) {
              textBox.hidden = true;
              vectorBox.hidden = true;
              canvas.hidden = false;
              await play((level) => shimmer.update(level, "jolt"));
            }
          }
        }
      } finally {
        show(to);
      }
    } finally {
      shimmer.rest();
      busy = false;
    }
  });
}
