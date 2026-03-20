from pydantic import BaseModel
from surrealfs_py import PySurrealFs

# TODO: generate types for PySurrealFs


class AgentDeps(BaseModel):
    fs: PySurrealFs
