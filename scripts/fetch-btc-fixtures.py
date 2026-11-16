#!/usr/bin/env python3
"""Fetch real Bitcoin mainnet data used as test vectors for the light client.

Public chain data only (block headers, a few transactions of one block and their Merkle
paths), taken from the Blockstream Esplora API. Output: a flat `key=value` text file.
"""
import json, sys, time, urllib.request

API = "https://blockstream.info/api"

def get(path, raw=False):
    for attempt in range(5):
        try:
            req = urllib.request.Request(API + path, headers={"User-Agent": "fixture-fetch"})
            with urllib.request.urlopen(req, timeout=30) as r:
                data = r.read().decode()
            return data if raw else json.loads(data)
        except Exception as e:  # noqa
            time.sleep(1 + attempt)
            err = e
    raise err

def header_at(height):
    h = get(f"/block-height/{height}", raw=True).strip()
    return h, get(f"/block/{h}/header", raw=True).strip()

out = {}
RETARGET = 2016 * 417  # 840672: first block of a new difficulty period
PERIOD_START = RETARGET - 2016
out["retarget_height"] = RETARGET
h, hdr = header_at(PERIOD_START)
out["period_start_hash"], out["period_start_header"] = h, hdr
# A run of consecutive headers crossing the boundary. It starts sixteen blocks before it, so
# that the block used for the receipt (RETARGET + 1) has the eleven predecessors that its
# median time past is computed from, with room for a deal to be opened in between.
first = RETARGET - 16
out["run_first_height"] = first
run = []
for height in range(first, RETARGET + 8):
    h, hdr = header_at(height)
    run.append(hdr)
    if height == first:
        out["run_first_hash"] = h
        info = get(f"/block/{h}")
        out["run_first_chainwork_note"] = "see test: chainwork is supplied by the checkpoint"
out["run_headers"] = ",".join(run)

# One transaction with its Merkle path, from the block right after the boundary.
bh, _ = header_at(RETARGET + 1)
txids = get(f"/block/{bh}/txids")
out["proof_block_hash"] = bh
out["proof_block_height"] = RETARGET + 1
out["proof_block_tx_count"] = len(txids)
# "cb" is the block's coinbase: every receipt carries it, to fix the depth of the Merkle tree.
for label, pos in (("cb", 0), ("a", 1), ("b", len(txids) - 1), ("c", len(txids) // 2)):
    txid = txids[pos]
    proof = get(f"/tx/{txid}/merkle-proof")
    assert proof["pos"] == pos
    out[f"proof_{label}_txid"] = txid
    out[f"proof_{label}_pos"] = pos
    out[f"proof_{label}_merkle"] = ",".join(proof["merkle"])
    out[f"proof_{label}_tx_hex"] = get(f"/tx/{txid}/hex", raw=True).strip()

with open(sys.argv[1], "w") as f:
    for k, v in out.items():
        f.write(f"{k}={v}\n")
print("wrote", sys.argv[1], "with", len(out), "entries")
