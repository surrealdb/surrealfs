import uvicorn
from pydantic_ai import Agent, ModelSettings, WebFetchTool, WebSearchTool

from tools import TOOLSET


def build_chat_agent() -> Agent:
    return Agent(
        model="claude-haiku-4-5-20251001",
        toolsets=[TOOLSET],
        instructions=(
            "You are a helpful assistant that organizes my thoughts, conversations, notes, into a well-structured text file system."
            # "You are a helpful assistant that takes notes of our to build a growing knowledge base from our interactions."
            # "Use the SurrealFs tools to organize your notes in the filesystem."
            # "Keep episodal memories of our interactions, as well as long-term memories."
            # "Keep notes about your to-do list, and your current task, to never lose track of what you're doing."
            # "For example, you can create a file /notes/todo.txt to keep track of your tasks."
            # "You can also create a file /notes/current_task.txt to keep track of your current task."
            "Every time you learn something about my preferences, store it in a file in the /preferences folder. For example, create files like /preferences/food.txt, /preferences/music.txt, /preferences/books.txt, etc."
            "When I talk about a project or task, organize the notes and current to-do list in a /project/<project_name> folder. For example, /project/social_media/2026/post_calendar_january.txt or /project/support/solutions/vector_index.txt"
            "Write your main notes in /notes.md, and read them every time we interact."
            "Before you answer, consider updating the /notes.md file with your latest thoughts and insights."
        ),
        model_settings=ModelSettings(
            extra_headers={
                "anthropic-beta": ",".join(["context-management-2025-06-27"])
            }
        ),
        builtin_tools=[WebFetchTool(), WebSearchTool()],
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
