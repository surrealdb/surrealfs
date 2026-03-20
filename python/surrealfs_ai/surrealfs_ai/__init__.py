import uvicorn
from pydantic_ai import Agent, ModelSettings, WebFetchTool, WebSearchTool
from surrealfs_py import PySurrealFs  # type: ignore

from surrealfs_ai.definitions import AgentDeps
from surrealfs_ai.tools import build_toolset, instructions


def build_chat_agent(
    model: str = "claude-haiku-4-5-20251001", enable_web_tools: bool = True
) -> Agent[AgentDeps, str]:
    agent = Agent(
        model,
        toolsets=[build_toolset()],
        instructions=instructions,
        deps_type=AgentDeps,
        model_settings=ModelSettings(
            extra_headers={
                "anthropic-beta": ",".join(["context-management-2025-06-27"])
            }
        ),
        builtin_tools=[WebFetchTool(), WebSearchTool()] if enable_web_tools else [],
    )
    return agent


__all__ = ["build_chat_agent", "PySurrealFs"]

if __name__ == "__main__":
    import os

    try:
        import logfire
    except ImportError:
        print("Failed to import logfire. Install with: uv sync --extra demo")
        raise

    _ = logfire.configure(send_to_logfire="if-token-present")
    logfire.instrument_pydantic_ai()
    logfire.instrument_anthropic()

    db_url = os.environ.get("SURREALDB_URL", "ws://localhost:8000")
    db_ns = os.environ.get("SURREALDB_NAMESPACE", "test")
    db_name = os.environ.get("SURREALDB_DATABASE", "test")

    fs = PySurrealFs.connect_ws(db_url, db_ns, db_name)

    # Chat UI demo:
    agent = build_chat_agent()
    app = agent.to_web(deps=AgentDeps(fs=fs))
    uvicorn.run(app, host="127.0.0.1", port=7932)
