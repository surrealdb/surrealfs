import uvicorn
from pydantic_ai import Agent, ModelSettings, WebFetchTool, WebSearchTool

from tools import TOOLSET


def build_chat_agent(
    model: str = "claude-haiku-4-5-20251001", enable_web_tools: bool = True
) -> Agent:
    return Agent(
        model=model,
        toolsets=[TOOLSET],
        instructions=(
            "You are a helpful assistant that organizes my thoughts, conversations, notes, into a well-structured text file system."
            "Every time you learn something about my preferences, store it in a file in the /preferences folder. For example, create files like /preferences/food.md, /preferences/music.md, /preferences/books.md, etc."
            "When I talk about a project or task, organize the notes and current to-do list in a /project/<project_name> folder. For example, /project/social_media/2026/post_calendar_january.md or /project/support/solutions/vector_index.md"
            "Write your main notes in /notes.md, and read them every time we interact."
            "Before you answer, consider updating the /notes.md file with your latest thoughts and insights."
        ),
        model_settings=ModelSettings(
            extra_headers={
                "anthropic-beta": ",".join(["context-management-2025-06-27"])
            }
        ),
        builtin_tools=[WebFetchTool(), WebSearchTool()] if enable_web_tools else [],
    )


async def demo() -> None:
    agent = build_chat_agent()
    result = await agent.run(
        "Create /demo/hello.txt containing 'hello world', then show its content",
    )
    print(result.output)


if __name__ == "__main__":
    # import asyncio
    # asyncio.run(demo())

    agent = build_chat_agent()
    app = agent.to_web()
    uvicorn.run(app, host="127.0.0.1", port=7932)
