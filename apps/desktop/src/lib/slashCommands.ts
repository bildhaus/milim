export type SlashCommand = {
  id: string;
  label: string;
  hint: string;
  group: "Commands" | "Settings";
  placeholder?: string;
  /**
   * How the command palette runs this command: "run" executes it without an
   * argument, "prefill" puts `/<id> ` in the composer for an argument.
   */
  palette: "run" | "prefill";
};

export const SLASH_COMMANDS: SlashCommand[] = [
  { id: "plan", label: "Plan mode", hint: "Toggle read-only planning", group: "Commands", placeholder: "/plan build feature X", palette: "run" },
  { id: "goal", label: "Goal", hint: "Make the next prompt an autonomous goal", group: "Commands", placeholder: "/goal build feature X", palette: "run" },
  { id: "model", label: "Model", hint: "Set the thread model", group: "Settings", placeholder: "/model llama3.2", palette: "prefill" },
  { id: "folder", label: "Folder", hint: "Set or pick a working folder", group: "Settings", placeholder: "/folder C:\\project", palette: "run" },
  { id: "sandbox", label: "Docker sandbox on", hint: "Enable Docker sandbox tools", group: "Settings", palette: "run" },
  { id: "nosandbox", label: "Docker sandbox off", hint: "Disable Docker sandbox tools", group: "Settings", palette: "run" },
  { id: "computer", label: "Computer on", hint: "Enable computer-use tools", group: "Settings", palette: "run" },
  { id: "nocomputer", label: "Computer off", hint: "Disable computer-use tools", group: "Settings", palette: "run" },
  { id: "memory", label: "Memory on", hint: "Enable scoped memories", group: "Settings", palette: "run" },
  { id: "nomemory", label: "Memory off", hint: "Disable scoped memories", group: "Settings", palette: "run" },
  { id: "privacy", label: "Privacy", hint: "Set privacy gate", group: "Settings", placeholder: "/privacy redact", palette: "prefill" },
  { id: "approval", label: "Approval", hint: "Set tool approval mode", group: "Settings", placeholder: "/approval guarded", palette: "prefill" },
  { id: "agent", label: "Agent", hint: "Set active agent or none", group: "Settings", placeholder: "/agent none", palette: "prefill" },
  { id: "compact", label: "Compact thread", hint: "Summarize prior context into a fresh checkpoint", group: "Commands", palette: "run" },
  { id: "export", label: "Export chat", hint: "Download this thread as JSON", group: "Commands", palette: "run" },
  { id: "import", label: "Import chat", hint: "Import a Milim thread JSON file", group: "Commands", palette: "run" },
  { id: "clear", label: "New chat", hint: "Start a fresh chat with current settings", group: "Commands", palette: "run" },
];
