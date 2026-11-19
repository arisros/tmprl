# Changelog fragments

A pull request that changes what a user sees adds its changelog entry here, as a file of its
own, instead of editing `CHANGELOG.md`. Two open pull requests then never conflict over the
top of `## Unreleased`.

- Name the file after the branch: `feat-worker-info.md`.
- Write one or more bullets, for a user, with a bold lead like the ones in `CHANGELOG.md`:

  ```markdown
  - **Payloads read like `jq -C`**: keys, strings and numbers each in their own colour.
  ```

The release PR runs `scripts/changelog.py promote`, which moves every fragment under the new
version's heading and deletes the files. A release candidate leaves them here.
`scripts/check-docs.sh` fails on a fragment whose first line is not a bullet.
