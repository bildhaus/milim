// Shared by the solution scripts: text edits on the repository in the
// current directory.
import { readFileSync, writeFileSync } from "node:fs";

export function replace(path, from, to) {
  const source = readFileSync(path, "utf8");
  if (!source.includes(from)) throw new Error(`${path}: expected text not found: ${from.slice(0, 60)}`);
  writeFileSync(path, source.replace(from, () => to));
}

export const applyRefund = (body) =>
  replace(
    "src/events.ts",
    `      return event.direction === "in" ? balance + event.amount : balance - event.amount;
    default:`,
    `      return event.direction === "in" ? balance + event.amount : balance - event.amount;
    case "refund":
      ${body}
    default:`,
  );

export const describeRefund = (body) =>
  replace(
    "src/format.ts",
    `      return \`transfer \${event.direction} \${cents(event.amount)}\`;
  }`,
    `      return \`transfer \${event.direction} \${cents(event.amount)}\`;
    case "refund":
      ${body}
  }`,
  );

export const totalsWithRefund = () =>
  replace("src/summary.ts", "{ deposit: 0, withdrawal: 0, transfer: 0 }", "{ deposit: 0, withdrawal: 0, transfer: 0, refund: 0 }");
