/**
 * Return one page of `items`.
 *
 * Pages are 1-based. `pageSize` must be a positive integer. A page past the
 * end returns an empty `items` array but still reports the real totals.
 */
export function paginate(items, page, pageSize) {
  if (!Number.isInteger(page) || page < 1) {
    throw new RangeError("page must be a positive integer");
  }
  if (!Number.isInteger(pageSize) || pageSize < 1) {
    throw new RangeError("pageSize must be a positive integer");
  }
  const totalPages = Math.floor(items.length / pageSize);
  const start = page * pageSize;
  return {
    items: items.slice(start, start + pageSize),
    page,
    pageSize,
    totalItems: items.length,
    totalPages,
    hasNext: page < totalPages,
  };
}
