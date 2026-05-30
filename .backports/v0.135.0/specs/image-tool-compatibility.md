# Image Tool Compatibility

## Upstream References

- `675cb1afb` - clarify `view_image` tool description
- `46391f7ef` - remove plain image wrapper spans
- `dc4e54d06` - restore legacy image detail values

Classification: `manual-port`.

Reason: these patches touch generated schemas and broad test fixtures that are
not present locally, but their protocol/tool behavior survives.

## Goal

Port image-input compatibility changes without restoring removed fixtures.

## Behavior

View image description:

- replace the old path-only guidance with the upstream visual-inspection wording

Plain image wrappers:

- serialize `UserInput::Image` directly as an `input_image` content item
- keep local-image labels and legacy wrapper parsing behavior

Legacy image detail:

- restore `ImageDetail::Auto` and `ImageDetail::Low` for persisted history
  compatibility
- keep new default behavior at `high`
- treat `auto`, `low`, and `high` as resize-to-fit for local image loading
- preserve non-original detail values through tool/code-mode outputs where
  those surfaces exist locally

## Tests

Port or adapt upstream coverage only where it maps to surviving behavior:

- `675cb1afb`: `view_image` tool description matches upstream wording
- `46391f7ef`: remote image user input serializes without wrapper text spans
- `46391f7ef`: local image labels and legacy wrapper parsing still work
- `dc4e54d06`: `auto` and `low` image detail values deserialize and round-trip
- `dc4e54d06`: non-original image detail values are preserved by local helpers

Do not copy generated schema fixtures directly; regenerate local schemas if the
local workflow requires it.

## Validation

Run after implementation:

- `cargo +stable fmt`
- `cargo +stable test -p codex-protocol`
- `cargo +stable test -p codex-tools`
- `cargo +stable test -p codex-core view_image`
