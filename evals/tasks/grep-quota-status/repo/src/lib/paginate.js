export function paginate(rows, page, pageSize) {
  const start = (Number(page) - 1) * Number(pageSize);
  return { items: rows.slice(start, start + Number(pageSize)), total: rows.length };
}
