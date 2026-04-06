import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import base from "./base.js";

export default [
  ...base,
  react.configs.flat.recommended,
  react.configs.flat["jsx-runtime"],
  {
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
    },
  },
];
