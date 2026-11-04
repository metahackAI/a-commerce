# Serving Muse Glimmer

[README](../README.md) › [docs](README.md)

The worker model, Muse Glimmer 30B, runs on one GPU that we host ourselves; every agent call on the live demo goes to it. This page covers what runs and why each setting is there, how fast it is, and a crash on the RTX 5090 that we traced and fixed. It also gives the scripts to measure all of it again.

## 1. What runs

| Part | What |
|---|---|
| GPU | One NVIDIA RTX 5090 (32 GB, Blackwell SM120), used by this model alone |
| Weights | [`nvidia/Muse-Glimmer-30B-NVFP4`](https://huggingface.co/nvidia/Muse-Glimmer-30B-NVFP4): NVIDIA's 4-bit (NVFP4) build of Muse Glimmer 30B, 24.7 GB, Apache-2.0 |
| Server | vLLM 0.28.0 with cuDNN 9.27: [`infra/models/Dockerfile`](../infra/models/Dockerfile). The stock image crashes on this GPU (§3) |
| Run it | `make up-models` (Compose profile `models`, [`infra/compose/compose.yaml`](../infra/compose/compose.yaml)) or the Kubernetes component [`models-vllm`](../infra/k8s/components/models-vllm/models.yaml). The demo server runs the same image and flags in its own container |

The flags, and why each is there:

| Flag | Why |
|---|---|
| `--max-model-len=65536` | A back-office turn with its tool results reaches 10–15k tokens; a long conversation keeps growing. |
| `--gpu-memory-utilization=0.85`, `--max-num-seqs=16` | Leaves room for about 80,000 tokens of KV cache next to the weights: one 64k conversation, or many short ones at once. |
| `--enable-prefix-caching`, `--mamba-cache-mode=align` | Every model call in a turn sends the same instructions and tool list again. Reusing that start cuts the first token on an 11k-token prompt from 2.4 s to 0.1 s (§2). Glimmer mixes Mamba layers with attention; `align` lets vLLM keep the Mamba state at cache-block edges so the reuse works for this model. |
| `--enable-chunked-prefill` | A long new prompt is read in pieces, so it does not stop other conversations' replies. |
| `--enable-auto-tool-choice`, `--tool-call-parser=muse_glimmer` | Tool calls come back as OpenAI `tool_calls`. |
| `--reasoning-parser=muse_glimmer` | The model's thinking goes to `reasoning_content`, not into the reply. |
| `--structured-outputs-config={"enable_in_reasoning": true}` | The agent asks for JSON with a schema for routing, plans and replies ([backend/agent.md](backend/agent.md)). Without this flag vLLM applies the schema only after it sees the thinking end, and it did not detect the end for this model, so the schema never applied (Oct 30: every chat answered "plan is not readable"). |

vLLM logs at start that this GPU "does not have native support for FP4 computation" and runs the 4-bit weights through its Marlin kernel instead. The speed below is with that kernel.

## 2. Speed

[`infra/models/speed.py`](../infra/models/speed.py), one request at a time, Nov 3 2026:

| Request | First token | Output |
|---|---|---|
| Short prompt (69 tokens), 400 tokens out | 0.08 s | 72 tokens/s |
| 11,063-token prompt, nothing cached, 300 out | 2.41 s | 62 tokens/s |
| The same prompt again (its start cached) | 0.11 s | 62 tokens/s |

For comparison, the workaround that also stopped the crash, `--enforce-eager` (§3, row E), measured 0.24 s, 2.77 s and 0.27 s to the first token and 22 tokens/s out: a third of the speed.

Under the load of §3 (16 agent-shaped conversations at once), the server answered 10,654 requests in 40 minutes.

## 3. A crash on the RTX 5090, and the fix

### What we saw

From Oct 31 11:35 to Nov 3 00:54 KST the model server died 42 times in 61 hours of team use: a median of 78 minutes between crashes (shortest 22 minutes, longest 4.2 hours). Each time:

- vLLM's engine stopped with `CUDA error: misaligned address` (in some crashes `an illegal memory access was encountered`). Requests in flight got HTTP 500, and Docker restarted the container, about a minute without a model.
- The kernel log showed `NVRM: Xid 13, Graphics SM Warp Exception ... Misaligned Address`, followed by `Xid 31 ... MMU Fault`. Xid 13 means a program running on the GPU read memory at a wrong address. Failing hardware shows up as other codes (Xid 48, 79, ECC errors), and there was no thermal or power slowdown.
- Other vLLM servers (0.27.1, other models) on the same machine and the same GPU model ran 5–9 days without a crash.

Separately, the server stopped answering four times without any error (Oct 31, Nov 1, Nov 2 twice). A watchdog or a person restarted it. We have not tied these stops to the crash.

### Finding the cause

Each row changes one thing from the demo's configuration and runs [`infra/models/stress.py`](../infra/models/stress.py) until the server dies or 40 minutes pass. The load is 8 conversations with a ~10k-token start and 8 with a short one, with tools, JSON schemas, streaming and tiny pings mixed in (row A ran only the 8 long ones). One run per row, Nov 3 01:00–08:28 KST.

| | Change from the demo configuration | Ran | Requests | Result |
|---|---|---|---|---|
| A | none (stock `vllm/vllm-openai:v0.28.0`, cuDNN 9.20) | 14 min | ~3,100 | crashed, misaligned address |
| B | prefix caching off | 22 min (3 min after the short conversations joined) | ~830 | crashed, illegal memory access |
| C | vLLM's older model runner (`VLLM_USE_V2_MODEL_RUNNER=0`) | — | 1 | failed on the first request: that runner cannot copy Mamba state for `align` |
| D | older model runner, prefix caching off | 21 min | ~900 | crashed, misaligned address |
| E | CUDA graphs off (`--enforce-eager`) | 40 min | 9,153 | no crash, no Xid; output 22 tokens/s |
| **F** | **cuDNN 9.27.0.42 and cudnn-frontend 1.32.1; flags as in A** | **40 min** | **10,654** | **no crash, no Xid; output 72 tokens/s** |

B and D answered fewer requests per minute because, without prefix caching, every turn reads its whole prompt again.

What the rows say:

- B and D: neither prefix caching nor vLLM's new model runner is the cause; the crash stays without them.
- E: the crash needs CUDA graphs (vLLM records the GPU work of a step once and replays it). Turning them off stops it, at a third of the speed.
- The cause, from the kernel log and an open vLLM issue:
  - Every `Graphics Exception: ESR` line in our kernel log (27 across the crashes) carries the data words `0x1c81fb60` / `0x1174`. These are the same words as in [vllm-project/vllm#52225](https://github.com/vllm-project/vllm/issues/52225): recurring Xid 13 misaligned-address crashes on SM120 GPUs with a 4-bit hybrid Mamba model, with the same Xid 31 fault.
  - In that thread, NVIDIA's cuDNN team traced it: a runtime-compiled cuDNN kernel can be freed while a CUDA graph that calls it is still replayed. The fix is in cuDNN 9.27 and cudnn-frontend 1.31.
  - Our engine process loads `libcudnn_engines_runtime_compiled.so.9`. Stock vLLM 0.28.0 ships cuDNN 9.20 and cudnn-frontend 1.27.
- F: upgrading those two packages and nothing else (CUDA graphs on, flags as in A) ran the full 40 minutes, more than three times the requests that killed A. Since Nov 3 07:45 KST the demo runs this image.

The fix is NVIDIA's. Our part was finding which fix applies and measuring that it holds.

### Limits

- One run per row. Before the fix, the same load killed the server within 3–21 minutes, so a clean 40 minutes is strong evidence, not proof. The demo's uptime since the switch is the running check: no restart and no Xid as of Nov 3 10:10 KST, a span that includes the final eval (462 task runs, 1,543 model calls).
- Only the RTX 5090 was tested. On other GPUs the stock image may be fine. The Kubernetes component keeps the stock image and says when to switch.
- A request with a JSON schema sometimes fails on its own with HTTP 500 (`grammar rejected tokens [200008]`, the end-of-turn token): 14 of 10,654 requests in F, about the same rate as before the fix (5 of ~3,100 in A). Only that request fails; the server keeps running. It can repeat on the same prompt: in the final eval it failed 8 of 1,543 calls, and 5 of those were one task's reply call, once in each of its 5 runs ([eval/results/2026-11-03-glimmer-judged.md](../eval/results/2026-11-03-glimmer-judged.md)).

### Effect on our eval numbers

Crashes during an eval run count as failed tasks. We checked the saved logs of the eval runs from Oct 31 to Nov 2 (`cargo xtask eval --logs`, not committed): 440 task runs and 1,721 model calls.

- 35 model calls failed (HTTP 500, 502, timeouts, or the agent waiting out a model that had just failed).
- Those failures fell in 17 runs (3.9%).
- 7 of those runs ended with "The assistant's model is not answering".

So crashes explain only a small part of the run-to-run swing noted in [design.md](design.md) §7 (one or two runs out of three); most of it is the model itself.

## 4. Measure it again

```bash
make up-models                  # builds infra/models/Dockerfile and serves Glimmer on :8000 inside Compose
# with the port reachable from where the scripts run:
python3 infra/models/speed.py  --base http://127.0.0.1:8000 --model nvidia/Muse-Glimmer-30B-NVFP4
python3 infra/models/stress.py --base http://127.0.0.1:8000 --model nvidia/Muse-Glimmer-30B-NVFP4 \
    --minutes 40 --long 8 --short 8 --log stress.log
```

While the load runs, watch the server, not the script:

- `docker inspect -f '{{.RestartCount}} {{.State.StartedAt}}' <container>`: a new start time means the engine died.
- `journalctl -k | grep Xid`: the GPU's own report.

Run `speed.py` while nothing else uses the GPU.
