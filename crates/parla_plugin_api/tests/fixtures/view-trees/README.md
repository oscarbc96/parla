# View-tree test vectors

Shared by every implementation of the view rules in `wit/view.wit`: the
guest SDK (`plugins/sdk-rust/tests/view_vectors.rs`) and the host
validator (`plugin_host::view`, plan M8 Task E1). Each `*.json` file is one
case:

- `verdict`: `"ok"`, `"invalid"` or `"limit"`.
- `rule`: the number of the rule in `view.wit` the tree breaks (absent for
  `ok`).
- `size`: for `ok` trees, the tree's size in bytes by rule 8's measure.
- `nodes`: the flat node list. Each node has `parent` (index or `null`), an
  optional `id`, and `kind`: a string for a kind without a payload
  (`"column"`), or a one-key object `{ "<kind>": payload }` with the WIT
  names (`"text": [{ "text": "Hi" }]`, `"button": { "label": "Go" }`).
  Record fields use the WIT kebab-case names; a missing `bool` is `false`,
  a missing `option` is absent, a missing list is empty, and enums are
  their WIT names (`"tone": "accent"`).
