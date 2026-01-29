import base64
import os
from dataclasses import dataclass
from typing import List, Literal

from openai import AsyncOpenAI
from pydantic import BaseModel, Field, field_validator
from pydantic_ai import FunctionToolset

# TODO: generate types
from surrealfs_py import PySurrealFs  # type: ignore


@dataclass
class GenerateImageGlobalArgs:
    model: str = "dall-e-2"
    size: Literal["512x512", "1024x1024"] = "512x512"
    n: int = 1


class GenerateImageArgs(BaseModel):
    prompt: str = Field(..., description="Natural language description of the image")
    path: str = Field(..., description="Destination path in SurrealFs")
    # model: str = Field("gpt-image-1", description="OpenAI image model")
    # size: str = Field("1024x1024", description="Image size like 1024x1024")
    # n: int = Field(1, ge=1, le=10, description="Number of images to generate")

    @field_validator("path")
    @classmethod
    def ensure_leading_slash(cls, v: str) -> str:
        return v if v.startswith("/") else f"/{v}"


def add_image_tools(
    toolset: FunctionToolset,
    fs: PySurrealFs,
    global_args: GenerateImageGlobalArgs = GenerateImageGlobalArgs(),
) -> None:
    async def generate_image(args: GenerateImageArgs) -> str:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            return "error: OPENAI_API_KEY is not set"

        client = AsyncOpenAI(api_key=api_key)

        try:
            resp = await client.images.generate(
                model=global_args.model,
                prompt=args.prompt,
                n=global_args.n,
                size=global_args.size,
                response_format="b64_json",
            )
        except Exception as e:  # noqa: E722
            return f"error: failed to generate image: {e}"

        paths: List[str] = []
        base_path = args.path
        if not resp.data:
            return "error: no images generated"
        for idx, item in enumerate(resp.data, start=1):
            b64 = item.b64_json
            if not b64:
                return "error: missing image data in response"
            data = base64.b64decode(b64)
            dest = base_path if global_args.n == 1 else f"{base_path}.{idx}"
            try:
                await fs.write_bytes(dest, data)
            except Exception as e:  # noqa: E722
                return f"error: failed to write image to {dest}: {e}"
            paths.append(dest)

        if len(paths) == 1:
            return f"image saved to {paths[0]}"
        return "images saved to: " + ", ".join(paths)

    toolset.add_function(
        generate_image,
        description=(
            "Generate image(s) with OpenAI and store them in SurrealFs. "
            "Requires OPENAI_API_KEY. Args: prompt, path, optional model/size/n. "
            "Returns the virtual file path(s) where images were saved."
        ),
        takes_ctx=False,
    )
