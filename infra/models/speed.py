#!/usr/bin/env python3
"""Single-request speed of an OpenAI-compatible model server (docs/serving.md §2).

Run it while nothing else uses the server. Three requests, twice: a short prompt with 400 output
tokens, an ~11k-token prompt with 300 output tokens, and the same long prompt again (its start is
now cached). Prints time to the first token and output tokens per second.

    python3 infra/models/speed.py --base http://127.0.0.1:8000 --model nvidia/Muse-Glimmer-30B-NVFP4
"""
import argparse
import json
import random
import time
import urllib.request


def run(base, model, prompt, max_tokens):
    body = {"model": model, "messages": [{"role": "user", "content": prompt}], "max_tokens": max_tokens,
            "stream": True, "stream_options": {"include_usage": True}}
    req = urllib.request.Request(base.rstrip("/") + "/v1/chat/completions", json.dumps(body).encode(),
                                 {"Content-Type": "application/json"})
    start = time.time()
    first = None
    usage = None
    with urllib.request.urlopen(req, timeout=300) as r:
        for line in r:
            line = line.decode().strip()
            if not line.startswith("data: ") or line == "data: [DONE]":
                continue
            chunk = json.loads(line[6:])
            usage = chunk.get("usage") or usage
            choices = chunk.get("choices") or []
            delta = choices[0]["delta"] if choices else {}
            if first is None and (delta.get("content") or delta.get("reasoning_content") or delta.get("reasoning")):
                first = time.time()
    end = time.time()
    out = usage["completion_tokens"]
    print(f"prompt={usage['prompt_tokens']} out={out} first_token={first - start:.2f}s "
          f"decode={(out - 1) / (end - first):.1f} tok/s total={end - start:.1f}s")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--base", required=True)
    ap.add_argument("--model", required=True)
    args = ap.parse_args()
    rng = random.Random(1)
    words = "order product variant stock price coupon member tier review refund".split()
    long_prompt = " ".join(rng.choices(words, k=11000)) + "\nsummarize the above in detail"
    for _ in range(2):
        run(args.base, args.model, "write a long story about a store, at least 400 words", 400)
        run(args.base, args.model, long_prompt, 300)
        run(args.base, args.model, long_prompt, 300)


if __name__ == "__main__":
    main()
