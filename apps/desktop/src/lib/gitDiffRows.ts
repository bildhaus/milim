export type DiffRowKind = "file" | "meta" | "hunk" | "add" | "delete" | "context" | "stat" | "blank";

export type DiffRow = {
  kind: DiffRowKind;
  oldNo: string;
  newNo: string;
  marker: string;
  text: string;
};

export type DiffSection = {
  id: string;
  path: string;
  start: number;
  end: number;
  additions: number;
  deletions: number;
  lineCount: number;
};

export type DiffStats = {
  files: number;
  additions: number;
  deletions: number;
};

export type GitFileTreeNode = {
  name: string;
  path: string;
  children: GitFileTreeNode[];
  fileIndex?: number;
};

function cleanGitPath(path: string): string {
  const trimmed = path.trim();
  if (trimmed.length >= 2 && trimmed.startsWith("\"") && trimmed.endsWith("\"")) {
    return trimmed.slice(1, -1).replace(/\\"/g, "\"").replace(/\\\\/g, "\\");
  }
  return trimmed;
}

function destinationPath(path: string): string {
  const cleaned = cleanGitPath(path);
  return cleaned.includes(" -> ")
    ? cleaned.slice(cleaned.lastIndexOf(" -> ") + 4)
    : cleaned.replace(/\{[^{}]* => ([^{}]*)\}/g, "$1");
}

export function gitFileTree(paths: string[]): GitFileTreeNode[] {
  type MutableNode = {
    name: string;
    path: string;
    children: Map<string, MutableNode>;
    fileIndex?: number;
  };

  const root = new Map<string, MutableNode>();
  paths.forEach((rawPath, fileIndex) => {
    const parts = destinationPath(rawPath).split(/[\\/]/).filter(Boolean);
    let siblings = root;
    let path = "";
    parts.forEach((name, index) => {
      path = path ? `${path}/${name}` : name;
      let node = siblings.get(name);
      if (!node) {
        node = { name, path, children: new Map() };
        siblings.set(name, node);
      }
      if (index === parts.length - 1) node.fileIndex = fileIndex;
      siblings = node.children;
    });
  });

  function finalize(nodes: Map<string, MutableNode>): GitFileTreeNode[] {
    return [...nodes.values()]
      .map((node): GitFileTreeNode => {
        let result: GitFileTreeNode = {
          name: node.name,
          path: node.path,
          children: finalize(node.children),
          fileIndex: node.fileIndex,
        };
        while (
          result.fileIndex === undefined &&
          result.children.length === 1 &&
          result.children[0].fileIndex === undefined
        ) {
          const child = result.children[0];
          result = {
            name: `${result.name}/${child.name}`,
            path: child.path,
            children: child.children,
          };
        }
        return result;
      })
      .sort((a, b) => {
        const folderOrder = Number(a.fileIndex !== undefined) - Number(b.fileIndex !== undefined);
        return folderOrder || a.name.localeCompare(b.name);
      });
  }

  return finalize(root);
}

function parseHunkStart(line: string): { oldNo: number; newNo: number } | null {
  const match = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/);
  if (!match) return null;
  return { oldNo: Number(match[1]), newNo: Number(match[2]) };
}

function diffFileLabel(line: string): string {
  const match = line.match(/^diff --git a\/(.+) b\/(.+)$/);
  return match ? cleanGitPath(match[2]) : line;
}

function contentLineText(line: string): string {
  return line.length > 0 ? line.slice(1) || " " : " ";
}

export function diffRows(text: string): DiffRow[] {
  let oldNo: number | null = null;
  let newNo: number | null = null;
  let inPatch = false;

  const rows = text.split(/\r?\n/).map((line): DiffRow => {
    if (line.startsWith("diff --git ")) {
      inPatch = true;
      oldNo = null;
      newNo = null;
      return { kind: "file", oldNo: "", newNo: "", marker: "", text: diffFileLabel(line) };
    }

    if (!inPatch) {
      return { kind: line.trim() ? "stat" : "blank", oldNo: "", newNo: "", marker: "", text: line || " " };
    }

    if (line.startsWith("@@")) {
      const hunk = parseHunkStart(line);
      if (hunk) {
        oldNo = hunk.oldNo;
        newNo = hunk.newNo;
      }
      return { kind: "hunk", oldNo: "", newNo: "", marker: "@@", text: line };
    }

    if (
      line.startsWith("index ") ||
      line.startsWith("--- ") ||
      line.startsWith("+++ ") ||
      line.startsWith("new file mode") ||
      line.startsWith("deleted file mode") ||
      line.startsWith("\\ No newline")
    ) {
      return { kind: "meta", oldNo: "", newNo: "", marker: "", text: line || " " };
    }

    if (line.startsWith("+")) {
      const row = { kind: "add" as const, oldNo: "", newNo: newNo == null ? "" : String(newNo), marker: "+", text: contentLineText(line) };
      if (newNo != null) newNo += 1;
      return row;
    }

    if (line.startsWith("-")) {
      const row = { kind: "delete" as const, oldNo: oldNo == null ? "" : String(oldNo), newNo: "", marker: "-", text: contentLineText(line) };
      if (oldNo != null) oldNo += 1;
      return row;
    }

    const row = {
      kind: "context" as const,
      oldNo: oldNo == null ? "" : String(oldNo),
      newNo: newNo == null ? "" : String(newNo),
      marker: " ",
      text: contentLineText(line),
    };
    if (oldNo != null) oldNo += 1;
    if (newNo != null) newNo += 1;
    return row;
  });
  const firstPatchRow = rows.findIndex((row) => row.kind === "file");
  return firstPatchRow >= 0 ? rows.slice(firstPatchRow) : rows;
}

export type DiffHunk = {
  path: string;
  /** Header plus body lines exactly as rendered, for server-side re-validation. */
  text: string;
};

/**
 * Raw hunks in the same order as the `hunk` rows produced by `diffRows`, so
 * the nth hunk row maps to the nth entry.
 */
export function diffHunks(text: string): DiffHunk[] {
  const hunks: DiffHunk[] = [];
  let path: string | null = null;
  let current: string[] | null = null;
  const flush = () => {
    if (current && path != null) {
      while (current.length > 1 && current[current.length - 1] === "") current.pop();
      hunks.push({ path, text: current.join("\n") });
    }
    current = null;
  };
  for (const line of text.split(/\r?\n/)) {
    if (line.startsWith("diff --git ")) {
      flush();
      path = diffFileLabel(line);
      continue;
    }
    if (path == null) continue;
    if (line.startsWith("@@")) {
      flush();
      current = [line];
      continue;
    }
    current?.push(line);
  }
  flush();
  return hunks;
}

export function diffSections(rows: DiffRow[]): DiffSection[] {
  const sections: DiffSection[] = [];
  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index];
    if (row.kind !== "file") continue;
    const end = rows.findIndex((next, nextIndex) => nextIndex > index && next.kind === "file");
    const sectionRows = rows.slice(index, end === -1 ? rows.length : end);
    const additions = sectionRows.filter((item) => item.kind === "add").length;
    const deletions = sectionRows.filter((item) => item.kind === "delete").length;
    sections.push({
      id: `${sections.length}:${row.text}`,
      path: row.text,
      start: index,
      end: end === -1 ? rows.length : end,
      additions,
      deletions,
      lineCount: sectionRows.length,
    });
  }
  return sections;
}

export function diffStats(rows: DiffRow[]): DiffStats {
  return diffSections(rows).reduce(
    (stats, section) => ({
      files: stats.files + 1,
      additions: stats.additions + section.additions,
      deletions: stats.deletions + section.deletions,
    }),
    { files: 0, additions: 0, deletions: 0 },
  );
}

export function findDiffSectionIndex(
  sections: DiffSection[],
  path: string,
  fallbackIndex = -1,
): number {
  const cleaned = cleanGitPath(path);
  const destination = destinationPath(path);
  const matched = sections.findIndex(
    (section) => section.path === cleaned || section.path === destination,
  );
  return matched >= 0
    ? matched
    : fallbackIndex >= 0 && fallbackIndex < sections.length
      ? fallbackIndex
      : -1;
}

export function shouldCollapseDiffSection(section: DiffSection): boolean {
  return section.lineCount > 320 || /(^|\/)(package-lock\.json|pnpm-lock\.yaml|yarn\.lock|bun\.lockb|Cargo\.lock)$/i.test(section.path);
}
