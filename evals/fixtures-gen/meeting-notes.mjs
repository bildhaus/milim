// Deterministic generator for the large-context fixture: fourteen weeks of
// long meeting notes (~640 KB in total) with the late-fee rules for
// src/fees.js scattered through them. Several rules are revised in later
// weeks, and the filler reuses words such as "late", "fee", "cap", "grace",
// "overdue", "enterprise", and "percent" in unrelated contexts, so a keyword
// search surfaces plenty of noise and reading the notes is the natural way
// to collect the rules.

import { rng } from "./large-files.mjs";

const WEEKS = 14;
const FIRST_MONDAY = Date.UTC(2024, 0, 8);
const TARGET_BYTES = 45_000;

const PEOPLE = [
  "Priya Raman",
  "Marco Bellini",
  "Dana Whitfield",
  "Kenji Mori",
  "Sofia Duarte",
  "Omar Haddad",
  "Lena Vogel",
  "Tomas Novak",
  "Aisha Bello",
  "Wei Zhang",
  "Grace Park",
  "Noah Fischer",
];
const TEAMS = ["Payments", "Platform", "Mobile", "Data", "Support", "Design", "Security", "Growth", "Infra", "Billing"];
const FEATURES = [
  "the new onboarding checklist",
  "saved search filters",
  "bulk CSV export",
  "the redesigned settings page",
  "SSO for enterprise workspaces",
  "offline mode for the mobile app",
  "the usage dashboard",
  "webhook retries with backoff",
  "two-factor recovery codes",
  "the audit log viewer",
  "multi-currency price display",
  "the invoice PDF redesign",
];
const METRICS = ["p95 latency", "signup conversion", "crash-free sessions", "ticket volume", "queue depth", "error rate", "weekly active teams", "cache hit rate"];
const TRENDS = [
  "flat week over week",
  "up about 3 percent",
  "down 2 percent after the rollout",
  "noisy because of the holiday traffic",
  "back to where it was before the incident",
  "slightly better than forecast",
  "worse on Android than on iOS",
];
const SYSTEMS = ["the job queue", "the search indexer", "the billing ledger", "the notification fan-out", "the report builder", "the CDN edge rules", "the auth gateway", "the metrics pipeline"];
const BUGS = ["duplicate email", "stale session", "timezone drift", "double submit", "missing avatar", "slow export", "flaky login", "orphaned upload"];
const CONDITIONS = [
  "two tabs are open at once",
  "the device clock is wrong",
  "a workspace has more than 500 members",
  "the export runs across midnight UTC",
  "the user switches accounts mid-request",
  "the CDN serves a cached redirect",
];

const pick = (next, list) => list[Math.floor(next() * list.length)];
const between = (next, low, high) => low + Math.floor(next() * (high - low + 1));
const dateOf = (week, days = 0) => new Date(FIRST_MONDAY + (week * 7 + days) * 86_400_000).toISOString().slice(0, 10);

/** Filler sentences; several reuse the rule vocabulary in unrelated senses. */
const SENTENCES = [
  (n) => `${pick(n, PEOPLE)} walked through the ${pick(n, METRICS)} numbers, which are ${pick(n, TRENDS)}.`,
  (n) => `${pick(n, FEATURES)} is behind a flag for ${between(n, 5, 60)} percent of workspaces and nobody has reported problems yet.`,
  (n) => `The ${pick(n, BUGS)} bug is still open; it only shows up when ${pick(n, CONDITIONS)}.`,
  (n) => {
    const [first, second] = shuffle(n, PEOPLE);
    return `${first} will pair with ${second} on ${pick(n, SYSTEMS)} next week.`;
  },
  (n) => `We ran late again, so the demo of ${pick(n, FEATURES)} moves to Thursday.`,
  (n) => `Latency on ${pick(n, SYSTEMS)} crept up by ${between(n, 10, 90)} ms at the 95th percentile after the last deploy.`,
  (n) => `The conference fee for the ${pick(n, ["Lisbon", "Austin", "Berlin", "Toronto"])} offsite was approved, capped at the usual travel budget.`,
  (n) => `The public API rate cap stays at ${between(n, 20, 80)} requests per second per token for now.`,
  (n) => `${pick(n, SYSTEMS)} now does a graceful shutdown instead of dropping in-flight work, which should end the grace-period alerts on deploys.`,
  (n) => `The postmortem for the ${pick(n, BUGS)} incident is overdue; ${pick(n, PEOPLE)} will finish the timeline first.`,
  (n) => `Code reviews older than two days are piling up again; ${pick(n, TEAMS)} asked for a rotation.`,
  (n) => `Capacity planning for ${pick(n, SYSTEMS)} assumes ${between(n, 2, 6)}x growth by the end of the year.`,
  (n) => `The enterprise SSO pilot has ${between(n, 3, 12)} customers, and two of them want SCIM before they expand.`,
  (n) => `Feedback from the design review: the empty states need copy that explains what to do next.`,
  (n) => `Coffee budget for the office is fine; nobody needs to approve the new grinder.`,
  (n) => `A rounding error in the ${pick(n, METRICS)} chart made the weekly email look worse than it was; ${pick(n, TEAMS)} fixed the query.`,
  (n) => `${pick(n, TEAMS)} is short one person until ${dateOf(between(n, 1, 13), between(n, 0, 4))}, so anything new goes through ${pick(n, PEOPLE)}.`,
  (n) => `The later of the two migration windows works better for ${pick(n, TEAMS)}, so we will use that one.`,
  (n) => `The nonprofit discount page got ${between(n, 40, 400)} visits from the newsletter but few signups.`,
  (n) => `On-call was quiet apart from one page about ${pick(n, SYSTEMS)} at ${between(n, 1, 5)} am, which turned out to be a noisy alert.`,
  (n) => `${pick(n, PEOPLE)} wants a written proposal before we change anything in ${pick(n, SYSTEMS)}.`,
  (n) => `We agreed to keep the dashboard percentages to one decimal place so the columns line up.`,
  (n) => `The contractor invoice for the accessibility audit arrived late, and finance paid it on the same day.`,
  (n) => `The flat design for the settings icons is done; the enterprise admin screens will follow the same style.`,
];

const TOPICS = [
  "Release train",
  "Hiring",
  "Incident follow-ups",
  "Roadmap check-in",
  "Support load",
  "Mobile performance",
  "Data retention",
  "Accessibility audit",
  "Pricing page experiments",
  "Office logistics",
  "Dependency upgrades",
  "Customer advisory board",
  "Observability",
  "Documentation",
];

const DECISIONS = [
  (n) => `${pick(n, FEATURES)} ships to everyone on ${dateOf(between(n, 1, 13), between(n, 0, 4))}.`,
  (n) => `${pick(n, TEAMS)} owns ${pick(n, SYSTEMS)} from now on.`,
  (n) => `Staging data is refreshed every ${pick(n, ["Monday", "Wednesday", "Friday"])} night.`,
  (n) => `Alerts on ${pick(n, SYSTEMS)} page only after ${between(n, 2, 10)} minutes of sustained errors.`,
  (n) => `The design system gets its own weekly office hour, run by ${pick(n, PEOPLE)}.`,
  (n) => `We keep the public API rate cap unchanged this quarter.`,
  (n) => `The retention window for raw logs is ${pick(n, ["14", "30", "90"])} days.`,
];

const ACTIONS = [
  (n) => `${pick(n, PEOPLE)}: write up the ${pick(n, BUGS)} investigation (due ${dateOf(between(n, 1, 13), 4)})`,
  (n) => `${pick(n, PEOPLE)}: schedule the ${pick(n, TOPICS).toLowerCase()} review`,
  (n) => `${pick(n, PEOPLE)}: draft the rollout plan for ${pick(n, FEATURES)}`,
  (n) => `${pick(n, PEOPLE)}: close out the overdue postmortem actions`,
  (n) => `${pick(n, PEOPLE)}: check capacity headroom on ${pick(n, SYSTEMS)}`,
];

/**
 * The late-fee rules, in the week they were recorded. `section` decides
 * where the text lands; discussion items get their own heading.
 */
const RULES = [
  {
    week: 1,
    section: "discussion",
    topic: "Collections follow-up",
    text: "Priya brought numbers from the collections review: roughly one invoice in nine is paid after its due date, and today nothing happens when that occurs. We agreed to start charging for it. The charge is 2% of the invoice amount, applied once, to invoices that are past due. Billing owns the implementation.",
  },
  { week: 1, section: "decisions", text: "Invoices paid after their due date get a one-time late fee of 2% of the invoice amount." },
  {
    week: 3,
    section: "discussion",
    topic: "Overdue charges",
    text: "Marco asked whether 2% is enough to change anyone's behavior and floated 3%. Support pushed back: most late payers are small customers who simply forgot. The proposal was rejected and the rate stays at 2%.",
  },
  { week: 4, section: "decisions", text: "Grace period for unpaid invoices: no late fee until an invoice is more than 7 days past its due date." },
  {
    week: 6,
    section: "updates",
    text: "Billing: after the pricing review, the surcharge on overdue invoices drops from 2% to 1.5% of the invoice amount, effective for every calculation from now on.",
  },
  {
    week: 7,
    section: "discussion",
    topic: "Large invoices",
    text: "Dana pointed out that a percentage charge on a six-figure invoice produces absurd numbers. Decision: the late fee on any single invoice never exceeds $40.00.",
  },
  { week: 9, section: "decisions", text: "Invoices of nonprofit customers are exempt from late fees." },
  {
    week: 10,
    section: "discussion",
    topic: "Payments in flight",
    text: "Support has had a steady stream of complaints from customers whose payment was in flight over a weekend. We are extending the grace period: a late fee now applies only once an invoice is more than 10 days past its due date, up from 7.",
  },
  {
    week: 11,
    section: "discussion",
    topic: "Collection costs",
    text: "Omar proposed raising the $40.00 ceiling on late fees to $60.00, since it barely covers what collections cost us. Finance wants a full quarter of data first, so this is deferred and the ceiling stays at $40.00.",
  },
  {
    week: 12,
    section: "decisions",
    text: "Enterprise customers are charged a flat $25.00 late fee instead of the percentage; the grace period still applies to them.",
  },
  {
    week: 13,
    section: "actions",
    text: "Billing: compute late fees in cents and round half up to the nearest cent (1.5% of $31.00 is $0.47); this is the final rule set for launch",
  },
];

/** Fisher-Yates with the seeded generator, so the order never depends on the engine's sort. */
function shuffle(next, list) {
  const out = [...list];
  for (let index = out.length - 1; index > 0; index -= 1) {
    const other = Math.floor(next() * (index + 1));
    [out[index], out[other]] = [out[other], out[index]];
  }
  return out;
}

function paragraph(next, sentences) {
  return Array.from({ length: sentences }, () => {
    const sentence = pick(next, SENTENCES)(next);
    return sentence[0].toUpperCase() + sentence.slice(1);
  }).join(" ");
}

function week(index) {
  const next = rng(9_000 + index);
  const date = dateOf(index);
  const rules = RULES.filter((rule) => rule.week === index);
  const attendees = shuffle(next, PEOPLE).slice(0, between(next, 6, 10));
  const updates = TEAMS.map((team) => `- **${team}:** ${paragraph(next, between(next, 3, 6))}`);
  const topics = [];
  const decisions = [];
  const actions = [];

  // Fill every section, then keep adding discussion until the file is long enough.
  for (let count = between(next, 5, 7); count > 0; count -= 1) decisions.push(`- ${pick(next, DECISIONS)(next)}`);
  for (let count = between(next, 5, 8); count > 0; count -= 1) actions.push(`- [ ] ${pick(next, ACTIONS)(next)}`);
  const size = () => updates.join("\n").length + topics.join("\n").length + decisions.join("\n").length + actions.join("\n").length;
  while (size() < TARGET_BYTES) {
    const paragraphs = Array.from({ length: between(next, 2, 4) }, () => paragraph(next, between(next, 4, 8)));
    topics.push(`### ${pick(next, TOPICS)}\n\n${paragraphs.join("\n\n")}`);
  }

  // Plant this week's rules at deterministic positions inside the filler.
  const insert = (list, item) => list.splice(between(next, 1, list.length - 1), 0, item);
  for (const rule of rules) {
    if (rule.section === "discussion") {
      insert(topics, `### ${rule.topic}\n\n${paragraph(next, 3)} ${rule.text} ${paragraph(next, 2)}\n\n${paragraph(next, 5)}`);
    } else if (rule.section === "decisions") {
      insert(decisions, `- ${rule.text}`);
    } else if (rule.section === "actions") {
      insert(actions, `- [ ] ${rule.text}`);
    } else {
      const position = TEAMS.indexOf("Billing");
      updates[position] = `- **Billing:** ${paragraph(next, 2)} ${rule.text.replace(/^Billing: /, "")} ${paragraph(next, 2)}`;
    }
  }

  return `# Weekly product sync, ${date}

Attendees: ${attendees.join(", ")}
Notes: ${pick(next, attendees)}

## Team updates

${updates.join("\n\n")}

## Discussion

${topics.join("\n\n")}

## Decisions

${decisions.join("\n")}

## Action items

${actions.join("\n")}
`;
}

const FEES = `/**
 * Late fee, in cents, for an invoice that is still unpaid on \`asOf\`.
 *
 * The rules were agreed in the weekly product syncs; see docs/meetings/.
 *
 * @param {{ amountCents: number, dueDate: string, customerType: "standard" | "nonprofit" | "enterprise" }} invoice
 *   \`amountCents\` is a non-negative integer and \`dueDate\` is YYYY-MM-DD.
 * @param {string} asOf - The day the fee is computed for, as YYYY-MM-DD.
 * @returns {number} The fee in cents.
 */
export function lateFee(invoice, asOf) {
  throw new Error("not implemented");
}
`;

/** Map of relative path to file content. */
export function meetingNotesFiles() {
  const files = {
    "package.json": `${JSON.stringify({ name: "billing-rules", private: true, type: "module", scripts: { test: "node --test test/" } }, null, 2)}\n`,
    "README.md": `# billing-rules

Billing rules shared by the invoicing jobs. Product decisions are recorded in
the weekly meeting notes under \`docs/meetings/\`, one file per week.
`,
    "src/fees.js": FEES,
  };
  for (let index = 0; index < WEEKS; index += 1) {
    files[`docs/meetings/${dateOf(index)}.md`] = week(index);
  }
  return files;
}
