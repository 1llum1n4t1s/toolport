# Native BGE test vectors

This fixture bounds debug test runtime. It contains outputs of Toolport's actual bundled Candle BGE encoder for the normalized public scale catalog, the small public search catalog and queries in dev.json, held_out.json (the old, public development set), dev-v2.json and search-eval/intents.json. Expected labels never enter the encoder. No external or blind set is included.

Each record contains a 32-byte SHA256(model asset digest + exact input text), then 384 little-endian f32 values. Document inputs use the production renderer and encoder. Query inputs include the production instruction prefix and are encoded in batches of at most eight. The asset digest binds the representation version, weights, tokenizer, config and notices.

Asset digest: `e5b38845f043257273ec89e20db563048cd032c505f6de74779ceaff69b65214`.

Only cfg(test) reads this file. Real production workers and search_bge_resources still run native inference. The resource probe explicitly bypasses cached query fixtures and memoization. Unit checks require complete fixture coverage, finite unit vectors and native sample cosine >= 0.99999. The existing Python parity, batch padding, lazy load and native cache tests continue running the actual encoder. Quality assertions and corpus labels are unchanged.

To regenerate, build the ignored `search_export_native_fixture` test with `--release --no-default-features --features test-support,search-static`, then run it with `SEARCH_NATIVE_CACHE` pointing to a disposable production content cache and `SEARCH_NATIVE_FIXTURE_OUTPUT` pointing to bge-native-vectors.bin. Missing document vectors are encoded natively; query vectors always use native inference. Keep TOOLPORT_SEARCH_BLIND_INTENTS unset.

Pinned fixture SHA256: `9bafacf56c33899640f99b348c452b3ff45546708bce17bf2b4d9ee8bf24d64f`; 3034 records, 4,757,312 bytes (2,118 document records plus 916 distinct public query records).
