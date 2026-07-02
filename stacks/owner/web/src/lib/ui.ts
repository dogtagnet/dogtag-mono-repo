import type { Category } from "./credential";

/** A friendly glyph per credential category (also used as the card avatar). */
export function categoryIcon(category: Category): string {
  switch (category) {
    case "Health":
      return "💉";
    case "Identity":
      return "🐕";
    case "Travel":
      return "✈️";
    case "Service":
      return "✂️";
    default:
      return "📄";
  }
}

/** The CSS custom property (accent colour) for a category. */
export function categoryVar(category: Category): string {
  return `var(--cat-${category.toLowerCase()})`;
}
