import { defineBasisConfig } from "@funish/basis/config";

export default defineBasisConfig({
  lint: {
    config: ["--fix", "--fix-suggestions", "--type-aware", "--type-check"],
  },
  fmt: {
    config: ["--write", "."],
  },
  git: {
    hooks: {
      "pre-commit": "bun basis git staged",
      "commit-msg": "bun basis git lint-commit",
    },
    staged: {
      rules: {
        "**/*.{ts,tsx,js,jsx}": "basis lint",
        "**/*.{json,md,yml,yaml}": "basis fmt",
      },
    },
  },
});
