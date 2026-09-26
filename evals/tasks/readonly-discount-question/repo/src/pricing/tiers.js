import { goldRate } from "./gold.js";

const standard = () => 0;
const silver = (_customer, subtotal) => (subtotal >= 10_000 ? 5 : 0);

export function tierPolicy(tier) {
  switch (tier) {
    case "gold":
      return goldRate;
    case "silver":
      return silver;
    default:
      return standard;
  }
}
