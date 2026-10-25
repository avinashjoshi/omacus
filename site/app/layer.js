// Where the sidebar's row menu sits, and which taps only close the phone drawer.

const DRAWER_KEEP = ".side, .sheets-toggle, .bar, [data-row-menu]";

export function placeRowMenu(x, y, size, frame, anchor) {
  const margin = 8;
  const width = Math.max(0, size?.width || 0);
  const height = Math.max(0, size?.height || 0);
  let left = x;
  let top = y;
  if (left + width > frame.width - margin) left = frame.width - width - margin;
  if (left < margin) left = margin;
  if (top + height > frame.height - margin) {
    const anchorTop = anchor && Number.isFinite(anchor.top) ? anchor.top : y;
    const above = anchorTop - height - 4;
    top = above >= margin ? above : Math.max(margin, frame.height - height - margin);
  }
  if (top < margin) top = margin;
  return { left: Math.round(left), top: Math.round(top) };
}

export function swallowDrawerTap(node, { drawer, narrow }) {
  if (!drawer || !narrow) return false;
  const target = node && typeof node.closest === "function"
    ? node
    : node?.parentElement && typeof node.parentElement.closest === "function"
      ? node.parentElement
      : null;
  if (!target) return false;
  if (target.closest(DRAWER_KEEP)) return false;
  return true;
}
