# The worker model server: vLLM 0.28.0 with cuDNN 9.27.
#
# Stock vllm/vllm-openai:v0.28.0 ships cuDNN 9.20. Serving Muse Glimmer (NVFP4) on an RTX 5090 (SM120)
# it crashed every 3 to 21 minutes under load with Xid 13 "misaligned address"
# (vllm-project/vllm#52225). cuDNN 9.27 and cudnn-frontend 1.31+ carry NVIDIA's fix; with them the
# same flags ran 40 minutes under the same load without a crash. Measurements: docs/serving.md.
FROM vllm/vllm-openai:v0.28.0

RUN python3 -m pip install --no-cache-dir --break-system-packages \
        nvidia-cudnn-cu13==9.27.0.42 nvidia-cudnn-frontend==1.32.1 \
    && python3 -c "import torch; v = torch.backends.cudnn.version(); assert v >= 92700, v"
