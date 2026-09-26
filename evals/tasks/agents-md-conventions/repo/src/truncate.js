/**
 * Shorten text to at most `max` characters, adding an ellipsis when cut.
 *
 * @param {string} text - Input text.
 * @param {number} max - Maximum length including the ellipsis.
 * @returns {string} The possibly shortened text.
 * @example
 * truncate("abcdef", 4); // "abc…"
 */
export function truncate(text, max) {
  if (typeof text !== "string") {
    throw new TypeError("text must be a string");
  }
  const chars = [...text];
  return chars.length <= max ? text : `${chars.slice(0, max - 1).join("")}…`;
}
