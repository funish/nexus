import { defineBasisConfig } from "@funish/basis";

export default defineBasisConfig({
  lint: {
    staged: {
      "*": "bun lint",
    },
    project: {
      check: "oxlint --fix --fix-suggestions --type-aware",
      format:
        "prettier --write --list-different . --ignore-path .gitignore . --plugin=@prettier/plugin-oxc",
    },
  },
  git: {
    hooks: {
      "pre-commit": "basis lint --staged",
      "commit-msg": "basis git --lint-commit",
    },
    commitMsg: {},
  },
});
