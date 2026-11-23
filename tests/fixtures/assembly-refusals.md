# Assembly refusal fixture

This fixture preserves the timing of a 30-minute replay built from the English,
Hindi and Telugu test splits of [Google FLEURS][fleurs] by Alexis Conneau and the
FLEURS authors. The source is licensed under [CC BY 4.0][license]. The replay
converted audio to mono 16 kHz PCM16 and concatenated it with silence padding.
The fixture contains no audio. All baseline and result text is synthetic.

The plan retains all 294 ranges, their language, sample ownership, context,
piece identity and model pins. WAV paths are replaced with `synthetic.wav` and
range identities are recomputed from the serialized production Range. Assembly
never opens those paths. The original plan revision identifies the saved input.

Baseline sentence times, the complete speech coverage, routing uncertainty,
piece bounds and three explicit no-speech outcomes are retained. Baseline and
coverage IDs are short synthetic names. Result-only sample fields and public
recognition text are omitted because the range-level assembler does not read them.
The existing captured baseline no-speech reply fixture stays beside the tests.

The test helper writes 291 synthetic results with matching SHA-256 receipts and
three canonical fallback files. The tests call the production receipt reader,
assembler, immutable publisher and readback. The full guard test calls
`assemble_saved`.

`assembly-refusals.json` is 181,870 bytes.

[fleurs]: https://huggingface.co/datasets/google/fleurs/tree/70bb2e84b976b7e960aa89f1c648e09c59f894dd
[license]: https://creativecommons.org/licenses/by/4.0/
