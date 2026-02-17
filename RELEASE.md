This release workflow can be performed manually, or by a human overseeing an agent. For agent use, bear in mind the need for elevated permissions for signed commits or `cargo` release workflows.

1. Inspect the most recent nightly build on GitHub (canary for any obvious issues in `main`).
2. Ensure there are no uncommitted changes in `main` and that it is up-to-date.
3. Check for pending PRs. If they are open, make sure that choice is intentional.
4. Branch a `release/vX.Y.Z` branch from `main`, where `X.Y.Z` is the bumped version of the last release.
   Follow SemVer practices; unless there is significant new functionality, a patch release is typically appropriate.
5. On that branch, run `cargo update` to ensure all dependencies are up-to-date within their SemVer constraints.
6. Run automated tests and perform final pre-release checks: smoke checks and basic functionality verification.
7. Commit the lockfile changes, if any.
8. Update the changelog in "Keep a changelog" format with all unreleased changes since the last tag.
9. Commit the changelog update.
10. Run `cargo release X.Y.Z`, where `X.Y.Z` is the target release version chosen in step 4.
11. If no issues in dry run, run with `--execute`.
12. The release is now tagged. Create a follow-up commit to update Cargo.toml and Cargo.lock to the next
    patch `-dev` version (increment `Z` by 1, then append `-dev`; for example `0.7.3-dev` -> `0.7.4-dev`).
13. Commit and push the version bump.
14. Merge the release branch into main if all checks pass.
15. Monitor the GitHub Actions to ensure the publish workflow for release binaries and crates.io works as expected.
    Inspect the release on GitHub to ensure all assets and the relevant changelog entry are present.
