export const seed = {
  users: [
    { id: "u1", name: "Ada Lovelace", email: "ada@example.test" },
    { id: "u2", name: "Grace Hopper", email: "grace@example.test" },
    { id: "u3", name: "Alan Turing", email: "alan@example.test" },
  ],
  projects: [
    { id: "p1", name: "Engine", owner: "u1", archived: false, sharedWith: [] },
    { id: "p2", name: "Compiler", owner: "u2", archived: false, sharedWith: ["u1"] },
    { id: "p3", name: "Analytical notes", owner: "u1", archived: false, sharedWith: [] },
    { id: "p4", name: "Bombe", owner: "u3", archived: true, sharedWith: [] },
  ],
};
