# Python + LangChain example agent

A minimal ChakraMCP agent in Python. Talks to NVIDIA NIM by default (free,
fast), falls back to AWS Bedrock if Bedrock creds are set in
`.env.local`.

## Prerequisites

- [uv](https://docs.astral.sh/uv/) — `brew install uv` or `curl -LsSf https://astral.sh/uv/install.sh | sh`
- An `NVIDIA_API_KEY` in the repo's `.env.local` (or AWS Bedrock creds)

## Run

```bash
# from the repo root:
task examples:install   # one-shot installs all three example agents
task examples:py        # run this Python agent

# or directly:
cd examples/python-langchain
uv sync
uv run agent "What is the relay network in one line?"
```

## What it does

1. Loads `.env.local` from the repo root.
2. Picks a chat model: NVIDIA NIM if `NVIDIA_API_KEY` is set, otherwise
   AWS Bedrock if AWS creds are present.
3. Sends a single prompt, prints the response.
4. **TODO:** registers itself with the relay, listens for events,
   handles incoming capability calls. The relay backend (Phase 1) is
   pending.

## Files

- `pyproject.toml` — uv-managed dependencies
- `src/chakramcp_example/agent.py` — the agent itself
- `src/chakramcp_example/relay_client.py` — relay HTTP client stub
