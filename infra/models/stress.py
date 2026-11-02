#!/usr/bin/env python3
"""Agent-shaped load against an OpenAI-compatible model server, to find crashes (docs/serving.md §3).

Each long worker plays a chat the way the agent does: a long shared system prompt, 40 tool
definitions half of the time, a JSON schema on some requests, streaming on some, and the
conversation growing turn by turn so that every turn reuses the cached start. Each short worker does
the same with a short system prompt. After every chat a worker sends a tiny identical ping, like a
health check. Requests that fail are logged and the worker starts a new chat.

    python3 infra/models/stress.py --base http://127.0.0.1:8000 --model nvidia/Muse-Glimmer-30B-NVFP4 \\
        --minutes 40 --long 8 --short 8 --log stress.log

The script only makes load. Whether the server died is read on the server: the container's restart
count or start time, and `journalctl -k | grep Xid`. Standard library only.
"""
import argparse
import json
import random
import threading
import time
import urllib.request

WORDS = ("order product variant stock price coupon member tier review refund return shipment "
         "category revenue campaign customer warehouse invoice size color season discount").split()
SCHEMA = {"type": "object", "properties": {"answer": {"type": "string"}, "confidence": {"type": "number"}},
          "required": ["answer", "confidence"]}


def tool(rng, i):
    props = {f"p{j}_{rng.choice(WORDS)}": {"type": rng.choice(["string", "integer", "boolean"]),
                                           "description": " ".join(rng.choices(WORDS, k=12))}
             for j in range(6)}
    return {"type": "function", "function": {
        "name": f"tool_{i}_{rng.choice(WORDS)}", "description": " ".join(rng.choices(WORDS, k=40)),
        "parameters": {"type": "object", "properties": props, "required": list(props)[:2]}}}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--base", required=True, help="server root, e.g. http://127.0.0.1:8000")
    ap.add_argument("--model", required=True)
    ap.add_argument("--minutes", type=float, default=40)
    ap.add_argument("--long", type=int, default=8, help="workers with a ~10k-token system prompt")
    ap.add_argument("--short", type=int, default=8, help="workers with a short system prompt")
    ap.add_argument("--log", default="stress.log")
    args = ap.parse_args()

    rng = random.Random(7)
    tools = [tool(rng, i) for i in range(40)]
    systems = {"long": "You are a store operations assistant. " + " ".join(rng.choices(WORDS, k=2500)),
               "short": "You are a store operations assistant. " + " ".join(rng.choices(WORDS, k=60))}
    lock = threading.Lock()
    stats = {"ok": 0, "err": 0}
    stop_at = time.time() + args.minutes * 60

    def log(msg):
        with lock, open(args.log, "a") as f:
            f.write(time.strftime("%H:%M:%S ") + msg + "\n")

    def call(body):
        req = urllib.request.Request(args.base.rstrip("/") + "/v1/chat/completions",
                                     json.dumps(body).encode(), {"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=180) as r:
            raw = r.read().decode()
        if body.get("stream"):
            return ""
        msg = json.loads(raw)["choices"][0]["message"]
        return msg.get("content") or json.dumps(msg.get("tool_calls") or "")[:400]

    def count(ok):
        with lock:
            stats["ok" if ok else "err"] += 1

    def worker(name, system, seed):
        wrng = random.Random(seed)
        while time.time() < stop_at:
            msgs = [{"role": "system", "content": system}]
            for _ in range(wrng.randint(2, 7)):
                if time.time() >= stop_at:
                    break
                msgs.append({"role": "user", "content": " ".join(wrng.choices(WORDS, k=wrng.randint(5, 60))) + "?"})
                body = {"model": args.model, "messages": msgs, "max_tokens": wrng.choice([8, 64, 200, 400])}
                kind = wrng.random()
                if kind < 0.5:
                    body["tools"] = tools
                elif kind < 0.65:
                    body["response_format"] = {"type": "json_schema", "json_schema": {"name": "r", "schema": SCHEMA}}
                if wrng.random() < 0.3:
                    body["stream"] = True
                try:
                    out = call(body)
                    count(True)
                except Exception as e:  # noqa: BLE001 - every failure is logged and counted
                    count(False)
                    log(f"{name} err {type(e).__name__}: {str(e)[:120]}")
                    time.sleep(5)
                    break
                msgs.append({"role": "assistant", "content": out or "ok"})
            try:
                call({"model": args.model, "max_tokens": 8,
                      "messages": [{"role": "user", "content": "ping: reply with the word pong"}]})
                count(True)
            except Exception as e:  # noqa: BLE001
                count(False)
                log(f"{name} ping err {type(e).__name__}")
                time.sleep(5)

    def reporter():
        while time.time() < stop_at:
            time.sleep(60)
            with lock:
                s = dict(stats)
            log(f"stats ok={s['ok']} err={s['err']}")

    log(f"start base={args.base} long={args.long} short={args.short} minutes={args.minutes}")
    threads = [threading.Thread(target=worker, args=(f"long{i}", systems["long"], i), daemon=True)
               for i in range(args.long)]
    threads += [threading.Thread(target=worker, args=(f"short{i}", systems["short"], 100 + i), daemon=True)
                for i in range(args.short)]
    for t in threads:
        t.start()
    threading.Thread(target=reporter, daemon=True).start()
    for t in threads:
        t.join()
    log(f"done ok={stats['ok']} err={stats['err']}")


if __name__ == "__main__":
    main()
