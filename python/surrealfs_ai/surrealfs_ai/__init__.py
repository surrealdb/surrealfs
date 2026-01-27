import uvicorn
from pydantic_ai import Agent, ModelSettings, WebFetchTool, WebSearchTool

from .tools import build_toolset, instructions


def build_chat_agent(
    model: str = "claude-haiku-4-5-20251001", enable_web_tools: bool = True
) -> Agent[None, str]:
    agent = Agent(
        model,
        toolsets=[build_toolset("surrealfs", "demo")],
        instructions=instructions,
        model_settings=ModelSettings(
            extra_headers={
                "anthropic-beta": ",".join(["context-management-2025-06-27"])
            }
        ),
        builtin_tools=[WebFetchTool(), WebSearchTool()] if enable_web_tools else [],
    )
    return agent


async def demo() -> None:
    agent = build_chat_agent()
    result = await agent.run(
        "Create /demo/hello.txt containing 'hello world', then show its content"
    )
    print(result.output)


if __name__ == "__main__":
    try:
        import logfire
    except ImportError:
        print("Failed to import logfire. Install with: uv sync --extra demo")
        raise

    _ = logfire.configure(send_to_logfire="if-token-present")
    logfire.instrument_pydantic_ai()
    logfire.instrument_anthropic()

    # Simple demo:
    # import asyncio
    # asyncio.run(demo())

    # Chat UI demo:
    agent = build_chat_agent()
    app = agent.to_web()
    uvicorn.run(app, host="127.0.0.1", port=7932)
