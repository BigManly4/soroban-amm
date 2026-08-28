# Main branch protection

The `main` branch is protected and should accept changes only through reviewed pull requests.

Maintainers should configure the repository with the following settings:

- Require a pull request before merging, with at least one approving review.
- Require CODEOWNERS review for changes under `.github/`, `contracts/`, and release-sensitive paths.
- Require the `CI` and `Smoke Test` workflow checks to pass before merging.
- Dismiss stale approvals when new commits are pushed.
- Require branches to be up to date before merging.
- Require conversation resolution and prevent force pushes or branch deletion.
- Do not permit direct pushes to `main`, including administrator bypasses except for emergency recovery.

These settings encode the policy described in `CONTRIBUTING.md`; repository administrators should periodically verify that the configured rules have not drifted.
