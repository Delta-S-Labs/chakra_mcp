"""
Example ChakraMCP agent — Python + LangChain.

Loads NVIDIA_API_KEY (or AWS Bedrock credentials) from .env.local at the
repo root, sends a single prompt to the LLM, prints the response.

The relay registration / discovery / message-send calls are stubbed out
with TODOs. Once the Rust relay's Phase 1 lands (see
`docs/chakramcp-build-spec.md`), uncomment the relay client calls.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

from dotenv import load_dotenv
from langchain_core.messages import HumanMessage, SystemMessage

# Load .env.local from the repo root (two levels up from this file).
ROOT_ENV = Path(__file__).resolve().parents[3] / ".env.local"
if ROOT_ENV.exists():
    load_dotenv(ROOT_ENV)
load_dotenv()  # also load .env in the example folder if present


SYSTEM_PROMPT = (
    "You are a small, well-mannered example agent on the ChakraMCP relay "
    "network. You answer questions in two short sentences."
)


def build_llm():
    """Return a LangChain chat model. Defaults to NVIDIA NIM, falls back to Bedrock."""
    nvidia_key = os.getenv("NVIDIA_API_KEY")
    if nvidia_key:
        from langchain_openai import ChatOpenAI

        return ChatOpenAI(
            api_key=nvidia_key,
            base_url=os.getenv("NVIDIA_BASE_URL", "https://integrate.api.nvidia.com/v1"),
            model=os.getenv("NVIDIA_MODEL", "meta/llama-3.1-70b-instruct"),
            temperature=0.5,
        )

    if os.getenv("AWS_REGION") and (os.getenv("AWS_ACCESS_KEY_ID") or os.getenv("AWS_PROFILE")):
        from langchain_aws import ChatBedrock

        return ChatBedrock(
            model_id=os.getenv("BEDROCK_MODEL_ID", "anthropic.claude-3-5-sonnet-20241022-v2:0"),
            region_name=os.getenv("AWS_REGION", "us-east-1"),
        )

    raise SystemExit(
        "No LLM configured. Set NVIDIA_API_KEY in .env.local "
        "(get a free key at https://build.nvidia.com/) or set AWS Bedrock creds."
    )


def main() -> None:
    prompt = " ".join(sys.argv[1:]) or "What is the relay network for AI agents in one line?"
    llm = build_llm()
    messages = [SystemMessage(SYSTEM_PROMPT), HumanMessage(prompt)]
    response = llm.invoke(messages)
    print(response.content)

    # TODO: relay integration — pending Rust backend Phase 1.
    # from .relay_client import RelayClient
    # client = RelayClient(base_url=os.getenv("RELAY_URL", "http://localhost:8080"))
    # client.register_agent(name="example-py", capabilities=["echo"])
    # for event in client.poll_events():
    #     ...


if __name__ == "__main__":
    main()
