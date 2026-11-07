export function listenMessages<T>(
  source: () => Window | null | undefined,
  handle: (message: T) => void,
) {
  addEventListener("message", (event: MessageEvent) => {
    if (event.origin !== location.origin || event.source !== source()) return;
    const message = event.data?.spawnshell;
    if (typeof message?.type === "string") handle(message as T);
  });
}
