# Native BGE test vectors

This fixture bounds debug test runtime. It contains outputs of Toolport's actual bundled Candle BGE encoder for the normalized public scale catalog, the small public search catalog and queries in dev.json, held_out.json (the old, public development set), dev-v2.json and search-eval/intents.json. Expected labels never enter the encoder. No external or blind set is included.

Each record contains a 32-byte SHA256(model asset digest + exact input text), then 384 little-endian f32 values. Document inputs use the production renderer and encoder, with schema parameters in canonical sorted key order (representation bge-sorted-cls-v2). Query inputs include the production instruction prefix and are encoded in batches of at most eight. The asset digest binds the representation version, weights, tokenizer, config and notices.

Asset digest: `70e9e73a54c1b97e2252c659341fd7612052a62fe04917ab9d1f67a6331622cc`.

Only cfg(test) reads this file. Real production workers and search_bge_resources still run native inference. The resource probe explicitly bypasses cached query fixtures and memoization. Unit checks require complete fixture coverage, finite unit vectors and native sample cosine >= 0.99999. The existing Python parity, batch padding, lazy load and native cache tests continue running the actual encoder. Quality assertions and corpus labels are unchanged.

To regenerate, build the ignored `search_export_native_fixture` test with `--release --no-default-features --features test-support,search-static`, then run it with `SEARCH_NATIVE_CACHE` pointing to a disposable production content cache and `SEARCH_NATIVE_FIXTURE_OUTPUT` pointing to bge-native-vectors.bin. Missing document vectors are encoded natively; query vectors always use native inference. Keep TOOLPORT_SEARCH_BLIND_INTENTS unset.

Pinned fixture SHA256: `0a85bea96a40976ff2b911bc9421454e62106439c341106d4bfb5a88bd978022`; 3034 records, 4,757,312 bytes (2,118 document records plus 916 distinct public query records).
