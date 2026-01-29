# generate_image

Generate one image and store the resulting bytes in the SurrealFs virtual filesystem.

## Usage
- `prompt` (required): natural language description of the image.
- `path` (required): path destination for the image. Ex: `images/my_image.png`

Requires `OPENAI_API_KEY` in the environment. Images are written via `write_bytes` so binary data is preserved.
