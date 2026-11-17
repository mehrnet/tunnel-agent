import js from "@eslint/js";
import importX from "eslint-plugin-import-x";
import solid from "eslint-plugin-solid";
import sonarjs from "eslint-plugin-sonarjs";
import tseslint from "typescript-eslint";

const commonRules = {
  complexity: ["error", 25],
  "max-depth": ["error", 4],
  "max-lines": ["error", { max: 300, skipBlankLines: true, skipComments: true }],
  "max-lines-per-function": ["error", { max: 80, skipBlankLines: true, skipComments: true, IIFEs: true }],
  "max-params": ["error", 7],
  "max-statements": ["error", 40],
  "no-duplicate-imports": "error",
  "import-x/no-duplicates": "error",
  "sonarjs/cognitive-complexity": ["error", 25],
  "sonarjs/no-duplicate-string": ["error", { threshold: 4 }],
  "sonarjs/no-identical-functions": "error",
  "@typescript-eslint/no-unused-vars": ["error", {
    argsIgnorePattern: "^_",
    caughtErrorsIgnorePattern: "^_",
    varsIgnorePattern: "^_",
  }],
};

export default tseslint.config(
  { ignores: ["node_modules/**", ".wrangler/**", "web-dist/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.ts", "web/**/*.{ts,tsx}"],
    plugins: { "import-x": importX, sonarjs },
    rules: commonRules,
  },
  {
    files: ["web/**/*.{ts,tsx}"],
    plugins: { solid },
    rules: solid.configs["flat/recommended"].rules,
  },
);
