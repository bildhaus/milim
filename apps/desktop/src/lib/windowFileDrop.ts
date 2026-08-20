export function dataTransferCarriesFiles(types: readonly string[]): boolean {
  return types.includes("Files");
}

export const WINDOW_ATTACH_FILES_EVENT = "milim:attach-files";
