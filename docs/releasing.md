# Releases

The first planned release is `v0.1.0`. Keep its changelog entries under
Unreleased until publication. No release tag or hosted build is needed while
the repository is private.

## Prepare locally

Run `bash scripts/check.sh native`, then `bash scripts/package-macos.sh` on the
Mac used to produce the release. The packaging script builds the release app
for that Mac's architecture, checks the binary architecture, and creates a ZIP
plus `SHA256SUMS.txt` under `target/dist/v<version>/<architecture>/`.
Bundle versions come from Cargo metadata. The current script produces unsigned,
unnotarized packages for local testing. It does not implement the signed public
installation path described as planned in the README.

Before offering signed downloads, configure an Apple Developer ID identity and
notarization credentials, sign the app, submit it for notarization, staple the
accepted ticket, and create the archive from that finalized app. Verify signing,
notarization, and a normal first launch on a separate Mac. Do not publish the
current unsigned ZIP as a signed release. Keep credentials outside the repository.

Extract the archive into a temporary folder, verify its checksum, and open the
extracted app with an offline fixture. Check theme selection, file comments,
line comments, search, and scrolling. Build Intel packages on an Intel Mac;
do not label an Apple silicon binary as universal. Linux packages need desktop
validation before being offered.

## Publish after the repository becomes public

1. Confirm the repository is public. Keep Actions disabled until then; this
   repository currently has no build or release workflow.
2. Move the first release's Unreleased entries under `0.1.0` with the actual
   publication date. Leave an empty Unreleased section for subsequent work.
3. Amend the initial commit if retaining single-commit history, and finish the
   guarded push before creating the release tag.
4. Tag the tested commit as `v0.1.0`. Create a draft GitHub release, attach the
   tested ZIP and checksum file, and include the changelog, supported architecture,
   macOS minimum version, and signing status in its notes.
5. If publishing multiple architectures, combine their checksum entries into
   one `SHA256SUMS.txt`. Verify each archive against its entry.
6. Publish the release after reviewing the assets. Update the README's pending
   download notice to point to the published release and list its actual assets.

Do not rewrite a published release tag. Any future automated release workflow
must be manually triggered and skip its jobs when `github.event.repository.private`
is true. Adding that guard is not a substitute for keeping Actions disabled
while this repository is private.
