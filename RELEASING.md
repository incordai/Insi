# Releasing insi

Nothing here runs by itself past a draft: each registry is published by a
person, from a release they have looked at.

## 1. Versions

One version everywhere, or the release workflow refuses the tag:

- `Cargo.toml` — `[workspace.package] version` and the five `insi-*`
  entries under `[workspace.dependencies]`
- `packaging/npm/package.json` — `version`
- `packaging/pip/pyproject.toml` — `version`
- `packaging/pip/src/insi_cli/__init__.py` — `__version__`

## 2. Tag

```sh
git tag v0.1.0 && git push origin v0.1.0
```

`.github/workflows/release.yml` builds `insi` for x86_64/aarch64 Linux,
x86_64/aarch64 macOS and x86_64 Windows, and drafts the GitHub release
`v0.1.0` with `insi-v0.1.0-<target>.tar.gz` for each and `checksums.json`.

## 3. Look, then publish the draft

Download one archive, run its `insi --version`, compare its SHA-256 with
`checksums.json`. Publish the release on GitHub: the npm and pip packages
download from it.

## 4. Registries

```sh
cargo publish --workspace                    # crates.io: insi-core … insi, in order

cp checksums.json packaging/npm/             # the digests each wrapper pins
cp checksums.json packaging/pip/src/insi_cli/

(cd packaging/npm && npm test && npm publish)
(cd packaging/pip && python test_insi_cli.py && python -m build && python -m twine upload dist/*)
```

A wrapper without `checksums.json` refuses to install anything, so a
package published without it fails loudly rather than running an unchecked
download. `cargo publish --workspace --dry-run` checks the crates without
uploading.
