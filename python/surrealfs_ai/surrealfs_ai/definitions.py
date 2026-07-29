from pydantic import BaseModel, ConfigDict
from surrealfs_py.surrealfs_py import PySurrealFs


class AgentDeps(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    fs: PySurrealFs
