import nextPlugin from "@next/eslint-plugin-next";
import reactConfig from "./react.js";

export default [
  ...reactConfig,
  {
    plugins: { "@next/next": nextPlugin },
    rules: {
      ...nextPlugin.configs.recommended.rules,
      ...nextPlugin.configs["core-web-vitals"].rules,
    },
  },
];
