# JCS test vectors

`vectors.json` is a list of `[input, canonical]` pairs for RFC 8785 (JSON
Canonicalization Scheme). Consent tokens hash the canonical form of the value
delivered, so every implementation (the relay and each SDK) must produce the
same bytes for these inputs.

- The first two cases are the examples in RFC 8785 §3.2.4 and §3.2.3.
- The rest are the reference implementation's test data:
  https://github.com/cyberphone/json-canonicalization/tree/dc406ceaf94b5fa554fcabb92c091089c2357e83/testdata
  (Apache-2.0).

They were taken from the `serde_json_canonicalizer` 0.4.1 crate, which ships
both.
