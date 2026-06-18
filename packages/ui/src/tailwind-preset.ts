import type { Config } from "tailwindcss";

/**
 * Shared Tailwind preset (impl §5.0). Maps semantic CSS-variable tokens (defined in `styles.css`,
 * with light + `.dark` palettes) onto Tailwind color utilities. Portals extend this preset; their
 * components reference ONLY these semantic names (`bg-surface`, `text-onPrimary`, `border-border`…).
 *
 * Uses the `<alpha-value>` placeholder so opacity modifiers (`bg-primary/90`) work while keeping
 * the config a plain serializable object.
 */
function token(variable: string): string {
  return `hsl(var(${variable}) / <alpha-value>)`;
}

const preset: Partial<Config> = {
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        background: token("--color-background"),
        surface: {
          DEFAULT: token("--color-surface"),
          muted: token("--color-surface-muted"),
        },
        onSurface: token("--color-onSurface"),
        primary: {
          DEFAULT: token("--color-primary"),
        },
        onPrimary: token("--color-onPrimary"),
        secondary: {
          DEFAULT: token("--color-secondary"),
        },
        onSecondary: token("--color-onSecondary"),
        sidebar: {
          DEFAULT: token("--color-sidebar"),
          muted: token("--color-sidebar-muted"),
          active: token("--color-sidebar-active"),
        },
        onSidebar: token("--color-onSidebar"),
        onSidebarMuted: token("--color-onSidebarMuted"),
        muted: token("--color-muted"),
        border: token("--color-border"),
        input: token("--color-input"),
        ring: token("--color-ring"),
        success: {
          DEFAULT: token("--color-success"),
        },
        onSuccess: token("--color-onSuccess"),
        warning: {
          DEFAULT: token("--color-warning"),
        },
        onWarning: token("--color-onWarning"),
        danger: {
          DEFAULT: token("--color-danger"),
        },
        onDanger: token("--color-onDanger"),
        accent: {
          DEFAULT: token("--color-accent"),
        },
        onAccent: token("--color-onAccent"),
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
      },
      fontFamily: {
        sans: [
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "Segoe UI",
          "Roboto",
          "Helvetica Neue",
          "Arial",
          "sans-serif",
        ],
      },
    },
  },
};

export default preset;
