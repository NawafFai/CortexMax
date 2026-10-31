# GitHub Actions in this fork

`workflows/release.yml` is the only active workflow: it builds the unsigned
macOS (Apple Silicon) `.dmg` and Windows (x64) `.exe` installers, and on a
`v*` tag attaches them to a GitHub Release.

The upstream Buzz workflows are kept, inactive, in `workflows-upstream-disabled/`
for reference. To enable one, move it back into `workflows/` and provide the
secrets it expects.
