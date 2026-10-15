# Offline BGE search embeddings

Source: [BAAI/bge-small-en-v1.5](https://huggingface.co/BAAI/bge-small-en-v1.5),
revision `5c38ec7c405ec4b44b94cc5a9bb96e735b38267a`, MIT.
384 dimensions, CLS pooling, L2 normalization. No Toolport labels train the model.
The full MIT notice and tokenizer license remain embedded in the gateway binary.

## Runtime decision

Candle 0.11 CPU is the brief's portable fallback. We store f16 safetensors and
promote weights to f32 for inference: BERT's layer-norm epsilon `1e-12` produces
non-finite vectors with pure f16 CPU arithmetic. On 20 round-2 documents, the
fallback's minimum cosine against the Python fp32 prototype was `0.9999996`.

ORT rc10 / 1.22 linked statically on Linux and has static archives for all four
shipped targets, but failed the required parity gate against Python ORT 1.31:
minimum cosine `0.9972795` (original int8), `0.99719054` (per-channel int8).
ORT rc14 / 1.31 passed (`0.99999934`) but has no Intel macOS prebuilt archive;
rc11 / 1.23, rc12 / 1.24 and rc13 / 1.28 also lack that target. Windows prebuilt archives
add a DirectML DLL dependency. No ORT DLL is shipped or loaded by this fallback.

The prescribed per-channel candidate uses ONNX Runtime `quantize_dynamic`,
QInt8, `per_channel=True`, `reduce_range=False`. Its standalone dev-v3 top1 was
175/255 (68.63%), versus source fp32 172/255 (67.45%); RRF top1 was 178/255.
A per-channel reduce-range parity check also failed on ORT rc10 (minimum
cosine `0.99776345`); it was not rescored or selected. The selected int8
candidate is retained in external evidence, not
bundled as a second semantic path.

## Reproduction and hashes

Run on devbox with NumPy and safetensors installed, from a local download of the
pinned revision:

```bash
python3 scripts/prepare-search-model.py /path/to/bge-small-en-v1.5 src-tauri/assets/search
sha256sum src-tauri/assets/search/model-f16.safetensors src-tauri/assets/search/tokenizer.json src-tauri/assets/search/config.json
```

The exact int8 export used the pinned source ONNX model:

```python
from onnxruntime.quantization import quantize_dynamic, QuantType
quantize_dynamic('bge-int8/onnx/model.onnx', 'model.int8.onnx',
                 weight_type=QuantType.QInt8, per_channel=True, reduce_range=False)
```

| Asset                               | SHA256                                                             |
| ----------------------------------- | ------------------------------------------------------------------ |
| Source fp32 safetensors             | `3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad` |
| Bundled f16 safetensors             | `1e2b41d9cd9cf8918c563847f68af3ac4c46085b60fd84fff985197057659cfa` |
| Tokenizer                           | `d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66` |
| Config                              | `094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750` |
| Source fp32 ONNX                    | `828e1496d7fabb79cfa4dcd84fa38625c0d3d21da474a00f08db0f559940cf35` |
| Per-channel int8 ONNX (not bundled) | `f8b2217838ea27564f870f96e377cb6e5ca0fa37dec9599cf305d5de011d6b7f` |

## Shipping behavior

The session loads on first search, never at gateway startup or tools/list.
One worker renders provider, humanized tool name, full description and schema
parameters in canonical sorted key order (160 description characters per parameter).
Documents use the round-2 tokenizer/decode projection with 254 content tokens;
final inputs have at most 256 tokens. Queries prepend
`Represent this sentence for searching relevant passages: `.
Catalog batches have at most eight documents. CPU work uses a private pool of
at most two threads. Disk entries use the model/representation digest plus the
rendered document hash, with the existing 32 MiB / 14 day limits. Searches use
lexical answers while background vectors are pending or the encoder is busy/unavailable.
Queries never queue behind catalog inference. A busy query reserves a bounded
200 ms opportunity between batches for the next query; the worker resumes when
a query acquires the session or the window expires. Search traces record the
ranker actually used. Failed model loads back off for 60 seconds and log once;
unavailable queries do not participate in fusion. Failed catalog jobs retry after
60 and 120 seconds. After three failed attempts they log once and retain their
snapshot; the next search after a 240-second cooldown can start another retry
cycle. Only a complete successful catalog is published.
The session unloads after
ten minutes idle. Unload also releases the tokenizer and joins the private CPU
pool workers so their retained packing buffers are freed. Immutable catalog
vectors remain resident.
Linux/glibc trims freed allocator pages after an actual unload and after a catalog
job drops its rendered documents and inference buffers.

Fusion uses RRF k=10, weights 1/1, while preserving literal-name and named-provider
behavior. Semantic-only candidates require cosine at least 0.60 and at least 85%
of the visible maximum. Non-name semantic confidence uses one fixed round rule:
cosine at least 0.75 and top-to-runner-up cosine margin at least 0.02. Existing
absolute lexical job evidence and 90% coverage remain required for confidence;
cosine can resolve a lexical tie or name-shape uncertainty. No threshold sweeps.
Existing user-configured endpoint search remains supported.

`search-static` is required for shipped gateways. Test-only lexical/prototype
controls are absent from production binaries. External integration evidence and
runtime-spike commands live outside the repository.
