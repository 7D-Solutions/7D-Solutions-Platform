import type { Config } from "tailwindcss";
import preset from "@7d/tokens/preset";

const config: Config = {
  // Pull in all design tokens defined in @7d/tokens.
  presets: [preset as Config],
  content: [
    "./app/**/*.{ts,tsx}",
    "./components/**/*.{ts,tsx}",
    "./src/**/*.{ts,tsx}",
  ],
};

export default config;
