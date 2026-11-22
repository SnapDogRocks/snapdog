/**
 * Scroll `list` just enough to show `row`, like `scrollIntoView({ block: "nearest" })`, but
 * only the list itself: the page and other scrollable ancestors stay where they are.
 * Returns whether the list scrolled.
 */
export function scrollRowIntoList(list: HTMLElement, row: HTMLElement): boolean {
  const listRect = list.getBoundingClientRect();
  const rowRect = row.getBoundingClientRect();
  const top = rowRect.top - listRect.top + list.scrollTop;
  const bottom = top + rowRect.height;
  let next = list.scrollTop;
  if (top < list.scrollTop) next = top;
  else if (bottom > list.scrollTop + list.clientHeight) next = bottom - list.clientHeight;
  if (next === list.scrollTop) return false;
  list.scrollTop = next;
  return true;
}
