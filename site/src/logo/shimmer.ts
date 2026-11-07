const BLOBS = 9;
const TICK_MS = 50;
const REST = 0.08;
const SPREAD = 0.3;
const SCATTER_PX = 10;
const JOLT_PX = 40;

type Motion = "scatter" | "jolt";

export class Shimmer {
  private readonly blobs: HTMLElement[] = [];
  private readonly weights: number[] = [];
  private tick = -1;

  constructor(container: HTMLElement) {
    for (let i = 0; i < BLOBS; i++) {
      const blob = document.createElement("span");
      blob.className = "ground-glow__blob";
      blob.style.left = `${((i + 0.5) / BLOBS) * 100}%`;
      container.append(blob);
      this.blobs.push(blob);
      this.weights.push(Math.sin((Math.PI * (i + 0.5)) / BLOBS));
    }
    this.rest();
  }

  update(level: number, motion: Motion) {
    const tick = Math.floor(performance.now() / TICK_MS);
    if (tick === this.tick) return;
    this.tick = tick;
    const jolt = (Math.random() - 0.5) * 2 * JOLT_PX * level;
    this.blobs.forEach((blob, i) => {
      const weight = this.weights[i];
      const offset =
        motion === "jolt" ? jolt : (Math.random() - 0.5) * 2 * SCATTER_PX;
      blob.style.opacity = String(
        REST + level * weight * SPREAD * Math.random(),
      );
      blob.style.transform = `translateX(${offset * weight}px)`;
    });
  }

  rest() {
    this.tick = -1;
    for (const blob of this.blobs) {
      blob.style.opacity = String(REST);
      blob.style.transform = "";
    }
  }
}
