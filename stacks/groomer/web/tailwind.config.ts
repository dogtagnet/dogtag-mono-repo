import preset from "@dogtag/ui/tailwind-preset";
import type { Config } from "tailwindcss";

const config: Config = {
  presets: [preset as Config],
  content: [
    "./index.html",
    "./src/**/*.{ts,tsx}",
    // scan the shared UI source so its semantic-token classes are emitted
    "../../../packages/ui/src/**/*.{ts,tsx}",
  ],
  theme: { extend: {} },
  plugins: [],
};

export default config;
