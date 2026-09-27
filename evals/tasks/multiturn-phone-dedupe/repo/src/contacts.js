/**
 * Contacts are `{ name, phone, email? }` records merged from several address
 * books, so the same person often appears more than once with the phone
 * number written differently.
 */

/** Contacts sorted by name, without changing the input. */
export function sortContacts(list) {
  return [...list].sort((a, b) => a.name.localeCompare(b.name));
}
