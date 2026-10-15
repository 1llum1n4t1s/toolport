#!/usr/bin/env python3
"""Store the pinned BGE fallback as f16 safetensors; never train on search labels.

Requires numpy and safetensors. Input is a local download of the README revision.
CPU inference promotes the stored weights to f32 for BERT layer-norm stability.
"""
import hashlib
import pathlib
import shutil
import sys

import numpy as np
from safetensors.numpy import load_file, save_file

source, output = map(pathlib.Path, sys.argv[1:])
output.mkdir(parents=True, exist_ok=True)
weights = load_file(str(source / "model.safetensors"))
assert weights["embeddings.word_embeddings.weight"].shape == (30522, 384)
save_file(
    {name: tensor.astype(np.float16) for name, tensor in weights.items()},
    str(output / "model-f16.safetensors"),
)
for name in ["tokenizer.json", "config.json"]:
    shutil.copyfile(source / name, output / name)
for path in [source / "model.safetensors", output / "model-f16.safetensors", output / "tokenizer.json", output / "config.json"]:
    print(hashlib.sha256(path.read_bytes()).hexdigest(), path.name)
