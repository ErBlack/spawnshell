export function isShortcut(event: KeyboardEvent): boolean {
  return (
    event.code === "Backquote" &&
    event.ctrlKey &&
    !event.metaKey &&
    !event.altKey &&
    !event.shiftKey
  );
}
