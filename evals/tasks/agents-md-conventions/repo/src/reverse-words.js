/**
 * Reverse the order of words while keeping the original spacing.
 *
 * @param {string} text - Input text.
 * @returns {string} The text with its words in reverse order.
 * @example
 * reverseWords("a  b c"); // "c  b a"
 */
export function reverseWords(text) {
  if (typeof text !== "string") {
    throw new TypeError("text must be a string");
  }
  const parts = text.split(/(\s+)/);
  const words = parts.filter((_, index) => index % 2 === 0).reverse();
  return parts.map((part, index) => (index % 2 === 0 ? words[index / 2] : part)).join("");
}
